use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    fs, io,
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, NaiveDate, NaiveTime, SecondsFormat, TimeZone, Utc};
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{
    assets,
    i18n::{self, Lang},
    model::{
        atomic_write_bytes, atomic_write_text, clean_text, escape_json_for_script, files_under,
        full_width_len, html_escape as esc, normalized_safe_url, safe_external_url, strip_marked,
        strip_tracking_params, validate_indexnow_key, xml_escape, ParsedIssue, ParsedItem,
        SearchDoc, SiteConfig, WorkerConfig,
    },
    parser::parse_digest_html_with_lang,
};

pub const CONTENT_SIGNAL_DEFAULT: &str = "ai-train=yes, search=yes, ai-input=yes";
pub const SEARCH_DOCS_URL: &str =
    "https://developers.cloudflare.com/ai-search/how-to/search-for-agents/";
pub const SEARCH_INSTRUCTIONS: &str = "Use this search API to find relevant content across this site with keywords or natural-language queries, instead of crawling individual pages. Results are returned in Markdown.";
pub const AGENT_SKILLS_SCHEMA: &str = "https://schemas.agentskills.io/discovery/0.2.0/schema.json";
pub const PUBLISHER_NAME: &str = "AI Digest";
pub const AUTHOR_NAME: &str = "eatingDream";
pub const HUB_URL: &str = "https://eatingdream.dev";
pub const AUTHOR_URL: &str = "https://github.com/eatingDream";
pub const REPO_URL: &str = "https://github.com/eatingDream/ai-digest";
pub const API_CATALOG_PATH: &str = ".well-known/api-catalog";
pub const OG_CARD_PATH: &str = "og-card.png";
pub const SEARCH_BOTS: &[&str] = &["Googlebot", "Bingbot", "Applebot", "DuckDuckBot"];
pub const AI_SEARCH_BOTS: &[&str] = &[
    "OAI-SearchBot",
    "ChatGPT-User",
    "Claude-SearchBot",
    "Claude-User",
    "PerplexityBot",
    "Perplexity-User",
];
pub const AI_TRAIN_BOTS: &[&str] = &[
    "GPTBot",
    "ClaudeBot",
    "Google-Extended",
    "Applebot-Extended",
    "CCBot",
    "Bytespider",
    "Amazonbot",
];
pub const GEO_ROOT_FILES: &[&str] = &[
    "sitemap.xml",
    "llms.txt",
    "llms-full.txt",
    "feed.xml",
    "rss.xml",
    "feed.json",
    "og-card.png",
    "search-index.json",
    "_worker.js",
    "_routes.json",
];
pub const SKILL_PATH: &str = ".well-known/agent-skills/ai-digest-reader/SKILL.md";
pub const SKILL_INDEX: &str = ".well-known/agent-skills/index.json";
pub const LLMS_FULL_BUDGET: usize = 512 * 1024;
const WORKER_SLOT: &str = "__WORKER_CFG_JSON__";
const DISALLOW_ALL: &str = "User-agent: *\nDisallow: /\n";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageType {
    #[default]
    Issue,
    Index,
}

#[derive(Clone, Debug)]
pub struct GeoPage {
    pub rel_html: String,
    pub url_path: String,
    pub canonical_path: String,
    pub rel_md: String,
    pub lang: Lang,
    pub page_type: PageType,
    pub kind: String,
    pub label: String,
    pub date_iso: String,
    pub is_canonical: bool,
    pub alternates: BTreeMap<String, String>,
    pub feed_path: String,
    pub parsed: Option<ParsedIssue>,
}
impl Default for GeoPage {
    fn default() -> Self {
        Self {
            rel_html: String::new(),
            url_path: "/".into(),
            canonical_path: "/".into(),
            rel_md: String::new(),
            lang: Lang::En,
            page_type: PageType::Issue,
            kind: String::new(),
            label: String::new(),
            date_iso: String::new(),
            is_canonical: true,
            alternates: BTreeMap::new(),
            feed_path: "/feed.xml".into(),
            parsed: None,
        }
    }
}

impl GeoPage {
    pub fn parse_html(&mut self, html: &str) -> Result<()> {
        if self.page_type == PageType::Issue {
            let parsed = parse_digest_html_with_lang(html, Some(self.lang))?;
            ensure!(
                !parsed.items.is_empty(),
                "public issue HTML contains no stories"
            );
            self.parsed = Some(parsed);
        }
        Ok(())
    }
}

pub fn site_origin(domain: &str) -> String {
    static DOMAIN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z0-9.-]+(?::[0-9]{1,5})?$").unwrap());
    let raw = domain.trim();
    let raw = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .unwrap_or(raw)
        .split('/')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.');
    let raw = if DOMAIN.is_match(raw) {
        raw
    } else {
        "digest.example.com"
    };
    format!("https://{raw}")
}

fn abs(origin: &str, path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        return path.to_owned();
    }
    format!(
        "{}/{}",
        origin.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn md_path(page: &GeoPage) -> String {
    format!("/{}", page.rel_md.trim_start_matches('/'))
}

fn item_url(canonical: &str, id: &str) -> String {
    if id.is_empty() {
        canonical.into()
    } else {
        let id: String =
            percent_encoding::utf8_percent_encode(id, percent_encoding::CONTROLS).collect();
        format!("{canonical}#{id}")
    }
}

fn trunc(text: &str, limit: usize) -> String {
    let text = clean_text(text);
    if text.chars().count() <= limit {
        return text;
    }
    let head: String = text.chars().take(limit - 1).collect();
    format!(
        "{}…",
        head.trim_end_matches([' ', ',', '，', '、', ';', '；', '。', '.'])
    )
}

fn plain_math(text: &str) -> String {
    static FORMULA: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\$([^\s$](?:[^$\r\n]*[^\s$])?)\$").unwrap());
    let mut result = String::new();
    let mut pos = 0;
    for captures in FORMULA.captures_iter(text) {
        let formula = captures.get(0).unwrap();
        let embedded_currency = text
            .as_bytes()
            .get(formula.end())
            .is_some_and(u8::is_ascii_digit);
        let escaped = text[..formula.start()]
            .bytes()
            .rev()
            .take_while(|&b| b == b'\\')
            .count()
            % 2
            == 1;
        if embedded_currency || escaped {
            continue;
        }
        result.push_str(&text[pos..formula.start()]);
        result.push_str(&captures[1]);
        pos = formula.end();
    }
    result.push_str(&text[pos..]);
    result
}

fn clean_meta(text: &str) -> String {
    plain_math(&text.replace('\u{2011}', "-"))
}

fn origin_domain(origin: &str) -> &str {
    let raw = origin.trim();
    raw.strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .unwrap_or(raw)
        .split('/')
        .next()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
}

fn custom_domain(origin: &str) -> Option<&str> {
    let domain = origin_domain(origin);
    let host = domain.split(':').next().unwrap_or(domain);
    (!host.is_empty()
        && !host.eq_ignore_ascii_case("digest.example.com")
        && !host.to_ascii_lowercase().ends_with(".pages.dev"))
    .then_some(host)
}

fn site_alternate_names(origin: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(domain) = custom_domain(origin) {
        names.push(format!("AI Digest ({domain})"));
        names.push(domain.to_owned());
    }
    names.push("AI 每日简报".into());
    names.push("Daily AI Briefing".into());
    names
}

fn trunc_headline(text: &str, max_chars: usize, max_fw: f32, lang: Lang) -> String {
    let cleaned = clean_text(&clean_meta(text));
    if cleaned.chars().count() <= max_chars && full_width_len(&cleaned) <= max_fw {
        return cleaned;
    }
    let mut cut = String::new();
    let mut fw = 0.0f32;
    for (count, ch) in cleaned.chars().enumerate() {
        let w = if ch.is_ascii() { 0.5 } else { 1.0 };
        if count + 1 > max_chars.saturating_sub(1) || fw + w > (max_fw - 1.0).max(1.0) {
            break;
        }
        cut.push(ch);
        fw += w;
    }
    if lang == Lang::En {
        let cut_chars = cut.chars().count();
        let next_char = cleaned.chars().nth(cut_chars);
        if next_char.is_some_and(char::is_alphanumeric) {
            if let Some(space) = cut.rfind(' ') {
                if space >= cut.len() * 65 / 100 {
                    cut.truncate(space);
                }
            }
        }
    }
    let trimmed = cut.trim_end_matches([
        ' ', ',', ';', ':', '·', '|', '—', '-', '(', '（', '，', '；', '：', '、', '。', '.',
    ]);
    format!("{trimmed}…")
}

fn seo_index_title(lang: Lang, origin: &str) -> String {
    match custom_domain(origin) {
        Some(domain) => {
            let domain = trunc(domain, 48);
            match lang {
                Lang::ZhCn => {
                    format!("AI Digest ({domain}) — 每日与每周前沿 AI 简报 | 模型·论文·开源工程")
                }
                Lang::En => {
                    format!("AI Digest ({domain}) — Curated Daily & Weekly Frontier AI Briefings")
                }
            }
        }
        None => lang
            .choose(
                "AI Digest — Curated Daily & Weekly Frontier AI Briefings",
                "AI Digest — 每日与每周前沿 AI 简报 | 模型·论文·开源工程",
            )
            .to_owned(),
    }
}

pub fn seo_page_title(page: &GeoPage, origin: &str, fallback_title: &str) -> String {
    if page.page_type == PageType::Index {
        return seo_index_title(page.lang, origin);
    }
    let raw_headline = page.parsed.as_ref().and_then(|parsed| {
        parsed
            .must
            .iter()
            .map(|m| m.title.as_str())
            .find(|s| !s.trim().is_empty())
            .or_else(|| {
                parsed
                    .items
                    .iter()
                    .map(|it| it.title.as_str())
                    .find(|s| !s.trim().is_empty())
            })
    });
    let suffix = if page.kind == "weekly" {
        let stamp = if !page.label.is_empty() {
            page.label.as_str()
        } else {
            page.date_iso.as_str()
        };
        match (page.lang, stamp.is_empty()) {
            (Lang::ZhCn, true) => " — AI Digest 周报".to_owned(),
            (Lang::ZhCn, false) => format!(" — AI Digest 周报 ({stamp})"),
            (Lang::En, true) => " — AI Digest Weekly".to_owned(),
            (Lang::En, false) => format!(" — AI Digest Weekly ({stamp})"),
        }
    } else {
        let stamp = if !page.date_iso.is_empty() {
            page.date_iso.as_str()
        } else {
            page.label.as_str()
        };
        match (page.lang, stamp.is_empty()) {
            (Lang::ZhCn, true) => " — AI Digest 日报".to_owned(),
            (Lang::ZhCn, false) => format!(" — AI Digest 日报 ({stamp})"),
            (Lang::En, true) => " — AI Digest Daily".to_owned(),
            (Lang::En, false) => format!(" — AI Digest Daily ({stamp})"),
        }
    };
    if let Some(raw) = raw_headline {
        let (max_chars, max_fw) = match page.lang {
            Lang::En => (70usize.saturating_sub(suffix.chars().count()).max(26), 40.0),
            Lang::ZhCn => (42, (42.0 - full_width_len(&suffix)).max(18.0)),
        };
        return format!(
            "{}{suffix}",
            trunc_headline(raw, max_chars, max_fw, page.lang)
        );
    }
    let raw_fallback = page
        .parsed
        .as_ref()
        .map(|p| p.title.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback_title);
    let cleaned = clean_text(&clean_meta(raw_fallback));
    if cleaned.is_empty() {
        "AI Digest".into()
    } else if !cleaned.contains("AI Digest") {
        let head = trunc_headline(
            &cleaned,
            page.lang.choose(54, 34),
            page.lang.choose(30.0, 34.0),
            page.lang,
        );
        format!("{head} — AI Digest")
    } else {
        trunc_headline(
            &cleaned,
            page.lang.choose(68, 42),
            page.lang.choose(36.0, 42.0),
            page.lang,
        )
    }
}

fn is_primary_citation_url(url: &str) -> bool {
    let Some(host) = url::Url::parse(url).ok().and_then(|u| {
        u.host_str()
            .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
    }) else {
        return false;
    };
    let host = host.strip_prefix("www.").unwrap_or(&host);
    !matches!(
        host,
        "x.com"
            | "twitter.com"
            | "reddit.com"
            | "old.reddit.com"
            | "news.ycombinator.com"
            | "mp.weixin.qq.com"
    )
}

fn feed_sibling(feed_path: &str, filename: &str) -> String {
    if let Some((dir, _)) = feed_path.rsplit_once('/') {
        if dir.is_empty() {
            format!("/{filename}")
        } else {
            format!("{dir}/{filename}")
        }
    } else {
        format!("/{filename}")
    }
}

fn archive_title(lang: Lang) -> &'static str {
    lang.choose(
        "AI Digest — Curated Daily & Weekly AI Briefings",
        "AI Digest — 每日与每周 AI 简报归档",
    )
}
fn archive_desc(lang: Lang) -> &'static str {
    lang.choose("Curated daily and weekly briefings on frontier AI models, research, systems engineering, and industry developments.",
        "每日与每周前沿 AI 模型发布、研究论文、工程实践与产业动态。")
}

fn archive_desc_for(lang: Lang, entries: &[&GeoPage]) -> String {
    let latest = entries.iter().max_by(|a, b| {
        a.date_iso
            .cmp(&b.date_iso)
            .then_with(|| a.kind.cmp(&b.kind))
    });
    let Some(latest) = latest else {
        return archive_desc(lang).into();
    };
    let headline = latest
        .parsed
        .as_ref()
        .and_then(|issue| {
            issue
                .must
                .first()
                .map(|m| m.title.as_str())
                .or_else(|| issue.items.first().map(|it| it.title.as_str()))
        })
        .map(clean_meta)
        .unwrap_or_default();
    if headline.is_empty() {
        return archive_desc(lang).into();
    }
    let count = entries.len();
    let raw = match lang {
        Lang::En => format!(
            "Curated daily & weekly frontier AI briefings ({count} issues). Latest ({}): {headline}.",
            latest.date_iso
        ),
        Lang::ZhCn => format!(
            "每日与每周前沿 AI 简报归档（共 {count} 期）。最新（{}）：{headline}。",
            latest.date_iso
        ),
    };
    trunc(&raw, lang.choose(158, 88))
}

/// Escape Markdown control syntax and raw HTML angle brackets while preserving
/// `$...$` inline math spans intact so plain text cannot inject active links or HTML.
fn md_escape_text(text: &str) -> String {
    static MATH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\$([^\s$](?:[^$\r\n]*[^\s$])?)\$").unwrap());
    let escape_plain = |s: &str, out: &mut String| {
        for (line_idx, line) in s.split('\n').enumerate() {
            if line_idx > 0 {
                out.push('\n');
            }
            let leading_ws = line.len() - line.trim_start_matches([' ', '\t']).len();
            out.push_str(&line[..leading_ws]);
            let rest = &line[leading_ws..];
            let mut chars = rest.chars().peekable();
            if chars.peek() == Some(&'#') {
                out.push('\\');
            }
            for ch in chars {
                if matches!(ch, '\\' | '<' | '>' | '[' | ']') {
                    out.push('\\');
                }
                out.push(ch);
            }
        }
    };
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for m in MATH.find_iter(text) {
        escape_plain(&text[pos..m.start()], &mut out);
        out.push_str(m.as_str());
        pos = m.end();
    }
    escape_plain(&text[pos..], &mut out);
    out
}

fn publisher_json(origin: &str) -> Value {
    let base = abs(origin, "/");
    let og_image = abs(origin, &format!("/{OG_CARD_PATH}"));
    json!({
        "@type": "Organization",
        "@id": format!("{base}#publisher"),
        "name": PUBLISHER_NAME,
        "alternateName": site_alternate_names(origin),
        "url": base,
        "logo": {
            "@type": "ImageObject",
            "url": og_image,
            "width": 1200,
            "height": 630
        },
        "founder": {
            "@type": "Person",
            "name": AUTHOR_NAME,
            "url": AUTHOR_URL,
            "sameAs": [HUB_URL, AUTHOR_URL]
        },
        "sameAs": [HUB_URL, REPO_URL, AUTHOR_URL]
    })
}

pub fn build_issue_json_ld(parsed: &ParsedIssue, page: &GeoPage, origin: &str) -> Value {
    let canonical = abs(origin, &page.canonical_path);
    let base = abs(origin, "/");
    let og_image = abs(origin, &format!("/{OG_CARD_PATH}"));
    let headline = clean_meta(&parsed.title);
    let description = clean_meta(&parsed.description);
    let mut article = json!({
        "@type": "NewsArticle",
        "@id": format!("{canonical}#article"),
        "headline": headline,
        "description": description,
        "inLanguage": page.lang.hreflang(),
        "isAccessibleForFree": true,
        "url": canonical,
        "mainEntityOfPage": {"@type": "WebPage", "@id": canonical},
        "image": {"@type": "ImageObject", "url": og_image, "width": 1200, "height": 630},
        "author": {"@type": "Person", "name": AUTHOR_NAME, "url": AUTHOR_URL, "sameAs": [HUB_URL, AUTHOR_URL]},
        "isPartOf": {"@type": "WebSite", "@id": format!("{base}#website"), "name": PUBLISHER_NAME, "url": base},
        "publisher": publisher_json(origin),
        "encoding": {"@type": "MediaObject", "encodingFormat": "text/markdown", "contentUrl": abs(origin, &md_path(page))}
    });
    if !page.date_iso.is_empty() {
        article["datePublished"] = json!(page.date_iso);
        article["dateModified"] = json!(page.date_iso);
    }
    if !parsed.top_tags.is_empty() {
        article["keywords"] = json!(parsed.top_tags.join(", "));
        let about: Vec<Value> = parsed
            .top_tags
            .iter()
            .map(|tag| clean_text(&clean_meta(tag)))
            .filter(|tag| !tag.is_empty())
            .map(|name| json!({"@type": "Thing", "name": name}))
            .collect();
        if !about.is_empty() {
            article["about"] = json!(about);
        }
    }
    let mut seen_citations = BTreeSet::new();
    let mut primary_citations = Vec::new();
    let mut other_citations = Vec::new();
    for item in &parsed.items {
        for (_, href) in &item.links {
            let Some(safe) = safe_external_url(href) else {
                continue;
            };
            let clean = strip_tracking_params(&safe);
            if !seen_citations.insert(clean.clone()) {
                continue;
            }
            if is_primary_citation_url(&clean) {
                primary_citations.push(clean);
            } else {
                other_citations.push(clean);
            }
        }
    }
    let citations: Vec<String> = primary_citations
        .into_iter()
        .chain(other_citations)
        .take(25)
        .collect();
    if !citations.is_empty() {
        article["citation"] = json!(citations);
    }
    let items: Vec<_> = parsed
        .items
        .iter()
        .take(20)
        .enumerate()
        .map(|(i, item)| {
            let mut list = json!({
                "@type": "ListItem",
                "position": i + 1,
                "name": clean_meta(&item.title),
                "url": item_url(&canonical, &item.id)
            });
            if !item.dek.is_empty() {
                list["description"] = json!(trunc(&clean_meta(&item.dek), 180));
            }
            list
        })
        .collect();
    if !items.is_empty() {
        article["mainEntity"] = json!({
            "@type": "ItemList",
            "numberOfItems": parsed.items_count,
            "itemListElement": items
        });
    }
    let edition_prefix =
        canonical_edition_prefix(&page.canonical_path, &page.kind).unwrap_or_default();
    let home_url = abs(origin, &format!("{edition_prefix}/"));
    let kind_label = if page.kind == "weekly" {
        page.lang.choose("Weekly", "周报")
    } else {
        page.lang.choose("Daily", "日报")
    };
    let kind_path = if page.kind.is_empty() {
        format!("{edition_prefix}/")
    } else {
        format!("{edition_prefix}/{}/latest", page.kind)
    };
    let breadcrumbs = json!({
        "@type": "BreadcrumbList",
        "@id": format!("{canonical}#breadcrumb"),
        "itemListElement": [
            {"@type": "ListItem", "position": 1, "name": PUBLISHER_NAME, "item": home_url},
            {"@type": "ListItem", "position": 2, "name": kind_label, "item": abs(origin, &kind_path)},
            {"@type": "ListItem", "position": 3, "name": headline, "item": canonical}
        ]
    });
    json!({"@context": "https://schema.org", "@graph": [article, breadcrumbs]})
}

pub fn build_index_json_ld(
    page: &GeoPage,
    origin: &str,
    entries: &[&GeoPage],
    search: bool,
) -> Value {
    let canonical = abs(origin, &page.canonical_path);
    let base = abs(origin, "/");
    let site_id = format!("{base}#website");
    let pub_id = format!("{base}#publisher");
    let description = archive_desc_for(page.lang, entries);
    let search_template = if search {
        abs(origin, "/.well-known/search?q={search_term_string}")
    } else {
        abs(origin, "/?q={search_term_string}")
    };
    let site = json!({
        "@type": "WebSite",
        "@id": site_id,
        "name": PUBLISHER_NAME,
        "alternateName": site_alternate_names(origin),
        "url": base,
        "description": archive_desc(page.lang),
        "inLanguage": ["zh-Hans", "en"],
        "publisher": {"@id": pub_id},
        "sameAs": [HUB_URL, REPO_URL, AUTHOR_URL],
        "potentialAction": {
            "@type": "SearchAction",
            "target": {
                "@type": "EntryPoint",
                "urlTemplate": search_template
            },
            "query-input": "required name=search_term_string"
        }
    });
    let name = page
        .lang
        .choose("AI Digest — Archive", "AI Digest — 归档目录");
    let mut collection = json!({
        "@type": "CollectionPage",
        "@id": canonical,
        "name": name,
        "description": description,
        "url": canonical,
        "inLanguage": page.lang.hreflang(),
        "isPartOf": {"@id": site_id},
        "publisher": {"@id": pub_id}
    });
    if !page.date_iso.is_empty() {
        collection["dateModified"] = json!(page.date_iso);
    }
    if !entries.is_empty() {
        let mut sorted = entries.to_vec();
        sort_recent(&mut sorted);
        let items: Vec<_> = sorted
            .iter()
            .take(20)
            .enumerate()
            .map(|(i, row)| {
                json!({
                    "@type": "ListItem",
                    "position": i + 1,
                    "name": clean_meta(title(row)),
                    "url": abs(origin, &row.canonical_path)
                })
            })
            .collect();
        collection["mainEntity"] = json!({
            "@type": "ItemList",
            "numberOfItems": sorted.len(),
            "itemListElement": items
        });
    }
    json!({
        "@context": "https://schema.org",
        "@graph": [publisher_json(origin), site, collection]
    })
}

pub fn build_geo_head_tags(
    page: &GeoPage,
    origin: &str,
    title: &str,
    description: &str,
    search: bool,
    ld: Option<&Value>,
) -> String {
    let canonical = esc(&abs(origin, &page.canonical_path));
    let md = esc(&abs(origin, &md_path(page)));
    let atom_feed = esc(&abs(origin, &page.feed_path));
    let rss_feed = esc(&abs(origin, &feed_sibling(&page.feed_path, "rss.xml")));
    let json_feed = esc(&abs(origin, &feed_sibling(&page.feed_path, "feed.json")));
    let og_image = esc(&abs(origin, &format!("/{OG_CARD_PATH}")));
    let title = esc(&clean_meta(title));
    let description = esc(&clean_meta(description));
    let mut tags = format!(
        r#"<meta name="robots" content="index,follow,max-snippet:-1,max-image-preview:large">
<meta name="title" content="{title}">
<meta name="description" content="{description}">
<link rel="canonical" href="{canonical}">
"#
    );
    let mut has_other_lang = false;
    for (key, target) in page
        .alternates
        .iter()
        .filter(|(k, _)| k.as_str() != "x-default")
        .chain(page.alternates.get_key_value("x-default"))
    {
        if normalized_safe_url(target).is_none() {
            continue;
        }
        if key != "x-default" && key != page.lang.hreflang() && key != page.lang.as_str() {
            has_other_lang = true;
        }
        writeln!(
            tags,
            r#"<link rel="alternate" hreflang="{}" href="{}">"#,
            esc(key),
            esc(&abs(origin, target))
        )
        .unwrap();
    }
    write!(
        tags,
        r#"<link rel="alternate" type="text/markdown" title="Markdown" href="{md}">
<link rel="alternate" type="application/atom+xml" title="AI Digest Atom Feed" href="{atom_feed}">
<link rel="alternate" type="application/rss+xml" title="AI Digest RSS Feed" href="{rss_feed}">
<link rel="alternate" type="application/feed+json" title="AI Digest JSON Feed" href="{json_feed}">
"#
    )
    .unwrap();
    if search {
        let endpoint = esc(&abs(origin, "/.well-known/search?q={query}"));
        write!(
            tags,
            r#"<link rel="search" type="text/markdown" title="Search for Agents" href="{endpoint}">
<meta name="agent:instructions" content="{}">
<meta name="agent:endpoint" content="{endpoint}">
<meta name="agent:method" content="GET">
<meta name="agent:documentation" content="{SEARCH_DOCS_URL}">
"#,
            esc(SEARCH_INSTRUCTIONS)
        )
        .unwrap();
    }
    write!(
        tags,
        r#"<meta property="og:type" content="{}">
<meta property="og:site_name" content="{PUBLISHER_NAME}">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{description}">
<meta property="og:url" content="{canonical}">
<meta property="og:locale" content="{}">
"#,
        if page.page_type == PageType::Issue {
            "article"
        } else {
            "website"
        },
        page.lang.choose("en_US", "zh_CN")
    )
    .unwrap();
    if has_other_lang {
        writeln!(
            tags,
            r#"<meta property="og:locale:alternate" content="{}">"#,
            page.lang.other().choose("en_US", "zh_CN")
        )
        .unwrap();
    }
    write!(
        tags,
        r#"<meta property="og:image" content="{og_image}">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="{title}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="{title}">
<meta name="twitter:description" content="{description}">
<meta name="twitter:image" content="{og_image}">
<meta name="twitter:image:alt" content="{title}">
"#
    )
    .unwrap();
    if page.page_type == PageType::Issue && !page.date_iso.is_empty() {
        let d = esc(&page.date_iso);
        writeln!(
            tags,
            r#"<meta property="article:published_time" content="{d}">
<meta property="article:modified_time" content="{d}">"#
        )
        .unwrap();
    }
    if let Some(ld) = ld {
        writeln!(
            tags,
            r#"<script type="application/ld+json">{}</script>"#,
            escape_json_for_script(&ld.to_string())
        )
        .unwrap();
    }
    tags
}

pub fn inject_geo_head(
    html: &str,
    page: &GeoPage,
    origin: &str,
    title: &str,
    description: &str,
    search: bool,
    ld: Option<&Value>,
) -> String {
    static ROBOTS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)<meta\s+name=['"]robots['"][^>]*>"#).unwrap());
    static TITLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<title>.*?</title\s*>").unwrap());
    static HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</head\s*>").unwrap());
    let html = strip_marked(html, "digest-meta");
    let html = strip_marked(&html, "digest-geo-head");
    let html = ROBOTS.replace_all(&html, "");
    let seo_title = seo_page_title(page, origin, title);
    let mut clean = TITLE
        .replacen(&html, 1, |_: &regex::Captures<'_>| {
            format!("<title>{}</title>", esc(&seo_title))
        })
        .into_owned();
    let tags = build_geo_head_tags(page, origin, &seo_title, description, search, ld);
    let block = format!("<!-- digest-geo-head:start --><!-- digest-meta:start -->\n{tags}<!-- digest-meta:end --><!-- digest-geo-head:end -->");
    // Use the original empty slot on an Opus page. The legacy fallback is the
    // closing head, after its renderer's scripts and stylesheet.
    let slot = html_head_slot(&clean);
    if let Some(i) = slot {
        clean.insert_str(i, &block);
    } else if let Some(i) = HEAD.find(&clean) {
        clean.insert_str(i.start(), &block);
    }
    clean
}

fn html_head_slot(html: &str) -> Option<usize> {
    static ICON: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)<link\b[^>]*\brel=["']icon["'][^>]*>"#).unwrap());
    ICON.find(html).map(|m| {
        let end = m.end();
        end + usize::from(html.as_bytes().get(end) == Some(&b'\n'))
    })
}

pub fn decorate_public_html(
    html: &str,
    page: &mut GeoPage,
    canonicals: &[&GeoPage],
    origin: &str,
    search: bool,
) -> Result<String> {
    page.parse_html(html)?;
    let (fallback_title, description, ld) = if let Some(issue) = &page.parsed {
        (
            issue.title.clone(),
            issue.description.clone(),
            build_issue_json_ld(issue, page, origin),
        )
    } else {
        (
            archive_title(page.lang).to_owned(),
            archive_desc_for(page.lang, canonicals),
            build_index_json_ld(page, origin, canonicals, search),
        )
    };
    Ok(inject_geo_head(
        html,
        page,
        origin,
        &fallback_title,
        &description,
        search,
        Some(&ld),
    ))
}

fn quote(value: &str) -> String {
    json!(value).to_string()
}

fn agent_frontmatter(out: &mut String, origin: &str, search: bool) {
    if search {
        writeln!(
            out,
            "agent:\n  instructions: {}\n  endpoint: {}\n  method: \"GET\"\n  documentation: {}",
            quote(SEARCH_INSTRUCTIONS),
            quote(&abs(origin, "/.well-known/search?q={query}")),
            quote(SEARCH_DOCS_URL)
        )
        .unwrap();
    }
}

fn add_json_ld(out: &mut String, ld: Option<&Value>) {
    if let Some(ld) = ld {
        writeln!(out, "```json\n{ld}\n```").unwrap();
    }
}

fn md_link(label: &str, href: &str) -> String {
    // Prevent a legacy link from reintroducing a forbidden scheme in a mirror.
    let label = label
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let Some(href) = normalized_safe_url(href) else {
        return label;
    };
    let href = href
        .replace('(', "%28")
        .replace(')', "%29")
        .replace('<', "%3C")
        .replace('>', "%3E");
    format!("[{label}]({href})")
}

pub fn render_issue_markdown(
    parsed: &ParsedIssue,
    page: &GeoPage,
    origin: &str,
    search: bool,
    ld: Option<&Value>,
) -> String {
    let canonical = abs(origin, &page.canonical_path);
    let md = abs(origin, &md_path(page));
    let mut out = format!("---\ntitle: {}\ndescription: {}\ndate: {}\nkind: {}\nlang: {}\ncanonical: {}\nurl: {}\nitems_count: {}\n",
        quote(&parsed.title), quote(&plain_math(&parsed.description)), quote(&page.date_iso), quote(&page.kind), quote(page.lang.as_str()),
        quote(&canonical), quote(&md), parsed.items_count);
    if !parsed.top_tags.is_empty() {
        writeln!(out, "tags: {}", json!(parsed.top_tags)).unwrap();
    }
    agent_frontmatter(&mut out, origin, search);
    let parts = [&parsed.period_line, &parsed.stats_line]
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.as_str())
        .chain(std::iter::once("[HTML]"))
        .collect::<Vec<_>>()
        .join(" · ");
    writeln!(
        out,
        "---\n\n# {}\n\n> {parts}({canonical}) · [llms.txt]({origin}/llms.txt)\n",
        md_escape_text(&parsed.title)
    )
    .unwrap();
    if !parsed.must.is_empty() {
        writeln!(out, "## {}\n", page.lang.choose("Must Read", "必读精选")).unwrap();
        for row in &parsed.must {
            let title = if row.anchor.starts_with('#') {
                md_link(&row.title, &format!("{canonical}{}", row.anchor))
            } else {
                md_escape_text(&row.title)
            };
            if row.dek.is_empty() {
                writeln!(out, "- **{title}**").unwrap();
            } else {
                writeln!(out, "- **{title}** — {}", md_escape_text(&row.dek)).unwrap();
            }
        }
        out.push('\n');
    }
    if !parsed.themes.is_empty() {
        writeln!(out, "## {}\n", i18n::t(page.lang, "themes")).unwrap();
        for theme in &parsed.themes {
            if !theme.title.is_empty() {
                writeln!(out, "### {}\n", md_escape_text(&theme.title)).unwrap();
            }
            writeln!(out, "{}\n", md_escape_text(&theme.body)).unwrap();
        }
    }
    if parsed.sections.is_empty() {
        for item in &parsed.items {
            format_item(&mut out, item, &canonical, page.lang);
        }
    } else {
        for section in &parsed.sections {
            let title = if section.title.is_empty() {
                page.lang.choose("Items", "精选条目").to_owned()
            } else {
                md_escape_text(&section.title)
            };
            writeln!(out, "## {title}\n").unwrap();
            for item in &section.items {
                format_item(&mut out, item, &canonical, page.lang);
            }
        }
        for item in parsed.items.iter().filter(|item| {
            item.section.is_empty()
                && !parsed
                    .sections
                    .iter()
                    .flat_map(|s| &s.items)
                    .any(|it| it == *item)
        }) {
            format_item(&mut out, item, &canonical, page.lang);
        }
    }
    add_json_ld(&mut out, ld);
    format!("{}\n", out.trim_end())
}

fn format_item(out: &mut String, item: &ParsedItem, canonical: &str, lang: Lang) {
    let number = if item.number.is_empty() {
        String::new()
    } else {
        format!("{}. ", item.number)
    };
    writeln!(out, "### {number}{}", md_escape_text(&item.title)).unwrap();
    let mut parts = Vec::new();
    if !item.source.is_empty() {
        parts.push(format!("Source: {}", md_escape_text(&item.source)));
    }
    if !item.when.is_empty() {
        parts.push(format!("Date: {}", md_escape_text(&item.when)));
    }
    if !item.tags.is_empty() {
        let escaped_tags: Vec<_> = item.tags.iter().map(|t| md_escape_text(t)).collect();
        parts.push(format!("Tags: {}", escaped_tags.join(", ")));
    }
    if !item.id.is_empty() {
        parts.push(md_link("Permalink", &item_url(canonical, &item.id)));
    }
    if !parts.is_empty() {
        writeln!(out, "*{}*", parts.join(" | ")).unwrap();
    }
    out.push('\n');
    for (label, body) in [
        ("", &item.dek),
        (i18n::t(lang, "label_look"), &item.look),
        (i18n::t(lang, "label_detail"), &item.detail),
        ("", &item.extra),
    ] {
        if body.is_empty() {
            continue;
        }
        if !label.is_empty() {
            write!(out, "**{label}**: ").unwrap();
        }
        writeln!(out, "{}\n", md_escape_text(body)).unwrap();
    }
    if !item.links.is_empty() {
        let links = item
            .links
            .iter()
            .map(|(title, href)| md_link(title, href))
            .collect::<Vec<_>>();
        writeln!(out, "Links: {}\n", links.join(" · ")).unwrap();
    }
}

pub fn render_index_markdown(
    page: &GeoPage,
    entries: &[&GeoPage],
    origin: &str,
    search: bool,
    ld: Option<&Value>,
) -> String {
    let heading = seo_index_title(page.lang, origin);
    let desc = archive_desc_for(page.lang, entries);
    let mut out = format!(
        "---\ntitle: {}\ndescription: {}\nlang: {}\ncanonical: {}\nurl: {}\n",
        quote(&heading),
        quote(&desc),
        quote(page.lang.as_str()),
        quote(&abs(origin, &page.canonical_path)),
        quote(&abs(origin, &md_path(page)))
    );
    if !page.date_iso.is_empty() {
        writeln!(out, "updated: {}", quote(&page.date_iso)).unwrap();
    }
    agent_frontmatter(&mut out, origin, search);
    let identity = match page.lang {
        Lang::En => format!(
            "Canonical site: {origin}/ — Independent bilingual (`zh-CN` / `en`) daily and weekly briefings on frontier AI models, research papers, open-source engineering, compute, and safety, curated from ~155 primary AI sources (arXiv, Hugging Face, GitHub, and research labs) by [{AUTHOR_NAME}]({HUB_URL}) ([GitHub]({REPO_URL}))."
        ),
        Lang::ZhCn => format!(
            "官方站点：{origin}/ — 独立双语（`zh-CN` / `en`）每日与每周前沿 AI 简报，从 arXiv、Hugging Face、GitHub 与前沿实验室等约 155 个一手 AI 信源精选模型发布、学术论文、开源工程、算力与安全动态，由 [{AUTHOR_NAME}]({HUB_URL})（[开源仓库]({REPO_URL})）维护发布。"
        ),
    };
    writeln!(
        out,
        "---\n\n# {}\n\n> {identity}\n\n## {}\n",
        md_escape_text(&heading),
        page.lang.choose("Agent Resources", "Agent 与订阅入口")
    )
    .unwrap();
    let rss_url = abs(origin, &feed_sibling(&page.feed_path, "rss.xml"));
    let json_feed_url = abs(origin, &feed_sibling(&page.feed_path, "feed.json"));
    writeln!(out, "- [llms.txt]({origin}/llms.txt): Site directory for AI agents\n- [llms-full.txt]({origin}/llms-full.txt): Expanded Markdown of recent issues\n- [API Catalog]({origin}/{API_CATALOG_PATH}): RFC 9727 machine-readable endpoint catalog\n- [Atom Feed]({}): Atom subscription feed\n- [RSS Feed]({rss_url}): RSS 2.0 subscription feed\n- [JSON Feed]({json_feed_url}): JSON Feed 1.1 subscription feed\n- [Sitemap]({origin}/sitemap.xml): Canonical XML sitemap", abs(origin, &page.feed_path)).unwrap();
    if search {
        writeln!(out, "- [Search for Agents]({origin}/.well-known/search?q={{query}}): Site-wide keyword and natural-language Markdown search (`GET` and `QUERY`)").unwrap();
    }
    out.push('\n');
    if !entries.is_empty() {
        writeln!(
            out,
            "## {}\n",
            page.lang.choose("Latest Issues", "最新一期")
        )
        .unwrap();
        for (kind, label) in [("daily", "Daily"), ("weekly", "Weekly")] {
            if let Some(entry) = entries.iter().find(|e| e.kind == kind) {
                writeln!(
                    out,
                    "- **{label}**: {} ([HTML]({})) — {}",
                    md_link(title(entry), &abs(origin, &md_path(entry))),
                    abs(origin, &entry.canonical_path),
                    md_escape_text(description(entry))
                )
                .unwrap();
            }
        }
        writeln!(out, "\n## {}\n", page.lang.choose("Archive", "归档目录")).unwrap();
        for entry in entries.iter().take(40) {
            writeln!(
                out,
                "- `{}` {} ([HTML]({})): {}",
                entry.date_iso,
                md_link(title(entry), &abs(origin, &md_path(entry))),
                abs(origin, &entry.canonical_path),
                md_escape_text(description(entry))
            )
            .unwrap();
        }
        out.push('\n');
    }
    add_json_ld(&mut out, ld);
    format!("{}\n", out.trim_end())
}

fn title(page: &GeoPage) -> &str {
    page.parsed
        .as_ref()
        .map_or(archive_title(page.lang), |issue| &issue.title)
}
fn description(page: &GeoPage) -> &str {
    page.parsed
        .as_ref()
        .map_or(archive_desc(page.lang), |issue| &issue.description)
}

pub fn render_robots_txt(origin: &str, signal: &str, allow_train: bool, encrypted: bool) -> String {
    if encrypted {
        return DISALLOW_ALL.into();
    }
    let mut out = format!("# AI Digest public site — search and AI agent discovery\n# Content Signals: https://contentsignals.org/\nUser-agent: *\nContent-Signal: {signal}\nAllow: /\n\n# Search Engine Crawlers\n");
    for bot in SEARCH_BOTS {
        write!(out, "User-agent: {bot}\nAllow: /\n\n").unwrap();
    }
    out.push_str("# AI Search & Assistant Crawlers\n");
    for bot in AI_SEARCH_BOTS {
        write!(out, "User-agent: {bot}\nAllow: /\n\n").unwrap();
    }
    out.push_str("# AI Training & Foundation Crawlers\n");
    for bot in AI_TRAIN_BOTS {
        write!(
            out,
            "User-agent: {bot}\n{}: /\n\n",
            if allow_train { "Allow" } else { "Disallow" }
        )
        .unwrap();
    }
    writeln!(out, "Sitemap: {origin}/sitemap.xml").unwrap();
    out
}

struct SitemapEntry<'a> {
    page: &'a GeoPage,
    canonical: String,
    lastmod: &'a str,
    changefreq: &'static str,
    priority: &'static str,
}

fn ordered_sitemap_entries<'a>(pages: &'a [GeoPage], origin: &str) -> Vec<SitemapEntry<'a>> {
    let latest_issue_date = pages
        .iter()
        .filter(|p| p.is_canonical && p.page_type == PageType::Issue && !p.date_iso.is_empty())
        .map(|p| p.date_iso.as_str())
        .max()
        .unwrap_or("");
    let mut indexes = Vec::new();
    let mut issues = Vec::new();
    let mut seen = BTreeSet::new();
    for page in pages.iter().filter(|p| p.is_canonical) {
        if page.page_type == PageType::Index {
            indexes.push(page);
        } else {
            issues.push(page);
        }
    }
    let index_rank = |path: &str| match path {
        "/" => 0u8,
        "/zh/" => 1,
        "/en/" => 2,
        _ => 3,
    };
    indexes.sort_by(|a, b| {
        index_rank(&a.canonical_path)
            .cmp(&index_rank(&b.canonical_path))
            .then_with(|| a.canonical_path.cmp(&b.canonical_path))
    });
    issues.sort_by(|a, b| {
        b.date_iso
            .cmp(&a.date_iso)
            .then_with(|| (a.kind != "weekly").cmp(&(b.kind != "weekly")))
            .then_with(|| (a.lang != Lang::ZhCn).cmp(&(b.lang != Lang::ZhCn)))
            .then_with(|| a.canonical_path.cmp(&b.canonical_path))
    });

    let mut out = Vec::with_capacity(indexes.len() + issues.len());
    for page in indexes {
        let canonical = abs(origin, &page.canonical_path);
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let lastmod = if !page.date_iso.is_empty() {
            page.date_iso.as_str()
        } else {
            latest_issue_date
        };
        out.push(SitemapEntry {
            page,
            canonical,
            lastmod,
            changefreq: "daily",
            priority: "1.0",
        });
    }
    let mut lang_ranks = BTreeMap::<Lang, usize>::new();
    for page in issues {
        let canonical = abs(origin, &page.canonical_path);
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let (changefreq, priority) = if page.kind == "weekly" {
            ("weekly", "0.9")
        } else {
            let rank = lang_ranks.entry(page.lang).or_insert(0);
            let is_recent = *rank < 7;
            *rank += 1;
            ("monthly", if is_recent { "0.9" } else { "0.8" })
        };
        out.push(SitemapEntry {
            page,
            canonical,
            lastmod: page.date_iso.as_str(),
            changefreq,
            priority,
        });
    }
    out
}

pub fn canonical_urls(pages: &[GeoPage], origin: &str) -> Vec<String> {
    ordered_sitemap_entries(pages, origin)
        .into_iter()
        .map(|entry| entry.canonical)
        .collect()
}

pub fn render_sitemap_xml(pages: &[GeoPage], origin: &str) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">\n");
    for entry in ordered_sitemap_entries(pages, origin) {
        writeln!(
            out,
            "  <url>\n    <loc>{}</loc>",
            xml_escape(&entry.canonical)
        )
        .unwrap();
        if !entry.lastmod.is_empty() {
            writeln!(out, "    <lastmod>{}</lastmod>", xml_escape(entry.lastmod)).unwrap();
        }
        writeln!(
            out,
            "    <changefreq>{}</changefreq>\n    <priority>{}</priority>",
            entry.changefreq, entry.priority
        )
        .unwrap();
        for (key, target) in entry
            .page
            .alternates
            .iter()
            .filter(|(k, _)| k.as_str() != "x-default")
            .chain(entry.page.alternates.get_key_value("x-default"))
        {
            if normalized_safe_url(target).is_none() {
                continue;
            }
            writeln!(
                out,
                r#"    <xhtml:link rel="alternate" hreflang="{}" href="{}" />"#,
                xml_escape(key),
                xml_escape(&abs(origin, target))
            )
            .unwrap();
        }
        out.push_str("  </url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

pub fn render_rss_feed(
    entries: &[&GeoPage],
    origin: &str,
    feed_path: &str,
    site_path: &str,
    lang: Lang,
    now: DateTime<Utc>,
) -> String {
    let feed = xml_escape(&abs(origin, feed_path));
    let site = xml_escape(&abs(origin, site_path));
    let title_text = match site_path {
        "/zh/" => "AI Digest（中文）",
        "/en/" => "AI Digest (English)",
        _ => "AI Digest",
    };
    let subtitle = lang.choose(
        "Curated daily and weekly frontier AI briefings",
        "每日与每周前沿 AI 简报",
    );
    let valid_date = |d: &str| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    let rfc2822 = |d: NaiveDate| {
        Utc.from_utc_datetime(&d.and_time(NaiveTime::MIN))
            .to_rfc2822()
    };
    let updated = entries
        .first()
        .and_then(|p| valid_date(&p.date_iso))
        .unwrap_or_else(|| now.date_naive());
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
  <channel>
    <title>{}</title>
    <link>{site}</link>
    <description>{}</description>
    <language>{}</language>
    <lastBuildDate>{}</lastBuildDate>
    <atom:link href="{feed}" rel="self" type="application/rss+xml" />
"#,
        xml_escape(title_text),
        xml_escape(subtitle),
        xml_escape(lang.hreflang()),
        xml_escape(&rfc2822(updated)),
    );
    for entry in entries.iter().take(20) {
        let url = xml_escape(&abs(origin, &entry.canonical_path));
        let date = valid_date(&entry.date_iso).unwrap_or(updated);
        write!(
            out,
            r#"    <item>
      <title>{}</title>
      <link>{url}</link>
      <guid isPermaLink="true">{url}</guid>
      <pubDate>{}</pubDate>
      <description>{}</description>
    </item>
"#,
            xml_escape(&clean_meta(title(entry))),
            xml_escape(&rfc2822(date)),
            xml_escape(&clean_meta(description(entry))),
        )
        .unwrap();
    }
    out.push_str("  </channel>\n</rss>\n");
    out
}

pub fn render_json_feed(
    entries: &[&GeoPage],
    origin: &str,
    feed_path: &str,
    site_path: &str,
    lang: Lang,
) -> String {
    let title_text = match site_path {
        "/zh/" => "AI Digest（中文）",
        "/en/" => "AI Digest (English)",
        _ => "AI Digest",
    };
    let subtitle = lang.choose(
        "Curated daily and weekly frontier AI briefings",
        "每日与每周前沿 AI 简报",
    );
    let items: Vec<Value> = entries
        .iter()
        .take(20)
        .map(|entry| {
            let url = abs(origin, &entry.canonical_path);
            let md = abs(origin, &md_path(entry));
            let desc = clean_meta(description(entry));
            let mut item = json!({
                "id": url,
                "url": url,
                "title": clean_meta(title(entry)),
                "summary": desc,
                "content_text": desc,
                "language": entry.lang.hreflang(),
                "_markdown_url": md
            });
            if !entry.date_iso.is_empty() {
                item["date_published"] = json!(format!("{}T00:00:00Z", entry.date_iso));
            }
            if let Some(parsed) = &entry.parsed {
                if !parsed.top_tags.is_empty() {
                    item["tags"] = json!(parsed.top_tags);
                }
            }
            item
        })
        .collect();
    let feed = json!({
        "version": "https://jsonfeed.org/version/1.1",
        "title": title_text,
        "home_page_url": abs(origin, site_path),
        "feed_url": abs(origin, feed_path),
        "description": subtitle,
        "language": lang.hreflang(),
        "icon": abs(origin, "/favicon.svg"),
        "authors": [{"name": AUTHOR_NAME, "url": AUTHOR_URL}],
        "items": items
    });
    format!("{feed:#}\n")
}

pub fn render_api_catalog(origin: &str, search: bool, lang_paths: bool) -> String {
    let mut items = vec![
        json!({"href": abs(origin, "/sitemap.xml"), "type": "application/xml"}),
        json!({"href": abs(origin, "/feed.xml"), "type": "application/atom+xml"}),
        json!({"href": abs(origin, "/rss.xml"), "type": "application/rss+xml"}),
        json!({"href": abs(origin, "/feed.json"), "type": "application/feed+json"}),
    ];
    if lang_paths {
        for lang_dir in ["/zh/", "/en/"] {
            items.push(json!({"href": abs(origin, &format!("{lang_dir}feed.xml")), "type": "application/atom+xml"}));
            items.push(json!({"href": abs(origin, &format!("{lang_dir}rss.xml")), "type": "application/rss+xml"}));
            items.push(json!({"href": abs(origin, &format!("{lang_dir}feed.json")), "type": "application/feed+json"}));
        }
    }
    let mut root_entry = json!({
        "anchor": abs(origin, "/"),
        "describedby": [
            {"href": abs(origin, "/llms.txt"), "type": "text/markdown"},
            {"href": abs(origin, "/llms-full.txt"), "type": "text/markdown"}
        ],
        "service-doc": [
            {"href": abs(origin, &format!("/{SKILL_PATH}")), "type": "text/markdown"}
        ],
        "item": items
    });
    if search {
        root_entry["search"] = json!([
            {"href": abs(origin, "/.well-known/search"), "type": "text/markdown"}
        ]);
    }
    let mut linkset = vec![root_entry];
    if search {
        linkset.push(json!({
            "anchor": abs(origin, "/.well-known/search"),
            "service-doc": [
                {"href": SEARCH_DOCS_URL, "type": "text/html"},
                {"href": abs(origin, "/llms.txt"), "type": "text/markdown"}
            ],
            "service-desc": [
                {"href": abs(origin, &format!("/{SKILL_PATH}")), "type": "text/markdown"}
            ],
            "describedby": [
                {"href": abs(origin, "/search-index.json"), "type": "application/json"}
            ]
        }));
    }
    linkset.push(json!({
        "anchor": abs(origin, &format!("/{SKILL_INDEX}")),
        "service-desc": [
            {"href": abs(origin, &format!("/{SKILL_PATH}")), "type": "text/markdown"}
        ]
    }));
    let catalog = json!({"linkset": linkset});
    format!("{catalog:#}\n")
}

pub fn render_atom_feed(
    entries: &[&GeoPage],
    origin: &str,
    feed_path: &str,
    site_path: &str,
    lang: Lang,
    now: DateTime<Utc>,
) -> String {
    let feed = xml_escape(&abs(origin, feed_path));
    let site = xml_escape(&abs(origin, site_path));
    let title_text = match site_path {
        "/zh/" => "AI Digest（中文）",
        "/en/" => "AI Digest (English)",
        _ => "AI Digest",
    };
    let subtitle = lang.choose(
        "Curated daily and weekly frontier AI briefings",
        "每日与每周前沿 AI 简报",
    );
    let date = entries.first().map_or("", |p| &p.date_iso);
    let valid_date = |d: &str| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok();
    let updated = valid_date(date).unwrap_or(now.date_naive());
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xml:lang="{lang}">
  <id>{feed}</id>
  <title>{title_text}</title>
  <subtitle>{subtitle}</subtitle>
  <updated>{updated}T00:00:00Z</updated>
  <link rel="self" type="application/atom+xml" href="{feed}" />
  <link rel="alternate" type="text/html" href="{site}" />
  <author><name>AI Digest</name></author>
"#
    );
    for entry in entries.iter().take(20) {
        let url = xml_escape(&abs(origin, &entry.canonical_path));
        let md = xml_escape(&abs(origin, &md_path(entry)));
        let date = valid_date(&entry.date_iso).unwrap_or(updated);
        write!(
            out,
            r#"  <entry>
    <id>{url}</id>
    <title>{}</title>
    <updated>{date}T00:00:00Z</updated>
    <link rel="alternate" type="text/html" href="{url}" />
    <link rel="alternate" type="text/markdown" href="{md}" />
    <summary>{}</summary>
  </entry>
"#,
            xml_escape(&plain_math(title(entry))),
            xml_escape(&plain_math(description(entry)))
        )
        .unwrap();
    }
    out.push_str("</feed>\n");
    out
}

fn canonical_edition_prefix<'a>(canonical_path: &'a str, kind: &str) -> Option<&'a str> {
    let needle = format!("/{kind}/");
    canonical_path
        .find(&needle)
        .map(|idx| &canonical_path[..idx])
}

fn alias_matches_canonical_prefix(page: &GeoPage) -> bool {
    canonical_edition_prefix(&page.canonical_path, &page.kind)
        .is_some_and(|prefix| page.url_path == format!("{prefix}/{}/latest", page.kind))
}

fn dedup_aliases<'a>(aliases: &[&'a GeoPage]) -> Vec<&'a GeoPage> {
    let mut best = BTreeMap::<(Lang, &str), (usize, &'a GeoPage)>::new();
    for (idx, &page) in aliases.iter().enumerate() {
        let key = (page.lang, page.kind.as_str());
        match best.get_mut(&key) {
            None => {
                best.insert(key, (idx, page));
            }
            Some(slot) => {
                if !alias_matches_canonical_prefix(slot.1) && alias_matches_canonical_prefix(page) {
                    slot.1 = page;
                }
            }
        }
    }
    let mut ordered: Vec<(usize, &'a GeoPage)> = best.into_values().collect();
    ordered.sort_by_key(|(idx, _)| *idx);
    ordered.into_iter().map(|(_, page)| page).collect()
}

fn latest_md_for_lang(
    lang: Lang,
    aliases: &[&GeoPage],
    entries: &BTreeMap<Lang, Vec<&GeoPage>>,
) -> Option<String> {
    aliases
        .iter()
        .find(|p| p.lang == lang && p.kind == "daily")
        .or_else(|| {
            aliases
                .iter()
                .find(|p| p.lang == lang && p.kind == "weekly")
        })
        .or_else(|| entries.get(&lang).and_then(|list| list.first()))
        .map(|p| md_path(p))
}

pub fn render_llms_txt(
    origin: &str,
    entries: &BTreeMap<Lang, Vec<&GeoPage>>,
    aliases: &[&GeoPage],
    search: bool,
    worker: bool,
) -> String {
    let negotiation = if worker {
        " Request `Accept: text/markdown` for content negotiation."
    } else {
        ""
    };
    let heading = match custom_domain(origin) {
        Some(domain) => format!("AI Digest ({domain})"),
        None => "AI Digest".to_owned(),
    };
    let mut out = format!(
        "# {heading}\n\n> Canonical site: {origin}/ — Independent bilingual (`zh-CN` / `en`) daily and weekly briefings on frontier AI models, research papers, open-source systems engineering, compute, and safety, curated from ~155 primary AI sources (arXiv, Hugging Face, GitHub, and research labs) by [{AUTHOR_NAME}]({HUB_URL}) ([source repository]({REPO_URL})). Pages have direct `.md` mirrors.{negotiation}\n\n## Agent Endpoints\n\n"
    );
    if search {
        writeln!(out, "- [Search for Agents]({origin}/.well-known/search?q={{query}}): Site-wide Markdown search supporting `GET ?q=...` and HTTP `QUERY` (`text/plain` or JSON `{{\"q\":\"...\"}}`).\n- [Archive Search Index JSON]({origin}/search-index.json): Structured JSON search index of all canonical items.").unwrap();
    }
    writeln!(out, "- [API Catalog (RFC 9727)]({origin}/{API_CATALOG_PATH}): Machine-readable `application/linkset+json` discovery catalog.\n- [Full Recent Digest Bundle]({origin}/llms-full.txt): Combined Markdown of the latest daily and weekly issues.\n- [Agent Skills Discovery]({origin}/{SKILL_INDEX}): Machine-readable usage instructions for AI agents.\n- [XML Sitemap]({origin}/sitemap.xml): Canonical URL index with reciprocal `hreflang` alternates.\n- [Atom Feed]({origin}/feed.xml) · [RSS 2.0 Feed]({origin}/rss.xml) · [JSON Feed 1.1]({origin}/feed.json): Machine-readable subscription feeds.").unwrap();
    let archive_links: Vec<String> = entries
        .iter()
        .filter_map(|(&lang, list)| {
            let first = list.first()?;
            let prefix = canonical_edition_prefix(&first.canonical_path, &first.kind).unwrap_or("");
            Some(format!(
                "[{} Archive Index]({origin}{prefix}/index.md)",
                lang.choose("English", "Chinese")
            ))
        })
        .collect();
    if !archive_links.is_empty() {
        writeln!(
            out,
            "- {}: Full Markdown archive directory.",
            archive_links.join(" · ")
        )
        .unwrap();
    }
    out.push('\n');
    let unique_aliases = dedup_aliases(aliases);
    if !unique_aliases.is_empty() {
        out.push_str("## Latest Briefings\n\n");
        for p in unique_aliases {
            let kind = if p.kind == "weekly" {
                "Weekly"
            } else {
                "Daily"
            };
            let label = format!("{} {kind} Latest ({})", p.lang, title(p));
            writeln!(
                out,
                "- {}: {}",
                md_link(&label, &abs(origin, &md_path(p))),
                md_escape_text(description(p))
            )
            .unwrap();
        }
        out.push('\n');
    }
    for (&lang, entries) in entries {
        if entries.is_empty() {
            continue;
        }
        writeln!(
            out,
            "## {}\n",
            lang.choose(
                "Recent English Briefings (en)",
                "Recent Chinese Briefings (zh-CN)"
            )
        )
        .unwrap();
        for entry in entries.iter().take(25) {
            let label = format!("{} ({})", title(entry), entry.date_iso);
            writeln!(
                out,
                "- {}: {}",
                md_link(&label, &abs(origin, &md_path(entry))),
                md_escape_text(description(entry))
            )
            .unwrap();
        }
        out.push('\n');
    }
    format!("{}\n", out.trim_end())
}

pub fn render_llms_full_txt(origin: &str, issues: &[(&GeoPage, String)], search: bool) -> String {
    let search = if search {
        format!(" | Search: {origin}/.well-known/search?q={{query}}")
    } else {
        String::new()
    };
    let mut out = format!("# AI Digest — Recent Briefings (Full Markdown)\n\n> Source: {origin}/ | Directory: {origin}/llms.txt{search}\n\n");
    for (page, md) in issues {
        let body = md
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---\n"))
            .map_or(md.as_str(), |(_, body)| body);
        let body = body
            .rsplit_once("\n```json\n")
            .filter(|(_, tail)| tail.starts_with('{') && tail.trim_end().ends_with("```"))
            .map_or(body, |(before, _)| before);
        let block = format!(
            "---\nSource-URL: {}\nMarkdown-URL: {}\n\n{}\n\n",
            abs(origin, &page.canonical_path),
            abs(origin, &md_path(page)),
            body.trim()
        );
        if out.len() + block.len() > LLMS_FULL_BUDGET {
            break;
        }
        out.push_str(&block);
    }
    format!("{}\n", out.trim_end())
}

pub fn render_agent_skill_files(
    origin: &str,
    zh_latest: Option<&str>,
    en_latest: Option<&str>,
    search: bool,
    worker: bool,
) -> (String, String) {
    let mut skill = format!("---\nname: ai-digest-reader\ndescription: Read and search curated daily and weekly AI briefings from AI Digest in clean Markdown.\n---\n\n# AI Digest Reader Skill\n\nUse the site's Markdown endpoints to read the briefings.\n\n## Quick Start\n\n1. Fetch the site directory at `{origin}/llms.txt`.\n");
    let has_latest = zh_latest.is_some() || en_latest.is_some();
    if has_latest {
        skill.push_str("2. Fetch the latest daily briefing as Markdown.\n");
        if let Some(zh) = zh_latest {
            writeln!(skill, "   - Chinese at `{origin}{zh}`").unwrap();
        }
        if let Some(en) = en_latest {
            writeln!(skill, "   - English at `{origin}{en}`").unwrap();
        }
    }
    if worker {
        let step = if has_latest { 3 } else { 2 };
        writeln!(
            skill,
            "{step}. Send `Accept: text/markdown` to request Markdown from an HTML page URL."
        )
        .unwrap();
    }
    if search {
        writeln!(skill, "\n## Site-Wide Search\n\n```sh\ncurl --get \"{origin}/.well-known/search\" --data-urlencode \"q=reasoning models\"\n```\n\nHTTP `QUERY` accepts `text/plain` or JSON `{{\"q\": \"...\"}}`.").unwrap();
    }
    writeln!(skill, "\n## Feeds & Discovery\n\n- RFC 9727 API Catalog at `{origin}/{API_CATALOG_PATH}`\n- Expanded recent bundle at `{origin}/llms-full.txt`\n- XML Sitemap at `{origin}/sitemap.xml`\n- Atom Feed at `{origin}/feed.xml`, RSS 2.0 at `{origin}/rss.xml`, JSON Feed 1.1 at `{origin}/feed.json`").unwrap();
    if search {
        writeln!(
            skill,
            "- Archive JSON index at `{origin}/search-index.json`"
        )
        .unwrap();
    }
    let digest = format!("sha256:{}", hex::encode(Sha256::digest(skill.as_bytes())));
    let index = json!({"$schema":AGENT_SKILLS_SCHEMA,"skills":[{"name":"ai-digest-reader","type":"skill-md",
        "description":"Read and search curated daily and weekly AI briefings from AI Digest using native Markdown endpoints and llms.txt.",
        "url":format!("/{SKILL_PATH}"),"digest":digest}]});
    (skill, format!("{index:#}\n"))
}

pub fn build_search_index(pages: &[GeoPage], origin: &str, now: DateTime<Utc>) -> Value {
    let mut docs = Vec::new();
    let mut seen = BTreeSet::new();
    for page in pages
        .iter()
        .filter(|p| p.is_canonical && p.page_type == PageType::Issue)
    {
        let Some(issue) = &page.parsed else {
            continue;
        };
        let url = abs(origin, &page.canonical_path);
        for item in &issue.items {
            let key = if item.id.is_empty() {
                &item.number
            } else {
                &item.id
            };
            if !seen.insert((page.lang, &page.kind, &page.label, key)) {
                continue;
            }
            let snippet = [&item.dek, &item.look, &item.detail]
                .into_iter()
                .filter(|s| !s.is_empty())
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            docs.push(SearchDoc {
                url: item_url(&url, &item.id),
                markdown: md_path(page),
                lang: page.lang,
                kind: page.kind.clone(),
                date: page.date_iso.clone(),
                issue_title: plain_math(&issue.title),
                title: plain_math(&item.title),
                source: item.source.clone(),
                tags: item.tags.clone(),
                snippet: trunc(&plain_math(&snippet), 420),
                links: item
                    .links
                    .iter()
                    .filter_map(|(_, url)| normalized_safe_url(url))
                    .take(2)
                    .collect(),
            });
        }
    }
    json!({"v":1,"origin":origin,"generated":now.to_rfc3339_opts(SecondsFormat::Secs,true),"docs":docs})
}

pub fn build_link_header_for(search: bool, feed: &str, alternate_md: &str) -> String {
    let json_feed = feed_sibling(feed, "feed.json");
    let mut header = format!(
        r#"</llms.txt>; rel="describedby"; type="text/markdown", <{alternate_md}>; rel="alternate"; type="text/markdown", </{API_CATALOG_PATH}>; rel="api-catalog"; type="application/linkset+json", </sitemap.xml>; rel="sitemap"; type="application/xml", <{feed}>; rel="alternate"; type="application/atom+xml", <{json_feed}>; rel="alternate"; type="application/feed+json""#
    );
    if search {
        header.push_str(r#", </.well-known/search>; rel="search"; type="text/markdown""#);
    }
    header
}

pub fn build_link_header(search: bool, feed: &str) -> String {
    build_link_header_for(search, feed, "/index.md")
}

pub fn render_geo_headers(signal: &str, link: &str) -> String {
    render_geo_headers_with_indexnow(signal, link, None)
}

pub fn render_geo_headers_with_indexnow(
    signal: &str,
    link: &str,
    indexnow_key: Option<&str>,
) -> String {
    let mut out = format!("/*\n  Content-Signal: {signal}\n  Link: {link}\n  Vary: Accept\n/404.html\n  X-Robots-Tag: noindex, nofollow\n  Cache-Control: no-store\n/robots.txt\n  Cache-Control: public, max-age=3600\n");
    for (path, kind, age) in [
        ("/sitemap.xml", "application/xml; charset=utf-8", 3600),
        ("/feed.xml", "application/atom+xml; charset=utf-8", 1800),
        ("/zh/feed.xml", "application/atom+xml; charset=utf-8", 1800),
        ("/en/feed.xml", "application/atom+xml; charset=utf-8", 1800),
        ("/rss.xml", "application/rss+xml; charset=utf-8", 1800),
        ("/zh/rss.xml", "application/rss+xml; charset=utf-8", 1800),
        ("/en/rss.xml", "application/rss+xml; charset=utf-8", 1800),
        ("/feed.json", "application/feed+json; charset=utf-8", 1800),
        (
            "/zh/feed.json",
            "application/feed+json; charset=utf-8",
            1800,
        ),
        (
            "/en/feed.json",
            "application/feed+json; charset=utf-8",
            1800,
        ),
        ("/llms.txt", "text/plain; charset=utf-8", 1800),
        ("/llms-full.txt", "text/plain; charset=utf-8", 1800),
        (
            "/search-index.json",
            "application/json; charset=utf-8",
            1800,
        ),
        (
            "/.well-known/api-catalog",
            r#"application/linkset+json; profile="https://www.rfc-editor.org/info/rfc9727""#,
            3600,
        ),
        (
            "/.well-known/agent-skills/index.json",
            "application/json; charset=utf-8",
            3600,
        ),
        (
            "/.well-known/agent-skills/ai-digest-reader/SKILL.md",
            "text/markdown; charset=utf-8",
            3600,
        ),
    ] {
        writeln!(
            out,
            "{path}\n  Content-Type: {kind}\n  Cache-Control: public, max-age={age}"
        )
        .unwrap();
    }
    writeln!(
        out,
        "/og-card.png\n  ! Vary\n  Content-Type: image/png\n  Cache-Control: public, max-age=86400"
    )
    .unwrap();
    if let Some(key) = indexnow_key.filter(|k| !k.is_empty()) {
        writeln!(
            out,
            "/{key}.txt\n  Content-Type: text/plain; charset=utf-8\n  Cache-Control: public, max-age=3600"
        )
        .unwrap();
    }
    out
}

pub fn render_pages_routes() -> String {
    render_pages_routes_with_indexnow(None)
}

pub fn render_pages_routes_with_indexnow(indexnow_key: Option<&str>) -> String {
    let mut exclude = vec![
        "/favicon.svg".to_owned(),
        "/assets/*".into(),
        "/og-card.png".into(),
        "/robots.txt".into(),
        "/sitemap.xml".into(),
        "/feed.xml".into(),
        "/zh/feed.xml".into(),
        "/en/feed.xml".into(),
        "/rss.xml".into(),
        "/zh/rss.xml".into(),
        "/en/rss.xml".into(),
        "/feed.json".into(),
        "/zh/feed.json".into(),
        "/en/feed.json".into(),
        "/llms.txt".into(),
        "/llms-full.txt".into(),
        "/search-index.json".into(),
        "/.well-known/api-catalog".into(),
        "/.well-known/agent-skills/*".into(),
    ];
    if let Some(key) = indexnow_key.filter(|k| !k.is_empty()) {
        exclude.push(format!("/{key}.txt"));
    }
    let data = json!({"version": 1, "include": ["/*"], "exclude": exclude});
    format!("{data:#}\n")
}

pub fn parse_headers(text: &str) -> (BTreeMap<String, String>, Vec<(String, String)>) {
    let mut globals = BTreeMap::new();
    let mut cache = Vec::new();
    let mut path = "";
    for line in text.lines() {
        let field = line.trim();
        if field.is_empty() || field.starts_with('#') {
            continue;
        }
        if line == line.trim_start() {
            path = field;
            continue;
        }
        if let Some((key, value)) = field.split_once(':') {
            if key.starts_with('!') {
                continue;
            }
            if path == "/*" {
                globals.insert(key.trim().into(), value.trim().into());
            } else if !path.is_empty() && key.trim().eq_ignore_ascii_case("cache-control") {
                cache.push((path.into(), value.trim().into()));
            }
        }
    }
    (globals, cache)
}

pub fn parse_redirects(text: &str) -> Vec<(String, String, u16)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let mut parts = line.split_whitespace();
            let source = parts.next()?;
            let dest = parts.next()?;
            let status: u16 = parts.next().map_or(Some(302), |s| s.parse().ok())?;
            if !source.starts_with('/')
                || source.starts_with("//")
                || !dest.starts_with('/')
                || dest.starts_with("//")
                || dest.contains('\\')
                || !matches!(status, 301 | 302 | 307 | 308)
            {
                return None;
            }
            Some((source.into(), dest.into(), status))
        })
        .collect()
}

pub fn inject_worker_config(worker: &str, cfg: &WorkerConfig) -> Result<String> {
    ensure!(
        worker.matches(WORKER_SLOT).count() == 1,
        "embedded worker must have exactly one configuration placeholder"
    );
    let config = escape_json_for_script(&serde_json::to_string(cfg)?)
        .replace(WORKER_SLOT, "\\u005f_WORKER_CFG_JSON__");
    let out = worker.replacen(WORKER_SLOT, &config, 1);
    ensure!(
        !out.contains(WORKER_SLOT),
        "unreplaced worker configuration placeholder"
    );
    Ok(out)
}

pub fn render_pages_worker(cfg: &WorkerConfig) -> Result<String> {
    inject_worker_config(assets::WORKER_JS, cfg)
}

pub fn worker_enabled(config: &SiteConfig) -> bool {
    !config.encrypt && config.platform.eq_ignore_ascii_case("pages") && config.geo_worker
}

pub fn finalize_public_worker(public: &Path, config: &SiteConfig) -> Result<Option<PathBuf>> {
    let worker = public.join("_worker.js");
    let routes = public.join("_routes.json");
    if !worker_enabled(config) {
        remove_file(&worker)?;
        remove_file(&routes)?;
        return Ok(None);
    }
    atomic_write_text(&worker, &public_worker_source(public, config)?)?;
    atomic_write_text(
        &routes,
        &render_pages_routes_with_indexnow(
            Some(config.indexnow_key.as_str()).filter(|k| !k.is_empty()),
        ),
    )?;
    Ok(Some(worker))
}

fn validate_worker_csp_headers(
    raw_headers: &str,
    global_headers: &BTreeMap<String, String>,
) -> Result<()> {
    let mut route = "";
    let mut global_csp_count = 0usize;
    for line in raw_headers.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line == line.trim_start()
            && (line.starts_with('/')
                || line.starts_with("http://")
                || line.starts_with("https://"))
        {
            route = trimmed;
            continue;
        }
        if trimmed.starts_with('!') {
            if route == "/*" {
                anyhow::bail!("global header removals cannot override finalized security headers");
            }
            let target = crate::model::parse_header_removal_target(trimmed)
                .context("malformed header removal directive in _headers")?;
            ensure!(
                !crate::model::is_protected_security_header(target, false),
                "route-level removal of security header is forbidden: {target}"
            );
            continue;
        }
        if let Some((key, _)) = trimmed.split_once(':') {
            let key = key.trim();
            if key.eq_ignore_ascii_case("Content-Security-Policy") {
                if route == "/*" {
                    global_csp_count += 1;
                } else {
                    anyhow::bail!("route-level Content-Security-Policy overrides are forbidden");
                }
            } else if route != "/*" && crate::model::is_protected_security_header(key, false) {
                anyhow::bail!("route-level security header override is forbidden: {key}");
            }
        }
    }
    ensure!(
        global_csp_count == 1,
        "finalized hash-locked Content-Security-Policy is required for the worker"
    );
    let policy = global_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Security-Policy"))
        .map(|(_, v)| v.as_str())
        .context("finalized hash-locked Content-Security-Policy is required for the worker")?;
    let mut directives = BTreeMap::<String, Vec<&str>>::new();
    for raw in policy.split(';') {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        let mut tokens = part.split_whitespace();
        let name = tokens
            .next()
            .context("empty CSP directive")?
            .to_ascii_lowercase();
        let values: Vec<&str> = tokens.collect();
        ensure!(
            directives.insert(name.clone(), values).is_none(),
            "duplicate CSP directive in worker headers: {name}"
        );
    }
    ensure!(
        !directives.contains_key("script-src-elem"),
        "unexpected CSP script-src-elem override"
    );
    for (name, want) in [
        ("default-src", &["'none'"][..]),
        ("script-src-attr", &["'none'"][..]),
        ("frame-ancestors", &["'none'"][..]),
        ("base-uri", &["'none'"][..]),
        ("form-action", &["'none'"][..]),
        ("object-src", &["'none'"][..]),
    ] {
        ensure!(
            directives.get(name).map(Vec::as_slice) == Some(want),
            "finalized hash-locked Content-Security-Policy is required for the worker"
        );
    }
    let script_src = directives
        .get("script-src")
        .context("finalized hash-locked Content-Security-Policy is required for the worker")?;
    ensure!(
        !script_src.is_empty()
            && (script_src.as_slice() == ["'none'"]
                || script_src.iter().all(|tok| {
                    tok.starts_with("'sha256-") && tok.ends_with('\'') && tok.len() > 10
                })),
        "finalized hash-locked Content-Security-Policy is required for the worker"
    );
    Ok(())
}

pub(crate) fn public_worker_config(public: &Path, config: &SiteConfig) -> Result<WorkerConfig> {
    let header_path = public.join("_headers");
    let headers = fs::read_to_string(header_path)
        .context("finalize response headers before building the worker")?;
    let (global_headers, cache_rules) = parse_headers(&headers);
    validate_worker_csp_headers(&headers, &global_headers)?;
    let redirects = match fs::read_to_string(public.join("_redirects")) {
        Ok(s) => parse_redirects(&s),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let link_header = global_headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("Link"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| build_link_header(config.geo_search, "/feed.xml"));
    Ok(WorkerConfig {
        global_headers,
        cache_rules,
        redirects,
        origin: site_origin(&config.domain),
        content_signal: config.content_signal.clone(),
        link_header,
        search_enabled: config.geo_search,
    })
}

pub(crate) fn public_worker_source(public: &Path, config: &SiteConfig) -> Result<String> {
    render_pages_worker(&public_worker_config(public, config)?)
}

fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn indexnow_key_stem(rel: &Path) -> Option<&str> {
    (rel.parent() == Some(Path::new("")))
        .then(|| rel.file_name().and_then(|s| s.to_str()))
        .flatten()
        .filter(|name| !name.eq_ignore_ascii_case("robots.txt"))
        .and_then(|name| name.strip_suffix(".txt"))
        .filter(|stem| !stem.is_empty() && validate_indexnow_key(stem).is_ok())
}

fn is_managed_indexnow_file(public: &Path, rel: &Path) -> bool {
    let Some(stem) = indexnow_key_stem(rel) else {
        return false;
    };
    fs::read_to_string(public.join(rel)).is_ok_and(|body| body == stem)
}

fn is_geo_path(rel: &Path) -> bool {
    rel.starts_with(".well-known")
        || rel
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("md"))
        || rel
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|name| GEO_ROOT_FILES.contains(&name))
}

fn is_geo_header_path(rel: &Path) -> bool {
    is_geo_path(rel) || indexnow_key_stem(rel).is_some()
}

fn prune_geo(public: &Path, keep: Option<&BTreeSet<PathBuf>>) -> Result<()> {
    if keep.is_none() {
        let wk = public.join(".well-known");
        match fs::symlink_metadata(&wk) {
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(wk)?,
            Ok(_) => remove_file(&wk)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    for path in files_under(public)? {
        let relative = path.strip_prefix(public)?;
        if (is_geo_path(relative) || is_managed_indexnow_file(public, relative))
            && keep.is_none_or(|keep| !keep.contains(relative))
        {
            remove_file(&path)?;
        }
    }
    Ok(())
}

pub fn clean_stale_geo_files(public: &Path) -> Result<()> {
    prune_geo(public, None)
}

fn merge_geo_headers(old: &str, geo: Option<&str>) -> String {
    let (mut global, _) = parse_headers(old);
    global.retain(|key, value| {
        let key = key.to_ascii_lowercase();
        !(matches!(key.as_str(), "content-signal" | "link")
            || (key == "vary" && value.eq_ignore_ascii_case("accept"))
            || (geo.is_some() && key == "x-robots-tag"))
    });
    if let Some(geo) = geo {
        global.extend(parse_headers(geo).0);
    }
    let mut rest = String::new();
    let mut favicon = String::new();
    let mut path = "";
    for line in old.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && line == line.trim_start() && line.starts_with('/') {
            path = trimmed;
        }
        if path == "/*"
            || (!path.is_empty() && is_geo_header_path(Path::new(path.trim_start_matches('/'))))
            || (geo.is_some() && matches!(path, "/404.html" | "/robots.txt"))
        {
            continue;
        }
        let block = if path == "/favicon.svg" {
            &mut favicon
        } else {
            &mut rest
        };
        writeln!(block, "{line}").unwrap();
    }
    let mut output = String::new();
    if !global.is_empty() {
        output.push_str("/*\n");
        for (key, value) in global {
            writeln!(output, "  {key}: {value}").unwrap();
        }
    }
    output.push_str(&favicon);
    output.push_str(&rest);
    if let Some(geo) = geo.and_then(|text| text.find("/404.html\n").map(|i| &text[i..])) {
        output.push_str(geo);
    }
    output
}

pub fn write_private_robots(public: &Path) -> Result<()> {
    clean_stale_geo_files(public)?;
    let path = public.join("_headers");
    let old = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let clean = merge_geo_headers(&old, None);
    const NOINDEX: &str = "  X-Robots-Tag: noindex, nofollow\n";
    let headers = if let Some(global) = clean.strip_prefix("/*\n") {
        let mut text = format!("/*\n{NOINDEX}");
        let mut global_block = true;
        for line in global.lines() {
            if line.starts_with('/') {
                global_block = false;
            }
            if global_block
                && line
                    .trim_start()
                    .split_once(':')
                    .is_some_and(|(key, _)| key.eq_ignore_ascii_case("X-Robots-Tag"))
            {
                continue;
            }
            writeln!(text, "{line}").unwrap();
        }
        text
    } else {
        format!("/*\n{NOINDEX}{clean}")
    };
    atomic_write_text(&path, &headers)?;
    atomic_write_text(&public.join("robots.txt"), DISALLOW_ALL)?;
    Ok(())
}

pub fn write_public_geo_bundle(
    public: &Path,
    pages: &[GeoPage],
    config: &SiteConfig,
    lang_paths: bool,
    now: DateTime<Utc>,
    extra_markdown: &BTreeMap<String, String>,
) -> Result<Vec<PathBuf>> {
    if config.encrypt {
        write_private_robots(public)?;
        return Ok(vec![public.join("robots.txt")]);
    }
    let origin = site_origin(&config.domain);
    let search = config.geo_search && worker_enabled(config);
    let mut entries = BTreeMap::<Lang, Vec<&GeoPage>>::new();
    let mut aliases = Vec::new();
    for page in pages {
        validate_md_path(&page.rel_md)?;
        if page.page_type == PageType::Issue {
            ensure!(
                page.parsed.is_some(),
                "GEO bundle is missing parsed issue HTML"
            );
            if page.is_canonical {
                entries.entry(page.lang).or_default().push(page);
            } else if page.rel_md.ends_with("/daily/latest.md")
                || page.rel_md.ends_with("/weekly/latest.md")
                || matches!(page.rel_md.as_str(), "daily/latest.md" | "weekly/latest.md")
            {
                aliases.push(page);
            }
        }
    }
    for path in extra_markdown.keys() {
        validate_md_path(path)?;
    }
    for entries in entries.values_mut() {
        sort_recent(entries);
    }

    let mut canonical_markdown = Vec::new();
    let mut generated = BTreeMap::<PathBuf, String>::new();
    generated.insert(
        "robots.txt".into(),
        render_robots_txt(&origin, &config.content_signal, config.geo_ai_train, false),
    );
    for page in pages {
        let md = if let Some(issue) = &page.parsed {
            let ld = build_issue_json_ld(issue, page, &origin);
            let markdown = render_issue_markdown(issue, page, &origin, search, Some(&ld));
            if page.is_canonical {
                canonical_markdown.push((page, markdown.clone()));
            }
            markdown
        } else {
            let list = entries.get(&page.lang).map_or(&[][..], Vec::as_slice);
            let ld = build_index_json_ld(page, &origin, list, search);
            render_index_markdown(page, list, &origin, search, Some(&ld))
        };
        ensure!(
            generated.insert(PathBuf::from(&page.rel_md), md).is_none(),
            "duplicate GEO Markdown path"
        );
    }
    for (path, content) in extra_markdown {
        ensure!(
            generated.insert(path.into(), content.clone()).is_none(),
            "duplicate GEO Markdown path"
        );
    }
    generated.insert("sitemap.xml".into(), render_sitemap_xml(pages, &origin));
    let primary = if entries.contains_key(&Lang::ZhCn) {
        Lang::ZhCn
    } else {
        Lang::En
    };
    let primary_list = entries.get(&primary).map_or(&[][..], Vec::as_slice);
    generated.insert(
        "feed.xml".into(),
        render_atom_feed(primary_list, &origin, "/feed.xml", "/", primary, now),
    );
    generated.insert(
        "rss.xml".into(),
        render_rss_feed(primary_list, &origin, "/rss.xml", "/", primary, now),
    );
    generated.insert(
        "feed.json".into(),
        render_json_feed(primary_list, &origin, "/feed.json", "/", primary),
    );
    if lang_paths {
        for (lang, path) in [(Lang::ZhCn, "/zh/"), (Lang::En, "/en/")] {
            if let Some(list) = entries.get(&lang) {
                let atom = format!("{path}feed.xml");
                generated.insert(
                    atom.trim_start_matches('/').into(),
                    render_atom_feed(list, &origin, &atom, path, lang, now),
                );
                let rss = format!("{path}rss.xml");
                generated.insert(
                    rss.trim_start_matches('/').into(),
                    render_rss_feed(list, &origin, &rss, path, lang, now),
                );
                let json_f = format!("{path}feed.json");
                generated.insert(
                    json_f.trim_start_matches('/').into(),
                    render_json_feed(list, &origin, &json_f, path, lang),
                );
            }
        }
    }
    canonical_markdown.sort_by_key(|(p, _)| std::cmp::Reverse((&p.date_iso, &p.kind)));
    generated.insert(
        "llms.txt".into(),
        render_llms_txt(&origin, &entries, &aliases, search, worker_enabled(config)),
    );
    generated.insert(
        "llms-full.txt".into(),
        render_llms_full_txt(&origin, &canonical_markdown, search),
    );
    generated.insert(
        API_CATALOG_PATH.into(),
        render_api_catalog(&origin, search, lang_paths),
    );
    let unique_aliases = dedup_aliases(&aliases);
    let zh_latest = latest_md_for_lang(Lang::ZhCn, &unique_aliases, &entries);
    let en_latest = latest_md_for_lang(Lang::En, &unique_aliases, &entries);
    let (skill, index) = render_agent_skill_files(
        &origin,
        zh_latest.as_deref(),
        en_latest.as_deref(),
        search,
        worker_enabled(config),
    );
    generated.insert(SKILL_PATH.into(), skill);
    generated.insert(SKILL_INDEX.into(), index);
    if search {
        let index = build_search_index(pages, &origin, now);
        generated.insert("search-index.json".into(), format!("{index}\n"));
    }
    let indexnow_key = Some(config.indexnow_key.as_str()).filter(|k| !k.is_empty());
    if let Some(key) = indexnow_key {
        validate_indexnow_key(key)?;
        generated.insert(format!("{key}.txt").into(), key.to_owned());
    }
    let old_headers = match fs::read_to_string(public.join("_headers")) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let default_index_md = pages
        .iter()
        .find(|p| p.page_type == PageType::Index && p.rel_md == "index.md")
        .or_else(|| pages.iter().find(|p| p.page_type == PageType::Index))
        .map_or_else(|| "/index.md".into(), md_path);
    let geo_headers = render_geo_headers_with_indexnow(
        &config.content_signal,
        &build_link_header_for(search, "/feed.xml", &default_index_md),
        indexnow_key,
    );
    generated.insert(
        "_headers".into(),
        merge_geo_headers(&old_headers, Some(&geo_headers)),
    );

    let mut keep: BTreeSet<_> = generated.keys().cloned().collect();
    keep.insert(PathBuf::from(OG_CARD_PATH));
    if worker_enabled(config) {
        keep.extend(["_worker.js".into(), "_routes.json".into()]);
    }
    prune_geo(public, Some(&keep))?;
    let mut written = Vec::new();
    for (relative, content) in generated {
        let path = public.join(relative);
        match fs::read(&path) {
            Ok(old) if old == content.as_bytes() => {}
            Ok(_) => atomic_write_text(&path, &content)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => atomic_write_text(&path, &content)?,
            Err(e) => return Err(e.into()),
        }
        written.push(path);
    }
    let og_path = public.join(OG_CARD_PATH);
    match fs::read(&og_path) {
        Ok(old) if old == assets::OG_CARD_PNG => {}
        Ok(_) => atomic_write_bytes(&og_path, assets::OG_CARD_PNG)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            atomic_write_bytes(&og_path, assets::OG_CARD_PNG)?
        }
        Err(e) => return Err(e.into()),
    }
    written.push(og_path);
    Ok(written)
}

fn sort_recent(entries: &mut [&GeoPage]) {
    entries.sort_by_key(|p| std::cmp::Reverse((&p.date_iso, &p.kind)));
}

fn validate_md_path(path: &str) -> Result<()> {
    let relative = Path::new(path);
    ensure!(
        relative
            .components()
            .all(|comp| matches!(comp, Component::Normal(_)))
            && relative.extension().is_some_and(|e| e == "md")
            && !path.contains('\\')
            && !path.split('/').any(|part| matches!(part, "" | "." | "..")),
        "invalid GEO Markdown path"
    );
    Ok(())
}
