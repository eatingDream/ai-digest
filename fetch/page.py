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
import argparse, os, re, sys, json
import requests
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import i18n  # noqa: E402
from lxml import html as lhtml

UA = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/124 Safari/537.36"
DROP = ["script", "style", "noscript", "nav", "footer", "header", "aside", "form", "svg", "iframe", "button"]


def get(url, **kw):
    r = requests.get(url, headers={"User-Agent": UA, "Accept-Language": "en,zh;q=0.8"}, timeout=25, **kw)
    r.raise_for_status()
    return r


def text_of(doc):
    for bad in DROP:
        for e in doc.xpath(f"//{bad}"):
            e.getparent().remove(e) if e.getparent() is not None else None
    main = doc.xpath("//article|//main|//*[@role='main']")
    root = max(main, key=lambda e: len(e.text_content())) if main else doc
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


def run_many(a):
    """多个 URL：每个起一个子进程并行抓取，按输入顺序输出（每条前加 ### URL）。把原来 N 次工具调用并成 1 次。"""
    import subprocess, concurrent.futures as cf

    def one(spec):
        u, _, kw = spec.partition("@@")
        cmd = [sys.executable, os.path.abspath(__file__), u, "--chars", str(a.chars), "--ctx", str(a.ctx)]
        if a.meta:
            cmd.append("--meta")
        if kw or a.find:
            cmd += ["--find", kw or a.find]
        try:
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
            return (r.stdout + r.stderr).strip()
        except Exception as e:
            return f"ERROR {type(e).__name__} → {tr('pg_fallback')}"

    with cf.ThreadPoolExecutor(min(8, len(a.url))) as ex:
        outs = list(ex.map(one, a.url))
    for spec, o in zip(a.url, outs):
        print(f"### {spec.partition('@@')[0]}\n{o}\n")


FL = i18n.resolve_lang()


def tr(key, **kw):
    return i18n.ft(FL, key, **kw)


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
    os.environ["DIGEST_LANG"] = FL  # parallel sub-processes inherit it
    if len(a.url) > 1 or "@@" in a.url[0]:
        return run_many(a)
    url = a.url[0]
    m = re.match(r"https?://(?:www\.)?arxiv\.org/(?:abs|pdf)/(\d{4}\.\d{4,5})", url)
    if m:
        url = f"https://arxiv.org/abs/{m.group(1)}"
    mg = re.match(r"https?://github\.com/([^/]+)/([^/#?]+)/?$", url)
    if mg:
        for br in ("HEAD", "main", "master"):
            try:
                raw = get(f"https://raw.githubusercontent.com/{mg.group(1)}/{mg.group(2)}/{br}/README.md").text
                print(f"# {mg.group(1)}/{mg.group(2)} README\n" + raw[: a.chars])
                return
            except Exception:
                continue
    try:
        r = get(url)
    except Exception as e:
        print(f"ERROR {type(e).__name__}: {str(e)[:150]} → {tr('pg_fallback')}")
        sys.exit(1)
    try:   # 先按 UTF-8 解码（requests 对无 charset 头的页面会猜成 ISO-8859-1，作者名会出现 Ã/Ä 乱码）
        doc = lhtml.fromstring(r.content.decode("utf-8"))
    except Exception:
        doc = lhtml.fromstring(r.content)
    md = meta_of(doc)
    if m:   # arXiv abs
        ab = " ".join(doc.xpath("//blockquote[contains(@class,'abstract')]//text()")).replace("Abstract:", "").strip()
        dl = " ".join(doc.xpath("//div[@class='dateline']//text()")).strip()
        au = ", ".join(doc.xpath("//div[contains(@class,'authors')]//a/text()")[:8])
        abstract = re.sub(r"\s+", " ", ab)[: a.chars]
        print(f"# {md['title']}\n{dl} | {au}\n{abstract}")
        return
    print(f"# {md['title']}" + (f"\n{tr('pg_date')}{md['date']}" if md["date"] else "") + (f"\n{tr('pg_desc')}{md['desc'][:300]}" if md["desc"] else ""))
    if a.meta:
        return
    body = text_of(doc)
    if len(body) < 200:
        print(tr("pg_short", n=len(body)))
        return
    if a.find:
        for kw in [k.strip() for k in a.find.split(",") if k.strip()]:
            hits = [mm.start() for mm in re.finditer(re.escape(kw), body, re.I)][:3]
            print("\n" + tr("pg_hits", kw=kw, n=len(hits), plus="+" if len(hits) == 3 else ""))
            for h in hits:
                print("…" + body[max(0, h - a.ctx): h + a.ctx + len(kw)].replace("\n", " ") + "…")
        return
    print(body[: a.chars] + ("\n" + tr("pg_trunc", n=len(body)) if len(body) > a.chars else ""))


if __name__ == "__main__":
    main()
