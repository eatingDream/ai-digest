#!/usr/bin/env python3
"""Slim page fetch for primary-source checks: prints the title plus the first N characters of the body, or
keyword contexts (use instead of fetching a whole page).

Examples:
  python3 fetch/page.py URL [--chars 3000]
  python3 fetch/page.py URL --find "keyword1,keyword2" [--ctx 160]    # verify numbers/dates: only the hits with context
  python3 fetch/page.py URL --meta                                    # title / date / description only
  python3 fetch/page.py URL1 "URL2@@keyword a,keyword b" URL3 [--chars N]   # several pages in parallel, one call;
                                                                      # "@@keywords" after a URL = --find for that URL only
Special cases: arxiv.org/abs/<id> -> title + date + authors + abstract; huggingface.co/<org>/<repo> -> first N chars of
the model card; github.com/<owner>/<repo> -> first N chars of the README (raw). A purely JS-rendered page that yields no
body prints a hint; then use a browser/WebFetch tool. Read-only, no cookies kept, output contains no HTML.
Exit codes: 0 ok; 1 the page could not be fetched (message says "use WebFetch instead"). Labels follow --lang / $DIGEST_LANG.
"""
import argparse
import concurrent.futures as cf
import os
import re
import sys
import threading

import requests
from lxml import html as lhtml

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)
import i18n  # noqa: E402

UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/124 Safari/537.36"
DROP = ["script", "style", "noscript", "nav", "footer", "header", "aside", "form", "svg", "iframe", "button"]

_THREAD_LOCAL = threading.local()
FL = i18n.resolve_lang()


def tr(key, **kw):
    return i18n.ft(FL, key, **kw)


def _session() -> requests.Session:
    sess = getattr(_THREAD_LOCAL, "session", None)
    if sess is None:
        sess = requests.Session()
        sess.headers.update({"User-Agent": UA, "Accept-Language": "en,zh;q=0.8"})
        _THREAD_LOCAL.session = sess
    return sess


def get(url, **kw):
    r = _session().get(url, timeout=25, **kw)
    r.raise_for_status()
    return r


def text_of(doc):
    for bad in DROP:
        for e in doc.xpath(f"//{bad}"):
            parent = e.getparent()
            if parent is not None:
                parent.remove(e)
    main_nodes = doc.xpath("//article|//main|//*[@role='main']")
    root = max(main_nodes, key=lambda e: len(e.text_content())) if main_nodes else doc
    parts = []
    for e in root.iter("h1", "h2", "h3", "p", "li", "pre", "blockquote", "td", "figcaption"):
        t = re.sub(r"\s+", " ", e.text_content()).strip()
        if len(t) < 2:
            continue
        if e.tag in ("h1", "h2", "h3"):
            parts.append("\n## " + t)
        else:
            parts.append(t)
    out, seen = [], set()
    for p in parts:   # 去掉重复行（嵌套 li/p 会重复）
        if p in seen:
            continue
        seen.add(p)
        out.append(p)
    return "\n".join(out)


def meta_of(doc):
    def m(*names):
        for n in names:
            v = doc.xpath(f"//meta[@property='{n}' or @name='{n}']/@content")
            if v:
                return v[0].strip()
        return ""
    title = (doc.xpath("//title/text()") or [""])[0].strip()
    return {"title": m("og:title") or title, "date": m("article:published_time", "date", "datePublished", "og:updated_time"),
            "desc": m("og:description", "description", "twitter:description")}


def _format_find(body: str, keywords: str, ctx_chars: int) -> str:
    out = []
    for kw in [k.strip() for k in keywords.split(",") if k.strip()]:
        hits = [mm.start() for mm in re.finditer(re.escape(kw), body, re.I)][:3]
        out.append("\n" + tr("pg_hits", kw=kw, n=len(hits), plus="+" if len(hits) == 3 else ""))
        for h in hits:
            out.append("…" + body[max(0, h - ctx_chars): h + ctx_chars + len(kw)].replace("\n", " ") + "…")
    return "\n".join(out)


def fetch_page_text(url: str, *, chars: int = 3000, find: str | None = None, ctx_chars: int = 160, meta: bool = False) -> tuple[bool, str]:
    """Fetch a single URL and return (ok, formatted_text)."""
    m = re.match(r"https?://(?:www\.)?arxiv\.org/(?:abs|pdf)/(\d{4}\.\d{4,5})", url)
    if m:
        url = f"https://arxiv.org/abs/{m.group(1)}"
    mg = re.match(r"https?://github\.com/([^/]+)/([^/#?]+)/?$", url)
    if mg:
        for br in ("HEAD", "main", "master"):
            try:
                raw = get(f"https://raw.githubusercontent.com/{mg.group(1)}/{mg.group(2)}/{br}/README.md").text
                head = f"# {mg.group(1)}/{mg.group(2)} README"
                if meta:
                    return True, head
                if find:
                    return True, head + "\n" + _format_find(raw, find, ctx_chars)
                return True, head + "\n" + raw[:chars]
            except Exception:
                continue
    try:
        r = get(url)
    except Exception as e:
        return False, f"ERROR {type(e).__name__}: {str(e)[:150]} → {tr('pg_fallback')}"
    if not r.content or not r.content.strip():
        return True, tr("pg_short", n=0)
    try:   # 先按 UTF-8 解码（requests 对无 charset 头的页面会猜成 ISO-8859-1，作者名会出现 Ã/Ä 乱码）
        doc = lhtml.fromstring(r.content.decode("utf-8"))
    except Exception:
        try:
            doc = lhtml.fromstring(r.content)
        except Exception:
            return True, tr("pg_short", n=len(r.content))
    md = meta_of(doc)
    if m:   # arXiv abs
        ab = " ".join(doc.xpath("//blockquote[contains(@class,'abstract')]//text()")).replace("Abstract:", "").strip()
        dl = " ".join(doc.xpath("//div[@class='dateline']//text()")).strip()
        au = ", ".join(doc.xpath("//div[contains(@class,'authors')]//a/text()")[:8])
        abstract = re.sub(r"\s+", " ", ab)[:chars]
        return True, f"# {md['title']}\n{dl} | {au}\n{abstract}"
    lines = [f"# {md['title']}" + (f"\n{tr('pg_date')}{md['date']}" if md["date"] else "") + (f"\n{tr('pg_desc')}{md['desc'][:300]}" if md["desc"] else "")]
    if meta:
        return True, "\n".join(lines)
    body = text_of(doc)
    if len(body) < 200:
        lines.append(tr("pg_short", n=len(body)))
        return True, "\n".join(lines)
    if find:
        lines.append(_format_find(body, find, ctx_chars))
        return True, "\n".join(lines)
    lines.append(body[:chars] + ("\n" + tr("pg_trunc", n=len(body)) if len(body) > chars else ""))
    return True, "\n".join(lines)


def run_many(a):
    """Fetch multiple URLs in parallel in-process and print in input order."""
    def one(spec):
        u, _, kw = spec.partition("@@")
        _ok, text = fetch_page_text(u, chars=a.chars, find=kw or a.find, ctx_chars=a.ctx, meta=a.meta)
        return text.strip()

    with cf.ThreadPoolExecutor(min(8, len(a.url))) as ex:
        outs = list(ex.map(one, a.url))
    for spec, o in zip(a.url, outs):
        print(f"### {spec.partition('@@')[0]}\n{o}\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("url", nargs="+", metavar="URL[@@kw,kw]", help="page URL(s); append @@kw1,kw2 to run --find on that URL only")
    ap.add_argument("--chars", type=int, default=3000, help="body characters to print (default 3000)")
    ap.add_argument("--find", metavar="KW1,KW2", help="print only the contexts around these keywords")
    ap.add_argument("--ctx", type=int, default=160, help="context characters around a --find hit (default 160)")
    ap.add_argument("--meta", action="store_true", help="print title / date / description only")
    ap.add_argument("--lang", help="language of the output labels: en | zh-CN (default: $DIGEST_LANG, else en)")
    a = ap.parse_args()
    global FL
    try:
        FL = i18n.resolve_lang(a.lang)
    except ValueError as e:
        ap.error(str(e))
    if len(a.url) > 1 or "@@" in a.url[0]:
        return run_many(a)
    ok, out = fetch_page_text(a.url[0], chars=a.chars, find=a.find, ctx_chars=a.ctx, meta=a.meta)
    print(out)
    if not ok:
        sys.exit(1)


if __name__ == "__main__":
    main()
