#!/usr/bin/env python3
"""Look up candidate details by short id, arXiv id, URL fragment or keyword
(use this instead of reading all of candidates.jsonl).

Examples:
  python3 fetch/cand.py 2611.12345                       # arXiv id
  python3 fetch/cand.py "orion-3" --chars 300 --max 5    # keyword, longer abstract
  python3 fetch/cand.py a3f9c1 --dir runs/2026-01-15/fetch
  python3 fetch/cand.py 2611.12345 --since 2026-01-12    # weekly: search every day since Monday

Searches runs/<latest date>/fetch and fetch-extra by default (repeat --dir to choose;
--since DATE covers every run from DATE on). Each match prints about 5 lines:
title / source, time, weight, ledger status / URL / authors / first N chars of the abstract.
Exit code: 0 always (no match prints "0 matches" for that key).
"""
import argparse, glob, json, os, re, sys
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.dirname(_os.path.abspath(__file__))))
import i18n
FL = i18n.resolve_lang()
def tr(key, **kw):
    return i18n.ft(FL, key, **kw)
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("keys", nargs="+", metavar="KEY", help="short id, arXiv id, URL fragment or keyword (several allowed)")
    ap.add_argument("--dir", action="append", help="fetch output directory to search (repeatable; default: latest runs/<date>/fetch + fetch-extra)")
    ap.add_argument("--since", metavar="YYYY-MM-DD", help="weekly: search runs/<D>/fetch and fetch-extra (D >= this date) plus gap-fill fetches under runs/*-weekly/fetch (no need to pass --dir per day)")
    ap.add_argument("--chars", type=int, default=500, help="abstract characters to print (default 500)")
    ap.add_argument("--max", type=int, default=3, help="max matches printed per key (default 3)")
    a = ap.parse_args()
    dirs = a.dir
    if a.since:
        dirs = []
        for d in sorted(glob.glob(os.path.join(ROOT, "runs", "20??-??-??"))):
            if os.path.basename(d) >= a.since:
                dirs += [os.path.join(d, "fetch"), os.path.join(d, "fetch-extra")] + sorted(glob.glob(os.path.join(d, "fetch-prev-*")))
        dirs += sorted(glob.glob(os.path.join(ROOT, "runs", "20??-??-??-weekly", "fetch")))[-1:]
    if not dirs:
        days = sorted(d for d in glob.glob(os.path.join(ROOT, "runs", "20??-??-??")))
        dirs = []
        if days:
            dirs = [os.path.join(days[-1], "fetch"), os.path.join(days[-1], "fetch-extra")]
    rows = []
    for d in dirs:
        p = os.path.join(d, "candidates.jsonl")
        if os.path.exists(p):
            rows += [json.loads(l) for l in open(p) if l.strip()]
    for k in a.keys:
        kk = k.lower().lstrip("#")
        hits = [r for r in rows if r["id"].startswith(kk) or r.get("extra", {}).get("arxiv_id") == kk]
        if not hits:
            hits = [r for r in rows if kk in r["url"].lower() or kk in r["title"].lower()]
        if len(dirs) > 2:   # 多天目录（--since）：同一标题只留最后一次出现的那条
            hits = list({r["title"]: r for r in hits}.values())
        if not hits:
            print(f"## {k}: {tr('c_nomatch')}")
            continue
        for r in hits[: a.max]:
            ex = r.get("extra", {})
            flags = [f"[{r['pref']}]", r["source"] + (("+" + ",".join(r["also_in"])) if r.get("also_in") else ""), (r.get("published") or "?")[5:16].replace("T", " ")]
            if r.get("in_ledger"):
                flags.append(tr("c_pushed") if r["in_ledger"] == "pushed" else tr("c_seen"))
            if ex.get("date_unknown"):
                flags.append(tr("c_date_unknown"))
            if ex.get("discovery_only"):
                flags.append(tr("c_discovery"))
            print(f"## {r['title']}\n   {' · '.join(flags)}\n   {r['url']}" + ("".join(f"\n   {kx}: {ex[kx]}" for kx in ("github", "project", "hn") if ex.get(kx))))
            if r.get("authors"):
                print(tr("c_authors") + ", ".join(r["authors"][:4]) + (tr("c_et_al") if len(r["authors"]) > 4 else ""))
            if r.get("summary"):
                print("   " + r["summary"][: a.chars].replace("\n", " "))
        if len(hits) > a.max:
            print(tr("c_more", n=len(hits) - a.max))


if __name__ == "__main__":
    main()
