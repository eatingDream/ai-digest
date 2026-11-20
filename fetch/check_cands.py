#!/usr/bin/env python3
"""Validate the papers-prefilter output runs/<P>/paper-cands.md(.tmp): every arXiv id must be in
that day's candidates, and the item count must be sane. With --finalize a passing file is renamed
(drops the .tmp suffix).

Examples:
  python3 fetch/check_cands.py runs/2026-01-15/paper-cands.md.tmp
  python3 fetch/check_cands.py runs/2026-01-15/paper-cands.md.tmp --finalize
  python3 fetch/check_cands.py runs/2026-01-15/paper-cands.md.tmp --json

Line format (minimum): `- 2611.12345 | title | one-line reason | tag` (pipe-separated, id first).
Output: `OK n=34 (strong 20 / backup 14) pushed 1 unknown 0 dup 0`; with --json one JSON object
{"ok", "n", "strong", "weak", "pushed", "unknown", "duplicates", "problems", "final"}.
Exit codes: 0 valid; 1 problems found (unknown ids, duplicates, or fewer than 10 / more than 60 items).
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
    ap.add_argument("path", help="paper-cands.md or paper-cands.md.tmp (under runs/<date>/)")
    ap.add_argument("--finalize", action="store_true", help="on success rename FILE.tmp to FILE")
    ap.add_argument("--dir", action="append", help="fetch output dir holding candidates.jsonl (repeatable; default: runs/<date>/fetch + fetch-extra)")
    ap.add_argument("--json", action="store_true", help="print one machine-readable JSON object instead of text")
    a = ap.parse_args()
    path = os.path.abspath(a.path)
    m = re.search(r"runs/(\d{4}-\d{2}-\d{2})/", path)
    dirs = a.dir or ([os.path.join(ROOT, "runs", m.group(1), "fetch"), os.path.join(ROOT, "runs", m.group(1), "fetch-extra")] if m else [])
    known = {}
    for d in dirs:
        f = os.path.join(d, "candidates.jsonl")
        if os.path.exists(f):
            for l in open(f):
                r = json.loads(l)
                aid = (r.get("extra") or {}).get("arxiv_id")
                if aid:
                    known[aid] = r.get("in_ledger")
    ids, strong, weak = [], 0, 0
    tier = ""
    for ln in open(path):
        if ln.startswith("## "):
            tier = ln
        mm = re.match(r"^- (?:[★☆] )?(\d{4}\.\d{4,5})\b", ln)
        if mm:
            ids.append(mm.group(1))
            if any(w in tier.lower() for w in ("强", "必看", "strong", "must")):
                strong += 1
            else:
                weak += 1
    unknown = [i for i in ids if i not in known]
    dup = sorted({i for i in ids if ids.count(i) > 1})
    pushed = [i for i in ids if known.get(i) == "pushed"]
    msg = f"n={len(ids)} ({tr('cc_strong')} {strong} / {tr('cc_weak')} {weak}) {tr('cc_pushed')} {len(pushed)} {tr('cc_unknown')} {len(unknown)} {tr('cc_dup')} {len(dup)}"
    bad = []
    if unknown: bad.append(tr("cc_bad_unknown") + " ".join(unknown[:10]))
    if dup: bad.append(tr("cc_bad_dup") + " ".join(dup[:10]))
    if not 10 <= len(ids) <= 60: bad.append(tr("cc_bad_count", n=len(ids)))
    result = {"ok": not bad, "n": len(ids), "strong": strong, "weak": weak, "pushed": len(pushed),
              "unknown": unknown, "duplicates": dup, "problems": bad, "final": None}
    if bad:
        print(json.dumps(result, ensure_ascii=False) if a.json else "PROBLEM " + msg + "\n  " + "\n  ".join(bad))
        return 1
    final = None
    if a.finalize:
        final = re.sub(r"\.tmp$", "", path)
        os.replace(path, final)
    result["final"] = final
    if a.json:
        print(json.dumps(result, ensure_ascii=False))
    else:
        print(f"OK {msg} → {final}" if final else f"OK {msg}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
