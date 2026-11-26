#!/usr/bin/env python3
"""Public GEO (Generative Engine Optimization) & Agent Readiness engine (DP6).

Generates machine-readable assets, HTML head SEO/GEO metadata, Markdown mirrors,
llms.txt / llms-full.txt, XML sitemaps, Atom feeds, Agent Skills discovery,
bilingual search index, and a zero-dependency Cloudflare Pages _worker.js for
Accept: text/markdown content negotiation and /.well-known/search (GET + QUERY).

Only invoked for public sites (ENCRYPT=0). Private encrypted sites (ENCRYPT=1)
call clean_stale_geo_files() so zero plaintext GEO artifacts ever leak.

Strict lower-triangular dependency boundary: imports only stdlib + i18n.
"""
from __future__ import annotations

import hashlib
import html as htmlmod
import json
import re
from dataclasses import dataclass, field
from datetime import datetime, timezone
from html.parser import HTMLParser
from pathlib import Path
from typing import Iterable

import i18n

CONTENT_SIGNAL_DEFAULT = "ai-train=yes, search=yes, ai-input=yes"
SEARCH_DOCS_URL = "https://developers.cloudflare.com/ai-search/how-to/search-for-agents/"
SEARCH_INSTRUCTIONS = (
    "Use this search API to find relevant content across this site with keywords "
    "or natural-language queries, instead of crawling individual pages. "
    "Results are returned in Markdown."
)
AGENT_SKILLS_SCHEMA = "https://schemas.agentskills.io/discovery/0.2.0/schema.json"

AI_SEARCH_BOTS: tuple[str, ...] = (
    "OAI-SearchBot",
    "ChatGPT-User",
    "Claude-SearchBot",
    "Claude-User",
    "PerplexityBot",
    "Perplexity-User",
)
AI_TRAIN_BOTS: tuple[str, ...] = (
    "GPTBot",
    "ClaudeBot",
    "Google-Extended",
    "Applebot-Extended",
    "CCBot",
    "Bytespider",
    "Amazonbot",
)

GEO_ROOT_FILES: tuple[str, ...] = (
    "sitemap.xml",
    "llms.txt",
    "llms-full.txt",
    "feed.xml",
    "search-index.json",
    "_worker.js",
    "_routes.json",
)
GEO_EXTRA_FILES: tuple[str, ...] = (
    "zh/feed.xml",
    "en/feed.xml",
    ".well-known/agent-skills/index.json",
    ".well-known/agent-skills/ai-digest-reader/SKILL.md",
)

_META_MARK_START = "<!-- digest-meta:start -->"
_META_MARK_END = "<!-- digest-meta:end -->"
_SIGNAL_VALS = frozenset({"yes", "no"})
_WS_RE = re.compile(r"\s+")
_TITLE_NUM_RE = re.compile(r"^\s*\d+\.\s*")
_DOMAIN_SAFE_RE = re.compile(r"^[A-Za-z0-9.-]+(?::\d{1,5})?$")


@dataclass
class GeoPage:
    """Descriptor for one public HTML page and its Markdown/GEO metadata."""

    rel_html: str
    url_path: str
    canonical_path: str
    rel_md: str
    lang: str
    page_type: str  # "issue" | "index"
    kind: str = ""  # "daily" | "weekly" | ""
    label: str = ""
    date_iso: str = ""
    is_canonical: bool = True
    alternates: dict[str, str] = field(default_factory=dict)
    feed_path: str = "/feed.xml"
    parsed: dict | None = None


def site_origin(domain: str) -> str:
    """Normalize SITE_DOMAIN into a safe https://<host> origin."""
    raw = (domain or "").strip()
    if raw.startswith("https://"):
        raw = raw[8:]
    elif raw.startswith("http://"):
        raw = raw[7:]
    raw = raw.split("/", 1)[0].strip().rstrip(".")
    if not raw or not _DOMAIN_SAFE_RE.fullmatch(raw):
        raw = "digest.example.com"
    return f"https://{raw}"


def parse_content_signal(
    raw: str | None = None, *, ai_train: str | None = None
) -> tuple[str, bool]:
    """Return canonical (Content-Signal header value, allow_ai_train)."""
    vals = {"ai-train": "yes", "search": "yes", "ai-input": "yes"}
    if raw:
        for part in raw.split(","):
            if "=" not in part:
                continue
            k, v = part.split("=", 1)
            k = k.strip().lower()
            v = v.strip().lower()
            if k in vals and v in _SIGNAL_VALS:
                vals[k] = v
    if ai_train is not None and ai_train.strip():
        low = ai_train.strip().lower()
        if low in ("0", "false", "no", "off"):
            vals["ai-train"] = "no"
        elif low in ("1", "true", "yes", "on"):
            vals["ai-train"] = "yes"
    sig = f"ai-train={vals['ai-train']}, search={vals['search']}, ai-input={vals['ai-input']}"
    return sig, (vals["ai-train"] == "yes")


def _clean_text(s: str) -> str:
    return _WS_RE.sub(" ", htmlmod.unescape(s or "")).strip()


def _esc_attr(s: str) -> str:
    return htmlmod.escape(s or "", quote=True)


def _esc_xml(s: str) -> str:
    return (
        (s or "")
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
        .replace("'", "&apos;")
    )


def _yaml_quote(s: str) -> str:
    cleaned = _clean_text(s)
    return json.dumps(cleaned, ensure_ascii=False)


def _json_ld_script(obj: dict) -> str:
    raw = json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
    safe = raw.replace("</", "<\\/")
    return f'<script type="application/ld+json">{safe}</script>'


def _truncate(text: str, limit: int = 160) -> str:
    t = _clean_text(text)
    if len(t) <= limit:
        return t
    cut = t[: limit - 1].rstrip(" ,;·|—-")
    return cut + "…"


class _DigestHTMLParser(HTMLParser):
    """Deterministic parser for render_html.py issue HTML DOM."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.title = ""
        self.html_title = ""
        self.period_line = ""
        self.stats_line = ""
        self.motto = ""
        self.edition = ""
        self.must: list[dict] = []
        self.themes: list[dict] = []
        self.sections: list[dict] = []
        self.items: list[dict] = []

        self._stack: list[tuple[str, set[str]]] = []
        self._skip_depth = 0
        self._in_title = False
        self._in_mast_meta = False
        self._mast_meta_spans: list[str] = []
        self._cur_span_buf: list[str] | None = None
        self._in_edition = False
        self._in_motto = False

        self._cur_section: dict | None = None
        self._in_sec_h2 = False
        self._sec_h2_buf: list[str] = []

        self._in_must_list = False
        self._cur_must: dict | None = None
        self._must_field: str | None = None
        self._must_buf: list[str] = []

        self._cur_story: dict | None = None
        self._story_field: str | None = None
        self._story_buf: list[str] = []
        self._in_story_links = False
        self._cur_link_href = ""
        self._cur_link_buf: list[str] = []
        self._skip_label_span = 0

    def _classes(self, attrs: list[tuple[str, str | None]]) -> set[str]:
        for k, v in attrs:
            if k == "class" and v:
                return set(v.split())
        return set()

    def _attr(self, attrs: list[tuple[str, str | None]], name: str) -> str:
        for k, v in attrs:
            if k == name and v is not None:
                return v
        return ""

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        cls = self._classes(attrs)
        self._stack.append((tag, cls))

        # Ignore secondary bilingual pane (lang-en) if parsing a legacy bilingual file
        if self._skip_depth > 0:
            self._skip_depth += 1
            return
        if tag == "div" and "lang-en" in cls and "lang-pane" in cls:
            self._skip_depth = 1
            return
        if tag in ("script", "style", "svg", "button"):
            self._skip_depth = 1
            return

        if tag == "meta":
            if self._attr(attrs, "name").lower() == "digest-title":
                content = self._attr(attrs, "content").strip()
                if content:
                    self.title = content
            return
        if tag == "title":
            self._in_title = True
            return

        if tag == "span" and "mast-meta" in cls:
            self._in_mast_meta = True
            return
        if self._in_mast_meta and tag == "span":
            if "motto" in cls:
                self._in_motto = True
            self._cur_span_buf = []
            return
        if tag == "span" and "edition" in cls:
            self._in_edition = True
            return

        if tag == "ol" and "must-list" in cls:
            self._in_must_list = True
            return
        if self._in_must_list and tag == "li":
            self._cur_must = {"title": "", "dek": "", "anchor": ""}
            return
        if self._cur_must is not None:
            if tag == "a" and "must-title" in cls:
                self._cur_must["anchor"] = self._attr(attrs, "href")
                self._must_field = "title"
                self._must_buf = []
                return
            if tag == "span" and "must-title" in cls:
                self._must_field = "title"
                self._must_buf = []
                return
            if tag == "span" and "must-dek" in cls:
                self._must_field = "dek"
                self._must_buf = []
                return

        if tag == "section" and "sec" in cls:
            self._cur_section = {
                "id": self._attr(attrs, "id"),
                "title": "",
                "items": [],
            }
            self.sections.append(self._cur_section)
            return
        if self._cur_section is not None and tag == "h2":
            self._in_sec_h2 = True
            self._sec_h2_buf = []
            return

        if tag == "article" and "story" in cls:
            tags_raw = self._attr(attrs, "data-tags")
            tags = [t.strip() for t in tags_raw.split("|") if t.strip()] if tags_raw else []
            self._cur_story = {
                "id": self._attr(attrs, "id"),
                "number": "",
                "title": "",
                "source": "",
                "when": "",
                "tags": tags,
                "dek": "",
                "look": "",
                "detail": "",
                "extra": "",
                "links": [],
                "section": self._cur_section["title"] if self._cur_section else "",
            }
            return

        if self._cur_story is not None:
            if tag == "span" and "source" in cls:
                self._story_field = "source"
                self._story_buf = []
            elif tag == "span" and "when" in cls:
                self._story_field = "when"
                self._story_buf = []
            elif tag == "span" and "story-no" in cls:
                self._story_field = "number"
                self._story_buf = []
            elif tag == "h3":
                self._story_field = "title"
                self._story_buf = []
            elif tag == "div" and "dek" in cls:
                self._story_field = "dek"
                self._story_buf = []
            elif tag == "div" and "look" in cls:
                self._story_field = "look"
                self._story_buf = []
            elif tag == "div" and "detail" in cls:
                self._story_field = "detail"
                self._story_buf = []
            elif tag == "div" and "extra" in cls:
                self._story_field = "extra"
                self._story_buf = []
            elif tag == "span" and ({"look-k", "detail-k", "extra-k"} & cls):
                self._skip_label_span += 1
            elif tag == "div" and "story-links" in cls:
                self._in_story_links = True
            elif self._in_story_links and tag == "a":
                self._cur_link_href = self._attr(attrs, "href")
                self._cur_link_buf = []

    def handle_endtag(self, tag: str) -> None:
        cls = self._stack.pop()[1] if self._stack else set()
        if self._skip_depth > 0:
            self._skip_depth -= 1
            return

        if tag == "title" and self._in_title:
            self._in_title = False
            return

        if self._in_mast_meta and tag == "span" and "mast-meta" not in cls:
            if self._cur_span_buf is not None:
                val = _clean_text("".join(self._cur_span_buf))
                if self._in_motto:
                    self.motto = val
                    self._in_motto = False
                elif val:
                    self._mast_meta_spans.append(val)
                self._cur_span_buf = None
            return
        if tag == "span" and "mast-meta" in cls:
            self._in_mast_meta = False
            if self._mast_meta_spans:
                self.period_line = self._mast_meta_spans[0]
            if len(self._mast_meta_spans) > 1:
                self.stats_line = self._mast_meta_spans[1]
            return
        if tag == "span" and "edition" in cls and self._in_edition:
            self._in_edition = False
            return

        if self._cur_must is not None:
            if self._must_field and tag in ("a", "span"):
                self._cur_must[self._must_field] = _clean_text("".join(self._must_buf))
                self._must_field = None
                self._must_buf = []
                return
            if tag == "li":
                if self._cur_must["title"]:
                    self.must.append(self._cur_must)
                self._cur_must = None
                return
        if tag == "ol" and "must-list" in cls:
            self._in_must_list = False
            return

        if self._in_sec_h2 and tag == "h2":
            if self._cur_section is not None:
                self._cur_section["title"] = _clean_text("".join(self._sec_h2_buf))
            self._in_sec_h2 = False
            self._sec_h2_buf = []
            return
        if tag == "section" and "sec" in cls:
            self._cur_section = None
            return

        if self._cur_story is not None:
            if self._skip_label_span > 0 and tag == "span":
                self._skip_label_span -= 1
                return
            if self._in_story_links and tag == "a":
                lbl = _clean_text("".join(self._cur_link_buf))
                if self._cur_link_href and lbl:
                    self._cur_story["links"].append((lbl, self._cur_link_href))
                self._cur_link_href = ""
                self._cur_link_buf = []
                return
            if self._in_story_links and tag == "div":
                self._in_story_links = False
                return
            if tag == "span" and self._story_field == "number":
                self._cur_story["number"] = _clean_text("".join(self._story_buf)).rstrip(".")
                self._story_field = "title"
                self._story_buf = []
                return
            if tag == "span" and self._story_field in ("source", "when"):
                self._cur_story[self._story_field] = _clean_text("".join(self._story_buf))
                self._story_field = None
                self._story_buf = []
                return
            if tag == "h3" and self._story_field == "title":
                raw_t = _clean_text("".join(self._story_buf))
                self._cur_story["title"] = _TITLE_NUM_RE.sub("", raw_t).strip()
                self._story_field = None
                self._story_buf = []
                return
            if tag == "div" and self._story_field in ("dek", "look", "detail", "extra"):
                val = _clean_text("".join(self._story_buf))
                if self._story_field == "extra" and self._cur_story["extra"]:
                    self._cur_story["extra"] += " " + val
                else:
                    self._cur_story[self._story_field] = val
                self._story_field = None
                self._story_buf = []
                return
            if tag == "article":
                if self._cur_story["title"]:
                    self.items.append(self._cur_story)
                    if self._cur_section is not None:
                        self._cur_section["items"].append(self._cur_story)
                self._cur_story = None
                self._story_field = None
                self._story_buf = []
                return

    def handle_data(self, data: str) -> None:
        if self._skip_depth > 0 or self._skip_label_span > 0:
            return
        if self._in_title:
            self.html_title += data
            return
        if self._in_edition:
            self.edition += data
            return
        if self._cur_span_buf is not None:
            self._cur_span_buf.append(data)
            return
        if self._must_field is not None:
            self._must_buf.append(data)
            return
        if self._in_sec_h2:
            self._sec_h2_buf.append(data)
            return
        if self._in_story_links and self._cur_link_href:
            self._cur_link_buf.append(data)
            return
        if self._story_field is not None:
            self._story_buf.append(data)


def parse_issue_html(html_text: str, *, lang: str = "zh-CN") -> dict:
    """Parse render_html.py issue HTML into structured metadata and items."""
    lg = i18n.normalize_lang(lang)
    parser = _DigestHTMLParser()
    parser.feed(html_text)
    parser.close()

    title = _clean_text(parser.title or parser.html_title or "AI Digest")
    edition = _clean_text(parser.edition)
    period_line = _clean_text(parser.period_line)
    stats_line = _clean_text(parser.stats_line)
    motto = _clean_text(parser.motto)

    tag_counts: dict[str, int] = {}
    for it in parser.items:
        for tg in it.get("tags", ()):
            tag_counts[tg] = tag_counts.get(tg, 0) + 1
    top_tags = [k for k, _ in sorted(tag_counts.items(), key=lambda kv: (-kv[1], kv[0]))[:10]]

    lead_titles = [it["title"] for it in parser.items[:3] if it.get("title")]
    if lg == "zh-CN":
        sep = "；"
        prefix = f"{title}（{period_line}，共 {len(parser.items)} 条）：" if period_line else f"{title}："
    else:
        sep = "; "
        prefix = f"{title} ({period_line}, {len(parser.items)} items): " if period_line else f"{title}: "
    desc = _truncate(prefix + sep.join(lead_titles), 160) if lead_titles else _truncate(title, 160)

    return {
        "title": title,
        "edition": edition,
        "period_line": period_line,
        "stats_line": stats_line,
        "motto": motto,
        "description": desc,
        "top_tags": top_tags,
        "must": parser.must,
        "sections": parser.sections,
        "items": parser.items,
        "items_count": len(parser.items),
    }


def _abs_url(origin: str, path: str) -> str:
    if path.startswith("https://") or path.startswith("http://"):
        return path
    p = path if path.startswith("/") else "/" + path
    return origin.rstrip("/") + p


def build_issue_json_ld(parsed: dict, *, page: GeoPage, origin: str) -> dict:
    """Build Schema.org NewsArticle + ItemList JSON-LD for a public digest issue."""
    canonical = _abs_url(origin, page.canonical_path)
    md_url = _abs_url(origin, "/" + page.rel_md.lstrip("/"))
    items_list = []
    for idx, it in enumerate(parsed.get("items", [])[:15], 1):
        entry: dict = {
            "@type": "ListItem",
            "position": idx,
            "name": it["title"],
            "url": f"{canonical}#{it['id']}" if it.get("id") else canonical,
        }
        if it.get("dek"):
            entry["description"] = _truncate(it["dek"], 180)
        items_list.append(entry)

    obj: dict = {
        "@context": "https://schema.org",
        "@type": "NewsArticle",
        "headline": parsed["title"],
        "description": parsed["description"],
        "inLanguage": page.lang,
        "url": canonical,
        "mainEntityOfPage": {"@type": "WebPage", "@id": canonical},
        "isPartOf": {"@type": "WebSite", "name": "AI Digest", "url": origin + "/"},
        "publisher": {"@type": "Organization", "name": "AI Digest", "url": origin + "/"},
        "encoding": {
            "@type": "MediaObject",
            "encodingFormat": "text/markdown",
            "contentUrl": md_url,
        },
    }
    if page.date_iso:
        obj["datePublished"] = page.date_iso
        obj["dateModified"] = page.date_iso
    if parsed.get("top_tags"):
        obj["keywords"] = ", ".join(parsed["top_tags"])
    if items_list:
        obj["mainEntity"] = {
            "@type": "ItemList",
            "numberOfItems": parsed.get("items_count", len(items_list)),
            "itemListElement": items_list,
        }
    return obj


def build_index_json_ld(
    *,
    page: GeoPage,
    origin: str,
    issues_meta: list[dict],
    search_enabled: bool = True,
) -> dict:
    """Build Schema.org WebSite + CollectionPage JSON-LD for a public archive index."""
    canonical = _abs_url(origin, page.canonical_path)
    if page.lang == "zh-CN":
        site_desc = "每日与每周精选前沿 AI 动态，覆盖模型发布、研究论文、工程实践与产业趋势。"
    else:
        site_desc = (
            "Curated daily and weekly briefings on frontier AI models, research papers, "
            "systems engineering, and industry developments."
        )
    website: dict = {
        "@type": "WebSite",
        "@id": origin + "/#website",
        "name": "AI Digest",
        "url": origin + "/",
        "description": site_desc,
        "inLanguage": ["zh-CN", "en"],
    }
    if search_enabled:
        website["potentialAction"] = {
            "@type": "SearchAction",
            "target": {
                "@type": "EntryPoint",
                "urlTemplate": f"{origin}/.well-known/search?q={{search_term_string}}",
            },
            "query-input": "required name=search_term_string",
        }
    collection: dict = {
        "@type": "CollectionPage",
        "@id": canonical,
        "name": f"AI Digest — {'归档目录' if page.lang == 'zh-CN' else 'Archive'}",
        "description": site_desc,
        "url": canonical,
        "inLanguage": page.lang,
        "isPartOf": {"@id": origin + "/#website"},
    }
    if issues_meta:
        collection["mainEntity"] = {
            "@type": "ItemList",
            "numberOfItems": len(issues_meta),
            "itemListElement": [
                {
                    "@type": "ListItem",
                    "position": idx,
                    "name": m["title"],
                    "url": _abs_url(origin, m["url_path"]),
                }
                for idx, m in enumerate(issues_meta[:20], 1)
            ],
        }
    return {"@context": "https://schema.org", "@graph": [website, collection]}


def build_geo_head_tags(
    *,
    page: GeoPage,
    origin: str,
    page_title: str,
    description: str,
    search_enabled: bool = True,
    json_ld: dict | None = None,
) -> str:
    """Build public SEO/GEO <head> tags (canonical, hreflang, OG, Agent discovery, JSON-LD)."""
    canonical = _abs_url(origin, page.canonical_path)
    md_url = _abs_url(origin, "/" + page.rel_md.lstrip("/"))
    feed_url = _abs_url(origin, page.feed_path)
    og_type = "article" if page.page_type == "issue" else "website"
    og_locale = "zh_CN" if page.lang == "zh-CN" else "en_US"

    lines = [
        '<meta name="robots" content="index,follow,max-snippet:-1,max-image-preview:large">',
        f'<meta name="title" content="{_esc_attr(page_title)}">',
        f'<meta name="description" content="{_esc_attr(description)}">',
        f'<link rel="canonical" href="{_esc_attr(canonical)}">',
    ]
    for hreflang, target in page.alternates.items():
        lines.append(
            f'<link rel="alternate" hreflang="{_esc_attr(hreflang)}" href="{_esc_attr(_abs_url(origin, target))}">'
        )
    lines.append(
        f'<link rel="alternate" type="text/markdown" title="Markdown" href="{_esc_attr(md_url)}">'
    )
    lines.append(
        f'<link rel="alternate" type="application/atom+xml" title="AI Digest Atom Feed" href="{_esc_attr(feed_url)}">'
    )
    if search_enabled:
        search_tpl = f"{origin}/.well-known/search?q={{query}}"
        lines.extend([
            f'<link rel="search" type="text/markdown" title="Search for Agents" href="{_esc_attr(search_tpl)}">',
            f'<meta name="agent:instructions" content="{_esc_attr(SEARCH_INSTRUCTIONS)}">',
            f'<meta name="agent:endpoint" content="{_esc_attr(search_tpl)}">',
            '<meta name="agent:method" content="GET">',
            f'<meta name="agent:documentation" content="{_esc_attr(SEARCH_DOCS_URL)}">',
        ])
    lines.extend([
        f'<meta property="og:type" content="{og_type}">',
        '<meta property="og:site_name" content="AI Digest">',
        f'<meta property="og:title" content="{_esc_attr(page_title)}">',
        f'<meta property="og:description" content="{_esc_attr(description)}">',
        f'<meta property="og:url" content="{_esc_attr(canonical)}">',
        f'<meta property="og:locale" content="{og_locale}">',
        '<meta name="twitter:card" content="summary">',
        f'<meta name="twitter:title" content="{_esc_attr(page_title)}">',
        f'<meta name="twitter:description" content="{_esc_attr(description)}">',
    ])
    if page.page_type == "issue" and page.date_iso:
        lines.append(f'<meta property="article:published_time" content="{_esc_attr(page.date_iso)}">')
    if json_ld:
        lines.append(_json_ld_script(json_ld))
    return "\n".join(lines)


def inject_geo_head(
    html_text: str,
    *,
    page: GeoPage,
    origin: str,
    page_title: str,
    description: str,
    search_enabled: bool = True,
    json_ld: dict | None = None,
) -> str:
    """Inject public SEO/GEO metadata block and update <title> idempotently."""
    pat_mark = re.compile(
        re.escape(_META_MARK_START) + r".*?" + re.escape(_META_MARK_END) + r"\n?",
        flags=re.S,
    )
    out = pat_mark.sub("", html_text)
    out = re.sub(r"\s*<meta\s+name=['\"]robots['\"][^>]*>", "", out, flags=re.I)

    escaped_title = htmlmod.escape(page_title, quote=False)
    out = re.sub(
        r"<title>.*?</title>",
        lambda _: f"<title>{escaped_title}</title>",
        out,
        count=1,
        flags=re.I | re.S,
    )
    tags = build_geo_head_tags(
        page=page,
        origin=origin,
        page_title=page_title,
        description=description,
        search_enabled=search_enabled,
        json_ld=json_ld,
    )
    block = f"{_META_MARK_START}\n{tags}\n{_META_MARK_END}\n"
    if re.search(r"<meta charset=", out, flags=re.I):
        return re.sub(
            r"(<meta charset=[^>]*>\s*)",
            lambda m: m.group(1) + block,
            out,
            count=1,
            flags=re.I,
        )
    if re.search(r"<head[^>]*>", out, flags=re.I):
        return re.sub(
            r"(<head[^>]*>)",
            lambda m: m.group(1) + "\n" + block,
            out,
            count=1,
            flags=re.I,
        )
    return out


def _agent_frontmatter_lines(origin: str, search_enabled: bool) -> list[str]:
    if not search_enabled:
        return []
    return [
        "agent:",
        f"  instructions: {_yaml_quote(SEARCH_INSTRUCTIONS)}",
        f"  endpoint: {_yaml_quote(origin + '/.well-known/search?q={query}')}",
        '  method: "GET"',
        f"  documentation: {_yaml_quote(SEARCH_DOCS_URL)}",
    ]


def render_issue_markdown(
    parsed: dict,
    *,
    page: GeoPage,
    origin: str,
    search_enabled: bool = True,
    json_ld: dict | None = None,
) -> str:
    """Render a clean Markdown representation of an issue with YAML frontmatter & JSON-LD."""
    canonical = _abs_url(origin, page.canonical_path)
    md_url = _abs_url(origin, "/" + page.rel_md.lstrip("/"))

    fm = [
        "---",
        f"title: {_yaml_quote(parsed['title'])}",
        f"description: {_yaml_quote(parsed['description'])}",
        f"date: {_yaml_quote(page.date_iso)}",
        f"kind: {_yaml_quote(page.kind)}",
        f"lang: {_yaml_quote(page.lang)}",
        f"canonical: {_yaml_quote(canonical)}",
        f"url: {_yaml_quote(md_url)}",
        f"items_count: {parsed.get('items_count', 0)}",
    ]
    if parsed.get("top_tags"):
        fm.append("tags: " + json.dumps(parsed["top_tags"], ensure_ascii=False))
    fm.extend(_agent_frontmatter_lines(origin, search_enabled))
    fm.append("---")

    lines = list(fm)
    lines.append("")
    lines.append(f"# {parsed['title']}")
    lines.append("")
    meta_parts = [p for p in (parsed.get("period_line"), parsed.get("stats_line")) if p]
    meta_parts.append(f"[HTML]({canonical})")
    meta_parts.append(f"[llms.txt]({origin}/llms.txt)")
    lines.append("> " + " · ".join(meta_parts))
    lines.append("")

    if parsed.get("must"):
        heading = "必读精选" if page.lang == "zh-CN" else "Must Read"
        lines.append(f"## {heading}")
        lines.append("")
        for m in parsed["must"]:
            dek = f" — {m['dek']}" if m.get("dek") else ""
            lines.append(f"- **{m['title']}**{dek}")
        lines.append("")

    sections = parsed.get("sections") or []
    if sections:
        for sec in sections:
            sec_title = sec.get("title") or ("精选条目" if page.lang == "zh-CN" else "Items")
            lines.append(f"## {sec_title}")
            lines.append("")
            for it in sec.get("items", []):
                lines.extend(_format_item_md(it, canonical, page.lang))
    else:
        for it in parsed.get("items", []):
            lines.extend(_format_item_md(it, canonical, page.lang))

    if json_ld:
        lines.append("```json")
        lines.append(json.dumps(json_ld, ensure_ascii=False, separators=(",", ":")))
        lines.append("```")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def _format_item_md(it: dict, canonical: str, lang: str) -> list[str]:
    num = f"{it['number']}. " if it.get("number") else ""
    out = [f"### {num}{it['title']}"]
    meta = []
    if it.get("source"):
        meta.append(f"Source: {it['source']}")
    if it.get("when"):
        meta.append(f"Date: {it['when']}")
    if it.get("tags"):
        meta.append("Tags: " + ", ".join(it["tags"]))
    if it.get("id"):
        meta.append(f"[Permalink]({canonical}#{it['id']})")
    if meta:
        out.append("*" + " | ".join(meta) + "*")
    out.append("")
    if it.get("dek"):
        out.append(it["dek"])
        out.append("")
    if it.get("look"):
        lbl = "值得看" if lang == "zh-CN" else "Why it matters"
        out.append(f"**{lbl}**: {it['look']}")
        out.append("")
    if it.get("detail"):
        lbl = "详情" if lang == "zh-CN" else "Details"
        out.append(f"**{lbl}**: {it['detail']}")
        out.append("")
    if it.get("extra"):
        out.append(it["extra"])
        out.append("")
    if it.get("links"):
        link_md = " · ".join(f"[{lbl}]({href})" for lbl, href in it["links"])
        out.append(f"Links: {link_md}")
        out.append("")
    return out


def render_index_markdown(
    *,
    page: GeoPage,
    origin: str,
    entries: list[dict],
    latest_daily: dict | None = None,
    latest_weekly: dict | None = None,
    search_enabled: bool = True,
    json_ld: dict | None = None,
) -> str:
    """Render clean Markdown for an archive index page (/index.md, /zh/index.md, /en/index.md)."""
    canonical = _abs_url(origin, page.canonical_path)
    md_url = _abs_url(origin, "/" + page.rel_md.lstrip("/"))
    if page.lang == "zh-CN":
        title = "AI Digest — AI 精选日报与周报归档"
        desc = "每日与每周精选前沿 AI 模型发布、研究论文、工程实践与产业动态。"
    else:
        title = "AI Digest — Curated Daily & Weekly AI Briefings"
        desc = "Curated daily and weekly briefings on frontier AI models, research, systems engineering, and industry developments."

    fm = [
        "---",
        f"title: {_yaml_quote(title)}",
        f"description: {_yaml_quote(desc)}",
        f"lang: {_yaml_quote(page.lang)}",
        f"canonical: {_yaml_quote(canonical)}",
        f"url: {_yaml_quote(md_url)}",
    ]
    if page.date_iso:
        fm.append(f"updated: {_yaml_quote(page.date_iso)}")
    fm.extend(_agent_frontmatter_lines(origin, search_enabled))
    fm.append("---")

    lines = list(fm)
    lines.extend([
        "",
        f"# {title}",
        "",
        f"> {desc}",
        "",
        "## Agent Resources" if page.lang != "zh-CN" else "## Agent 与订阅入口",
        "",
        f"- [llms.txt]({origin}/llms.txt): Site directory for AI agents",
        f"- [llms-full.txt]({origin}/llms-full.txt): Expanded Markdown of recent issues",
        f"- [Atom Feed]({_abs_url(origin, page.feed_path)}): Machine-readable subscription feed",
        f"- [Sitemap]({origin}/sitemap.xml): Canonical XML sitemap",
    ])
    if search_enabled:
        lines.append(
            f"- [Search for Agents]({origin}/.well-known/search?q={{query}}): Site-wide keyword & natural-language Markdown search (`GET` and `QUERY`)"
        )
    lines.append("")

    if latest_daily or latest_weekly:
        lines.append("## 最新一期" if page.lang == "zh-CN" else "## Latest Issues")
        lines.append("")
        if latest_daily:
            lines.append(
                f"- **Daily**: [{latest_daily['title']}]({_abs_url(origin, '/' + latest_daily['rel_md'].lstrip('/'))}) "
                f"([HTML]({_abs_url(origin, latest_daily['url_path'])})) — {latest_daily['description']}"
            )
        if latest_weekly:
            lines.append(
                f"- **Weekly**: [{latest_weekly['title']}]({_abs_url(origin, '/' + latest_weekly['rel_md'].lstrip('/'))}) "
                f"([HTML]({_abs_url(origin, latest_weekly['url_path'])})) — {latest_weekly['description']}"
            )
        lines.append("")

    if entries:
        lines.append("## 归档目录" if page.lang == "zh-CN" else "## Archive")
        lines.append("")
        for e in entries[:40]:
            md_href = _abs_url(origin, "/" + e["rel_md"].lstrip("/"))
            html_href = _abs_url(origin, e["url_path"])
            lines.append(
                f"- `{e['date_iso']}` [{e['title']}]({md_href}) ([HTML]({html_href})): {e['description']}"
            )
        lines.append("")

    if json_ld:
        lines.append("```json")
        lines.append(json.dumps(json_ld, ensure_ascii=False, separators=(",", ":")))
        lines.append("```")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def render_robots_txt(
    *, origin: str, content_signal: str, allow_train: bool = True
) -> str:
    """Render RFC 9309 robots.txt with Content Signals and explicit AI crawler rules."""
    lines = [
        "# AI Digest public site — search and AI agent discovery",
        "# Content Signals: https://contentsignals.org/",
        "User-agent: *",
        f"Content-Signal: {content_signal}",
        "Allow: /",
        "",
        "# AI Search & Assistant Crawlers",
    ]
    for bot in AI_SEARCH_BOTS:
        lines.extend([f"User-agent: {bot}", "Allow: /", ""])
    lines.append("# AI Training & Foundation Crawlers")
    train_rule = "Allow: /" if allow_train else "Disallow: /"
    for bot in AI_TRAIN_BOTS:
        lines.extend([f"User-agent: {bot}", train_rule, ""])
    lines.append(f"Sitemap: {origin}/sitemap.xml")
    return "\n".join(lines) + "\n"


def render_sitemap_xml(pages: list[GeoPage], *, origin: str) -> str:
    """Render Sitemaps 0.9 XML with xhtml:link hreflang alternates for canonical pages."""
    lines = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" '
        'xmlns:xhtml="http://www.w3.org/1999/xhtml">',
    ]
    seen: set[str] = set()
    for p in pages:
        if not p.is_canonical:
            continue
        loc = _abs_url(origin, p.canonical_path)
        if loc in seen:
            continue
        seen.add(loc)
        lines.append("  <url>")
        lines.append(f"    <loc>{_esc_xml(loc)}</loc>")
        if p.date_iso:
            lines.append(f"    <lastmod>{_esc_xml(p.date_iso)}</lastmod>")
        if p.page_type == "index":
            lines.append("    <changefreq>daily</changefreq>")
            lines.append("    <priority>1.0</priority>")
        elif p.kind == "weekly":
            lines.append("    <changefreq>weekly</changefreq>")
            lines.append("    <priority>0.9</priority>")
        else:
            lines.append("    <changefreq>monthly</changefreq>")
            lines.append("    <priority>0.8</priority>")
        for hreflang, target in p.alternates.items():
            href = _abs_url(origin, target)
            lines.append(
                f'    <xhtml:link rel="alternate" hreflang="{_esc_xml(hreflang)}" href="{_esc_xml(href)}" />'
            )
        lines.append("  </url>")
    lines.append("</urlset>")
    return "\n".join(lines) + "\n"


def render_atom_feed(
    entries: list[dict],
    *,
    origin: str,
    feed_path: str,
    site_path: str,
    lang: str,
    title: str,
    subtitle: str,
) -> str:
    """Render a valid Atom 1.0 feed listing the latest 20 issues."""
    feed_url = _abs_url(origin, feed_path)
    site_url = _abs_url(origin, site_path)
    updated_date = entries[0]["date_iso"] if entries and entries[0].get("date_iso") else datetime.now(timezone.utc).strftime("%Y-%m-%d")
    updated_rfc = f"{updated_date}T00:00:00Z"

    lines = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<feed xmlns="http://www.w3.org/2005/Atom" xml:lang="{_esc_xml(lang)}">',
        f"  <id>{_esc_xml(feed_url)}</id>",
        f"  <title>{_esc_xml(title)}</title>",
        f"  <subtitle>{_esc_xml(subtitle)}</subtitle>",
        f"  <updated>{updated_rfc}</updated>",
        f'  <link rel="self" type="application/atom+xml" href="{_esc_xml(feed_url)}" />',
        f'  <link rel="alternate" type="text/html" href="{_esc_xml(site_url)}" />',
        "  <author><name>AI Digest</name></author>",
    ]
    for e in entries[:20]:
        entry_url = _abs_url(origin, e["url_path"])
        md_url = _abs_url(origin, "/" + e["rel_md"].lstrip("/"))
        edate = f"{e['date_iso']}T00:00:00Z" if e.get("date_iso") else updated_rfc
        lines.extend([
            "  <entry>",
            f"    <id>{_esc_xml(entry_url)}</id>",
            f"    <title>{_esc_xml(e['title'])}</title>",
            f"    <updated>{edate}</updated>",
            f'    <link rel="alternate" type="text/html" href="{_esc_xml(entry_url)}" />',
            f'    <link rel="alternate" type="text/markdown" href="{_esc_xml(md_url)}" />',
            f"    <summary>{_esc_xml(e['description'])}</summary>",
            "  </entry>",
        ])
    lines.append("</feed>")
    return "\n".join(lines) + "\n"


def render_llms_txt(
    *,
    origin: str,
    entries_by_lang: dict[str, list[dict]],
    latest_links: list[tuple[str, str, str]],
    search_enabled: bool = True,
) -> str:
    """Render concise /llms.txt per llmstxt.org and Cloudflare Agent Readiness guidelines."""
    lines = [
        "# AI Digest",
        "",
        "> Curated bilingual (zh-CN / en) daily and weekly briefings on frontier AI models, research papers, systems engineering, and industry developments. Every page supports `Accept: text/markdown` content negotiation and has a direct `.md` mirror.",
        "",
        "## Agent Endpoints",
        "",
    ]
    if search_enabled:
        lines.append(
            f"- [Search for Agents]({origin}/.well-known/search?q={{query}}): Site-wide Markdown search supporting `GET ?q=...` and HTTP `QUERY` (`text/plain` or JSON `{{\"q\":\"...\"}}`)."
        )
    lines.extend([
        f"- [Full Recent Digest Bundle]({origin}/llms-full.txt): Combined Markdown of the latest daily and weekly issues.",
        f"- [Agent Skills Discovery]({origin}/.well-known/agent-skills/index.json): Machine-readable usage instructions for AI agents.",
        f"- [XML Sitemap]({origin}/sitemap.xml): Canonical URL index with `hreflang` alternates.",
        f"- [Atom Feed]({origin}/feed.xml): Machine-readable subscription feed for new briefings.",
        "",
    ])

    if latest_links:
        lines.append("## Latest Briefings")
        lines.append("")
        for label, md_path, desc in latest_links:
            lines.append(f"- [{label}]({_abs_url(origin, md_path)}): {desc}")
        lines.append("")

    for lg, items in entries_by_lang.items():
        if not items:
            continue
        heading = "Recent Chinese Briefings (zh-CN)" if lg == "zh-CN" else "Recent English Briefings (en)"
        lines.append(f"## {heading}")
        lines.append("")
        for e in items[:25]:
            md_url = _abs_url(origin, "/" + e["rel_md"].lstrip("/"))
            lines.append(f"- [{e['title']} ({e['date_iso']})]({md_url}): {e['description']}")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def render_llms_full_txt(
    *,
    origin: str,
    issue_markdowns: list[tuple[GeoPage, str]],
    max_bytes: int = 512_000,
) -> str:
    """Concatenate recent canonical issue Markdowns into /llms-full.txt within context budget."""
    header = (
        "# AI Digest — Recent Briefings (Full Markdown)\n\n"
        f"> Source: {origin}/ | Directory: {origin}/llms.txt | Search: {origin}/.well-known/search?q={{query}}\n\n"
    )
    chunks = [header]
    used = len(header.encode("utf-8"))
    for page, md_text in issue_markdowns:
        # Strip per-file YAML frontmatter & trailing JSON-LD block to keep llms-full.txt lean
        body = re.sub(r"\A---\n.*?\n---\n+", "", md_text, flags=re.S)
        body = re.sub(r"\n```json\n\{.*?\}\n```\s*\Z", "\n", body, flags=re.S)
        block = f"---\nSource-URL: {_abs_url(origin, page.canonical_path)}\nMarkdown-URL: {_abs_url(origin, '/' + page.rel_md.lstrip('/'))}\n\n{body.strip()}\n\n"
        b_len = len(block.encode("utf-8"))
        if used + b_len > max_bytes and len(chunks) > 1:
            break
        chunks.append(block)
        used += b_len
    return "".join(chunks).rstrip() + "\n"


def render_agent_skill_files(
    *, origin: str, has_lang_paths: bool, search_enabled: bool = True
) -> tuple[str, str]:
    """Return (SKILL.md, index.json) per Agent Skills Discovery RFC v0.2.0."""
    latest_zh = f"{origin}/zh/daily/latest.md" if has_lang_paths else f"{origin}/daily/latest.md"
    latest_en = f"{origin}/en/daily/latest.md" if has_lang_paths else f"{origin}/daily/latest.md"
    search_section = (
        f"""## Site-Wide Search

Query across all archived daily and weekly AI briefings in one call:

```sh
curl --get "{origin}/.well-known/search" --data-urlencode "q=reasoning models"
```

HTTP `QUERY` (RFC 10008) is also supported with `text/plain` or JSON `{{"q": "..."}}`.
"""
        if search_enabled
        else ""
    )
    skill_md = f"""---
name: ai-digest-reader
description: Read and search curated daily and weekly AI briefings from AI Digest in clean Markdown.
---

# AI Digest Reader Skill

Use this site's native Markdown endpoints and search API instead of scraping HTML.

## Quick Start

1. Fetch the site directory: `{origin}/llms.txt`
2. Fetch the latest daily briefing as Markdown:
   - Chinese: `{latest_zh}`
   - English: `{latest_en}`
3. Or request any HTML page URL with `Accept: text/markdown`.

{search_section}
## Feeds & Discovery

- Expanded recent bundle: `{origin}/llms-full.txt`
- XML Sitemap: `{origin}/sitemap.xml`
- Atom Feed: `{origin}/feed.xml`
"""
    digest = hashlib.sha256(skill_md.encode("utf-8")).hexdigest()
    index_obj = {
        "$schema": AGENT_SKILLS_SCHEMA,
        "skills": [
            {
                "name": "ai-digest-reader",
                "type": "skill-md",
                "description": (
                    "Read and search curated daily and weekly AI briefings from AI Digest "
                    "using native Markdown endpoints, llms.txt, and /.well-known/search."
                ),
                "url": "/.well-known/agent-skills/ai-digest-reader/SKILL.md",
                "digest": f"sha256:{digest}",
            }
        ],
    }
    return skill_md, json.dumps(index_obj, ensure_ascii=False, indent=2) + "\n"


def build_search_index(pages: list[GeoPage], *, origin: str) -> dict:
    """Build compact bilingual JSON search index from canonical issue pages."""
    docs: list[dict] = []
    seen_ids: set[str] = set()
    for p in pages:
        if not p.is_canonical or p.page_type != "issue" or not p.parsed:
            continue
        parsed = p.parsed
        canonical_url = _abs_url(origin, p.canonical_path)
        md_path = "/" + p.rel_md.lstrip("/")
        for it in parsed.get("items", []):
            doc_id = f"{p.lang}:{p.kind}:{p.label}:{it.get('id') or it.get('number')}"
            if doc_id in seen_ids:
                continue
            seen_ids.add(doc_id)
            snippet_parts = [x for x in (it.get("dek"), it.get("look"), it.get("detail")) if x]
            snippet = _truncate(" ".join(snippet_parts), 420)
            docs.append({
                "u": f"{canonical_url}#{it['id']}" if it.get("id") else canonical_url,
                "m": md_path,
                "l": p.lang,
                "k": p.kind,
                "d": p.date_iso,
                "it": parsed["title"],
                "t": it["title"],
                "s": it.get("source", ""),
                "g": it.get("tags", []),
                "p": snippet,
                "lk": [href for _, href in it.get("links", [])[:2]],
            })
    return {
        "v": 1,
        "origin": origin,
        "generated": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "docs": docs,
    }


def build_link_header(*, search_enabled: bool = True, feed_path: str = "/feed.xml") -> str:
    """Build RFC 8288 Link header advertising machine-readable discovery endpoints."""
    parts = [
        '</llms.txt>; rel="describedby"; type="text/markdown"',
        '</index.md>; rel="alternate"; type="text/markdown"',
        '</sitemap.xml>; rel="sitemap"; type="application/xml"',
        f'<{feed_path}>; rel="alternate"; type="application/atom+xml"',
    ]
    if search_enabled:
        parts.append('</.well-known/search>; rel="search"; type="text/markdown"')
    return ", ".join(parts)


def render_geo_headers(*, content_signal: str, link_header: str) -> str:
    """Return Cloudflare Pages _headers rules for public GEO discovery & MIME headers."""
    return (
        "/*\n"
        f"  Content-Signal: {content_signal}\n"
        f"  Link: {link_header}\n"
        "  Vary: Accept\n"
        "/404.html\n"
        "  X-Robots-Tag: noindex, nofollow\n"
        "  Cache-Control: no-store\n"
        "/robots.txt\n"
        "  Cache-Control: public, max-age=3600\n"
        "/sitemap.xml\n"
        "  Content-Type: application/xml; charset=utf-8\n"
        "  Cache-Control: public, max-age=3600\n"
        "/feed.xml\n"
        "  Content-Type: application/atom+xml; charset=utf-8\n"
        "  Cache-Control: public, max-age=1800\n"
        "/zh/feed.xml\n"
        "  Content-Type: application/atom+xml; charset=utf-8\n"
        "  Cache-Control: public, max-age=1800\n"
        "/en/feed.xml\n"
        "  Content-Type: application/atom+xml; charset=utf-8\n"
        "  Cache-Control: public, max-age=1800\n"
        "/llms.txt\n"
        "  Content-Type: text/plain; charset=utf-8\n"
        "  Cache-Control: public, max-age=1800\n"
        "/llms-full.txt\n"
        "  Content-Type: text/plain; charset=utf-8\n"
        "  Cache-Control: public, max-age=1800\n"
        "/search-index.json\n"
        "  Content-Type: application/json; charset=utf-8\n"
        "  Cache-Control: public, max-age=1800\n"
        "/.well-known/agent-skills/index.json\n"
        "  Content-Type: application/json; charset=utf-8\n"
        "  Cache-Control: public, max-age=3600\n"
        "/.well-known/agent-skills/ai-digest-reader/SKILL.md\n"
        "  Content-Type: text/markdown; charset=utf-8\n"
        "  Cache-Control: public, max-age=3600\n"
    )


def render_pages_routes() -> str:
    """Return Cloudflare Pages _routes.json bypassing _worker.js on pure static files."""
    obj = {
        "version": 1,
        "include": ["/*"],
        "exclude": [
            "/favicon.svg",
            "/robots.txt",
            "/sitemap.xml",
            "/feed.xml",
            "/zh/feed.xml",
            "/en/feed.xml",
            "/llms.txt",
            "/llms-full.txt",
            "/search-index.json",
            "/.well-known/agent-skills/*",
        ],
    }
    return json.dumps(obj, indent=2) + "\n"


def _parse_headers_file(headers_text: str) -> tuple[dict[str, str], list[tuple[str, str]]]:
    """Extract global /* headers and per-path Cache-Control rules from _headers."""
    global_headers: dict[str, str] = {}
    cache_rules: list[tuple[str, str]] = []
    cur_path = ""
    for raw in headers_text.splitlines():
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        if not raw[0].isspace():
            cur_path = raw.strip()
            continue
        line = raw.strip()
        if line.startswith("!") or ":" not in line:
            continue
        k, v = line.split(":", 1)
        k = k.strip()
        v = v.strip()
        if cur_path == "/*":
            global_headers[k] = v
        elif k.lower() == "cache-control":
            cache_rules.append((cur_path, v))
    return global_headers, cache_rules


def _parse_redirects_file(redirects_text: str) -> list[tuple[str, str, int]]:
    out: list[tuple[str, str, int]] = []
    for raw in redirects_text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) >= 2:
            status = int(parts[2]) if len(parts) >= 3 and parts[2].isdigit() else 302
            out.append((parts[0], parts[1], status))
    return out


def render_pages_worker(
    *,
    origin: str,
    content_signal: str,
    link_header: str,
    global_headers: dict[str, str],
    cache_rules: list[tuple[str, str]],
    redirects: list[tuple[str, str, int]],
    search_enabled: bool = True,
) -> str:
    """Render self-contained Cloudflare Pages _worker.js for Markdown negotiation & /.well-known/search."""
    cfg_json = json.dumps(
        {
            "origin": origin,
            "contentSignal": content_signal,
            "linkHeader": link_header,
            "searchEnabled": search_enabled,
            "globalHeaders": global_headers,
            "cacheRules": cache_rules,
            "redirects": redirects,
        },
        ensure_ascii=False,
    )
    return f"""// Auto-generated by ai-digest (geo.py) for public Cloudflare Pages (ENCRYPT=0).
// Provides Accept: text/markdown content negotiation, /.well-known/search (GET + RFC 10008 QUERY),
// static redirect parity, and preserves build-time SHA-256 CSP & security headers.
const CFG = {cfg_json};
let cachedIndex = null;

function prefersMarkdown(accept) {{
  if (!accept) return false;
  let mdQ = -1;
  let htmlQ = -1;
  let anyQ = -1;
  for (const part of accept.split(",")) {{
    const segs = part.trim().split(";");
    const mime = (segs[0] || "").trim().toLowerCase();
    if (!mime) continue;
    let q = 1.0;
    for (let i = 1; i < segs.length; i++) {{
      const kv = segs[i].trim().split("=");
      if (kv.length === 2 && kv[0].trim().toLowerCase() === "q") {{
        const n = Number.parseFloat(kv[1]);
        if (Number.isFinite(n)) q = Math.max(0, Math.min(1, n));
      }}
    }}
    if (mime === "text/markdown" || mime === "text/x-markdown") mdQ = Math.max(mdQ, q);
    else if (mime === "text/html" || mime === "application/xhtml+xml") htmlQ = Math.max(htmlQ, q);
    else if (mime === "*/*" || mime === "text/*") anyQ = Math.max(anyQ, q);
  }}
  if (mdQ <= 0) return false;
  if (htmlQ >= 0) return mdQ > htmlQ;
  return mdQ >= anyQ;
}}

function matchCacheRule(pathname) {{
  let matched = null;
  for (const [pat, val] of CFG.cacheRules) {{
    if (pat === pathname) {{
      matched = val;
    }} else if (pat.endsWith("/*")) {{
      const prefix = pat.slice(0, -1);
      if (pathname.startsWith(prefix)) matched = val;
    }}
  }}
  return matched;
}}

function applyBaseHeaders(headers, pathname, status, extra) {{
  for (const [k, v] of Object.entries(CFG.globalHeaders)) {{
    headers.set(k, v);
  }}
  headers.set("Content-Signal", CFG.contentSignal);
  headers.set("Link", CFG.linkHeader);
  headers.set("Vary", "Accept");
  if (status === 404) {{
    headers.set("X-Robots-Tag", "noindex, nofollow");
    headers.set("Cache-Control", "no-store");
  }} else {{
    const cc = matchCacheRule(pathname);
    if (cc) headers.set("Cache-Control", cc);
  }}
  if (extra) {{
    for (const [k, v] of Object.entries(extra)) {{
      headers.set(k, v);
    }}
  }}
}}

function mapToMdCandidates(pathname) {{
  const clean = pathname.replace(/\\/+/g, "/");
  if (clean.endsWith("/index.md")) {{
    const base = clean.slice(0, -"/index.md".length);
    return base ? [clean, base + ".md"] : ["/index.md"];
  }}
  if (clean.endsWith(".md")) return [clean];
  if (clean === "/" || clean === "/index" || clean === "/index.html") return ["/index.md"];
  if (clean.endsWith("/")) return [clean + "index.md", clean.slice(0, -1) + ".md"];
  if (clean.endsWith(".html")) return [clean.slice(0, -5) + ".md"];
  return [clean + ".md", clean + "/index.md"];
}}

function tokenize(text) {{
  const out = new Set();
  if (!text) return out;
  const low = String(text).toLowerCase();
  const ascii = low.match(/[a-z0-9][a-z0-9._+-]*/g) || [];
  for (const w of ascii) {{
    if (w.length >= 2 || /^[0-9]$/.test(w)) out.add(w);
  }}
  const cjkRuns = low.match(/[\\u4e00-\\u9fff]+/g) || [];
  for (const run of cjkRuns) {{
    for (let i = 0; i < run.length; i++) {{
      out.add(run[i]);
      if (i + 1 < run.length) out.add(run.slice(i, i + 2));
    }}
  }}
  return out;
}}

async function loadSearchIndex(env, origin) {{
  if (cachedIndex) return cachedIndex;
  const resp = await env.ASSETS.fetch(new Request(origin + "/search-index.json"));
  if (!resp.ok) return null;
  cachedIndex = await resp.json();
  return cachedIndex;
}}

async function handleSearch(request, env, url) {{
  const method = request.method.toUpperCase();
  if (method !== "GET" && method !== "HEAD" && method !== "QUERY") {{
    return new Response("Method Not Allowed\\n", {{
      status: 405,
      headers: {{
        "Content-Type": "text/plain; charset=utf-8",
        Allow: "GET, HEAD, QUERY",
        "Accept-Query": "text/plain, application/json",
      }},
    }});
  }}
  let q = (url.searchParams.get("q") || "").trim();
  let langFilter = (url.searchParams.get("lang") || "").trim().toLowerCase();
  let limit = Number.parseInt(url.searchParams.get("limit") || "10", 10);
  if (method === "QUERY") {{
    const raw = await request.text();
    if (raw.length > 4096) {{
      return new Response("Query body too large (max 4096 bytes).\\n", {{
        status: 413,
        headers: {{
          "Content-Type": "text/markdown; charset=utf-8",
          "Accept-Query": "text/plain, application/json",
        }},
      }});
    }}
    const ct = (request.headers.get("content-type") || "").toLowerCase();
    if (ct.includes("application/json") || raw.trim().startsWith("{{")) {{
      try {{
        const body = JSON.parse(raw);
        if (typeof body.q === "string") q = body.q.trim();
        if (typeof body.lang === "string") langFilter = body.lang.trim().toLowerCase();
        if (Number.isFinite(body.limit)) limit = body.limit;
      }} catch (_) {{
        return new Response("Invalid JSON body; expected {{\\"q\\": \\"...\\"}}.\\n", {{
          status: 400,
          headers: {{
            "Content-Type": "text/markdown; charset=utf-8",
            "Accept-Query": "text/plain, application/json",
          }},
        }});
      }}
    }} else {{
      q = raw.trim();
    }}
  }}
  q = q.replace(/[\\x00-\\x1f\\x7f]/g, " ").trim().slice(0, 300);
  if (!Number.isFinite(limit) || limit < 1) limit = 10;
  if (limit > 20) limit = 20;

  const baseMdHeaders = {{
    "Content-Type": "text/markdown; charset=utf-8",
    "Accept-Query": "text/plain, application/json",
    "Content-Signal": CFG.contentSignal,
    "X-Content-Type-Options": "nosniff",
    "Cache-Control": "public, max-age=300",
  }};
  if (!q) {{
    const usage =
      "# Search for Agents\\n\\n" +
      "Provide a query via `GET /.well-known/search?q=<query>` or `QUERY /.well-known/search` " +
      "with `text/plain` or JSON `{{\\"q\\": \\"...\\"}}`.\\n";
    return new Response(method === "HEAD" ? null : usage, {{ status: 400, headers: baseMdHeaders }});
  }}

  const idx = await loadSearchIndex(env, url.origin);
  if (!idx || !Array.isArray(idx.docs)) {{
    return new Response("Search index unavailable.\\n", {{
      status: 503,
      headers: {{ ...baseMdHeaders, "Retry-After": "60" }},
    }});
  }}

  const qLow = q.toLowerCase();
  const qToks = [...tokenize(q)];
  const scored = [];
  for (const doc of idx.docs) {{
    if (langFilter) {{
      const dl = (doc.l || "").toLowerCase();
      if (dl !== langFilter && !dl.startsWith(langFilter)) continue;
    }}
    const titleLow = (doc.t || "").toLowerCase();
    const tagsLow = (doc.g || []).join(" ").toLowerCase();
    const srcLow = (doc.s || "").toLowerCase();
    const snipLow = (doc.p || "").toLowerCase();
    let score = 0;
    if (titleLow.includes(qLow)) score += 10;
    if (snipLow.includes(qLow)) score += 5;
    if (tagsLow.includes(qLow)) score += 6;
    for (const tok of qToks) {{
      const w = tok.length >= 2 ? 1.5 : 0.4;
      if (titleLow.includes(tok)) score += 4.0 * w;
      if (tagsLow.includes(tok)) score += 3.0 * w;
      if (srcLow.includes(tok)) score += 2.0 * w;
      if (snipLow.includes(tok)) score += 1.0 * w;
    }}
    if (score > 0) {{
      scored.push({{ score, doc }});
    }}
  }}
  scored.sort((a, b) => b.score - a.score || (b.doc.d || "").localeCompare(a.doc.d || ""));
  const top = scored.slice(0, limit);
  const maxScore = top.length ? top[0].score : 1;

  const escQ = q.replace(/"/g, "'");
  const lines = [`# Search results for: "${{escQ}}"`, ""];
  if (!top.length) {{
    lines.push("No matching items found. Try broader keywords or check `/llms.txt` for all briefings.");
  }} else {{
    for (const {{ score, doc }} of top) {{
      const norm = (score / maxScore).toFixed(3);
      const relMd = (doc.m || "").replace(/^\\//, "");
      const tags = (doc.g || []).join(", ");
      lines.push(`## ${{relMd}} (score: ${{norm}})`);
      lines.push("");
      lines.push(`### ${{doc.t}}`);
      const meta = [`Date: ${{doc.d}}`, `Lang: ${{doc.l}}`];
      if (doc.s) meta.push(`Source: ${{doc.s}}`);
      if (tags) meta.push(`Tags: ${{tags}}`);
      lines.push(`- ${{meta.join(" | ")}}`);
      lines.push(`- URL: ${{doc.u}}`);
      lines.push(`- Markdown: ${{CFG.origin}}${{doc.m}}`);
      if (doc.p) {{
        lines.push("");
        lines.push(doc.p);
      }}
      if (Array.isArray(doc.lk) && doc.lk.length) {{
        lines.push("");
        lines.push("Sources: " + doc.lk.join(" , "));
      }}
      lines.push("");
      lines.push("---");
      lines.push("");
    }}
  }}
  const body = lines.join("\\n").trimEnd() + "\\n";
  const byteLen = new TextEncoder().encode(body).byteLength;
  const headers = new Headers(baseMdHeaders);
  headers.set("x-markdown-tokens", String(Math.max(1, Math.floor(byteLen / 4))));
  return new Response(method === "HEAD" ? null : body, {{ status: 200, headers }});
}}

export default {{
  async fetch(request, env) {{
    const url = new URL(request.url);
    const pathname = url.pathname;

    if (CFG.searchEnabled && pathname === "/.well-known/search") {{
      return handleSearch(request, env, url);
    }}

    // Apply build-time _redirects parity (Cloudflare Pages skips _redirects under _worker.js)
    for (const [src, dst, code] of CFG.redirects) {{
      if (pathname === src) {{
        return Response.redirect(new URL(dst, url.origin).toString(), code);
      }}
    }}

    // Direct .md or /index.md URL fallback
    if (pathname.endsWith(".md")) {{
      for (const cand of mapToMdCandidates(pathname)) {{
        const mdResp = await env.ASSETS.fetch(new Request(new URL(cand, url.origin), {{
          method: request.method === "HEAD" ? "HEAD" : "GET",
        }}));
        if (mdResp.ok) {{
          const buf = request.method === "HEAD" ? null : await mdResp.arrayBuffer();
          const headers = new Headers(mdResp.headers);
          const tok = buf ? Math.max(1, Math.floor(buf.byteLength / 4)) : 1;
          applyBaseHeaders(headers, pathname, 200, {{
            "Content-Type": "text/markdown; charset=utf-8",
            "x-markdown-tokens": String(tok),
          }});
          return new Response(buf, {{ status: 200, headers }});
        }}
      }}
    }}

    // Accept: text/markdown content negotiation
    if ((request.method === "GET" || request.method === "HEAD") && prefersMarkdown(request.headers.get("accept"))) {{
      for (const cand of mapToMdCandidates(pathname)) {{
        const mdResp = await env.ASSETS.fetch(new Request(new URL(cand, url.origin), {{
          method: request.method === "HEAD" ? "HEAD" : "GET",
        }}));
        if (mdResp.ok) {{
          const buf = request.method === "HEAD" ? null : await mdResp.arrayBuffer();
          const headers = new Headers(mdResp.headers);
          const tok = buf ? Math.max(1, Math.floor(buf.byteLength / 4)) : 1;
          applyBaseHeaders(headers, pathname, 200, {{
            "Content-Type": "text/markdown; charset=utf-8",
            "x-markdown-tokens": String(tok),
          }});
          return new Response(buf, {{ status: 200, headers }});
        }}
      }}
    }}

    // Default static asset delivery (unmodified HTML body, guaranteed security/GEO headers)
    const assetResp = await env.ASSETS.fetch(request);
    const headers = new Headers(assetResp.headers);
    applyBaseHeaders(headers, pathname, assetResp.status);
    return new Response(assetResp.body, {{
      status: assetResp.status,
      statusText: assetResp.statusText,
      headers,
    }});
  }},
}};
"""


def _prune_empty_dirs(root: Path) -> None:
    if not root.is_dir():
        return
    for d in sorted(
        (p for p in root.rglob("*") if p.is_dir()),
        key=lambda p: len(p.parts),
        reverse=True,
    ):
        try:
            d.rmdir()
        except OSError:
            pass


def clean_stale_geo_files(
    public_dir: Path, keep_rels: set[str] | None = None
) -> None:
    """Remove stale or disallowed GEO artifacts from public_dir.

    When keep_rels is None (ENCRYPT=1), strips EVERY GEO artifact so private sites
    never leak plaintext Markdown, sitemaps, feeds, llms.txt, search index, or worker.
    """
    if not public_dir.is_dir():
        return
    keep = keep_rels or set()
    for rel in GEO_ROOT_FILES + GEO_EXTRA_FILES:
        if rel not in keep:
            p = public_dir / rel
            if p.is_file():
                p.unlink()
    for md_file in sorted(public_dir.rglob("*.md")):
        rel = md_file.relative_to(public_dir).as_posix()
        if rel not in keep:
            md_file.unlink()
    if keep_rels is None:
        wk = public_dir / ".well-known"
        if wk.is_dir():
            for p in sorted(wk.rglob("*"), reverse=True):
                if p.is_file():
                    p.unlink()
                elif p.is_dir():
                    try:
                        p.rmdir()
                    except OSError:
                        pass
    _prune_empty_dirs(public_dir)


def write_public_geo_bundle(
    *,
    public_dir: Path,
    pages: list[GeoPage],
    origin: str,
    content_signal: str,
    allow_train: bool = True,
    has_lang_paths: bool = False,
    search_enabled: bool = True,
    extra_md_files: dict[str, str] | None = None,
) -> list[Path]:
    """Write all static GEO artifacts for a public (ENCRYPT=0) build and prune stale ones."""
    public_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []
    keep_rels: set[str] = set()

    def _write(rel: str, text: str) -> Path:
        p = public_dir / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        data = text.encode("utf-8")
        if not p.is_file() or p.read_bytes() != data:
            p.write_bytes(data)
        keep_rels.add(rel)
        written.append(p)
        return p

    # 1. robots.txt (public with Content Signals + explicit AI bot rules + Sitemap)
    _write(
        "robots.txt",
        render_robots_txt(
            origin=origin, content_signal=content_signal, allow_train=allow_train
        ),
    )

    # 2. Per-page .md mirrors (issues + indexes + latest aliases)
    entries_by_lang: dict[str, list[dict]] = {}
    canonical_issue_mds: list[tuple[GeoPage, str]] = []
    latest_links: list[tuple[str, str, str]] = []
    seen_latest_md: set[str] = set()

    # Pre-build canonical entries index grouped by lang
    for p in pages:
        if p.is_canonical and p.page_type == "issue" and p.parsed:
            entry = {
                "title": p.parsed["title"],
                "description": p.parsed["description"],
                "date_iso": p.date_iso,
                "kind": p.kind,
                "label": p.label,
                "url_path": p.canonical_path,
                "rel_md": p.rel_md,
            }
            entries_by_lang.setdefault(p.lang, []).append(entry)

    for lg in entries_by_lang:
        entries_by_lang[lg].sort(key=lambda e: (e["date_iso"], e["kind"]), reverse=True)

    for p in pages:
        if p.page_type == "issue" and p.parsed:
            jld = build_issue_json_ld(p.parsed, page=p, origin=origin)
            md_text = render_issue_markdown(
                p.parsed,
                page=p,
                origin=origin,
                search_enabled=search_enabled,
                json_ld=jld,
            )
            _write(p.rel_md, md_text)
            if p.is_canonical:
                canonical_issue_mds.append((p, md_text))
            elif p.rel_md.endswith("/latest.md") or p.rel_md == "latest.md":
                if p.rel_md not in seen_latest_md and ("daily/latest.md" in p.rel_md or "weekly/latest.md" in p.rel_md):
                    seen_latest_md.add(p.rel_md)
                    lbl = f"{p.lang} {p.kind.capitalize()} Latest ({p.parsed['title']})"
                    latest_links.append((lbl, "/" + p.rel_md.lstrip("/"), p.parsed["description"]))
        elif p.page_type == "index":
            lang_entries = entries_by_lang.get(p.lang, [])
            lat_daily = next((e for e in lang_entries if e["kind"] == "daily"), None)
            lat_weekly = next((e for e in lang_entries if e["kind"] == "weekly"), None)
            jld = build_index_json_ld(
                page=p,
                origin=origin,
                issues_meta=lang_entries,
                search_enabled=search_enabled,
            )
            md_text = render_index_markdown(
                page=p,
                origin=origin,
                entries=lang_entries,
                latest_daily=lat_daily,
                latest_weekly=lat_weekly,
                search_enabled=search_enabled,
                json_ld=jld,
            )
            _write(p.rel_md, md_text)

    if extra_md_files:
        for rel, md_text in extra_md_files.items():
            _write(rel, md_text)

    # 3. sitemap.xml
    _write("sitemap.xml", render_sitemap_xml(pages, origin=origin))

    # 4. Atom feeds (/feed.xml + per-lang feeds when bilingual)
    all_entries: list[dict] = []
    for lst in entries_by_lang.values():
        all_entries.extend(lst)
    all_entries.sort(key=lambda e: (e["date_iso"], e["kind"]), reverse=True)
    primary_lang = "zh-CN" if "zh-CN" in entries_by_lang else (next(iter(entries_by_lang), "en"))
    primary_entries = entries_by_lang.get(primary_lang, all_entries)
    _write(
        "feed.xml",
        render_atom_feed(
            primary_entries,
            origin=origin,
            feed_path="/feed.xml",
            site_path="/",
            lang=primary_lang,
            title="AI Digest",
            subtitle="Curated daily and weekly frontier AI briefings",
        ),
    )
    if has_lang_paths:
        if entries_by_lang.get("zh-CN"):
            _write(
                "zh/feed.xml",
                render_atom_feed(
                    entries_by_lang["zh-CN"],
                    origin=origin,
                    feed_path="/zh/feed.xml",
                    site_path="/zh/",
                    lang="zh-CN",
                    title="AI Digest（中文）",
                    subtitle="每日与每周前沿 AI 精选日报与周报",
                ),
            )
        if entries_by_lang.get("en"):
            _write(
                "en/feed.xml",
                render_atom_feed(
                    entries_by_lang["en"],
                    origin=origin,
                    feed_path="/en/feed.xml",
                    site_path="/en/",
                    lang="en",
                    title="AI Digest (English)",
                    subtitle="Curated daily and weekly frontier AI briefings",
                ),
            )

    # 5. llms.txt & llms-full.txt
    _write(
        "llms.txt",
        render_llms_txt(
            origin=origin,
            entries_by_lang=entries_by_lang,
            latest_links=latest_links,
            search_enabled=search_enabled,
        ),
    )
    canonical_issue_mds.sort(key=lambda pair: (pair[0].date_iso, pair[0].kind), reverse=True)
    _write(
        "llms-full.txt",
        render_llms_full_txt(origin=origin, issue_markdowns=canonical_issue_mds),
    )

    # 6. Agent Skills Discovery (/.well-known/agent-skills/index.json + SKILL.md)
    skill_md, skill_idx = render_agent_skill_files(
        origin=origin, has_lang_paths=has_lang_paths, search_enabled=search_enabled
    )
    _write(".well-known/agent-skills/ai-digest-reader/SKILL.md", skill_md)
    _write(".well-known/agent-skills/index.json", skill_idx)

    # 7. Search index for /.well-known/search
    if search_enabled:
        idx_obj = build_search_index(pages, origin=origin)
        _write(
            "search-index.json",
            json.dumps(idx_obj, ensure_ascii=False, separators=(",", ":")) + "\n",
        )

    # Preserve _worker.js / _routes.json during static write; finalize_public_worker manages them.
    keep_rels.update({"_worker.js", "_routes.json"})
    clean_stale_geo_files(public_dir, keep_rels=keep_rels)
    return written


def finalize_public_worker(
    public_dir: Path,
    *,
    origin: str,
    content_signal: str,
    search_enabled: bool = True,
    enable_worker: bool = True,
) -> Path | None:
    """Emit _worker.js & _routes.json after finalize_csp_headers so exact CSP hashes are embedded."""
    wp = public_dir / "_worker.js"
    rp = public_dir / "_routes.json"
    if not enable_worker:
        wp.unlink(missing_ok=True)
        rp.unlink(missing_ok=True)
        return None
    headers_path = public_dir / "_headers"
    headers_text = headers_path.read_text(encoding="utf-8") if headers_path.is_file() else ""
    global_headers, cache_rules = _parse_headers_file(headers_text)
    redirects_path = public_dir / "_redirects"
    redirects = (
        _parse_redirects_file(redirects_path.read_text(encoding="utf-8"))
        if redirects_path.is_file()
        else []
    )
    link_header = build_link_header(search_enabled=search_enabled)
    worker_js = render_pages_worker(
        origin=origin,
        content_signal=content_signal,
        link_header=link_header,
        global_headers=global_headers,
        cache_rules=cache_rules,
        redirects=redirects,
        search_enabled=search_enabled,
    )
    wp.write_text(worker_js, encoding="utf-8")
    rp.write_text(render_pages_routes(), encoding="utf-8")
    return wp
