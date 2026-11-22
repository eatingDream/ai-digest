#!/usr/bin/env python3
"""Compress X search results (JSON files saved from search_posts_all / search_news) into compact lead lines.

Each input file is one tool call result, written to disk by the agent (never pasted into the context):
  python3 fetch/xparse.py A1=agent-tools/a.txt A2=agent-tools/b.txt B1=... C1=... [--chars 200] [--out runs/<P>/x-raw.md]

Output: first line = number of posts returned per group (copy it to the top of x-leads.md), then one
line per post: [group] MM-DD HH:MM (local) x.com/<user>/status/<id> | first N chars of the text.
Posts are de-duplicated across groups by id; engagement numbers and metadata are dropped; posts that are
only a link or too short are skipped. A parse failure prints one line (no traceback).
When a result carries a next_token a "truncated" warning is printed (see fetch/x-recall.md, rule 2b).
Exit code: 0 (also when a file could not be parsed; that is reported in the output).
"""
import argparse
import glob
import json
import os
import re
import sys
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)
import i18n  # noqa: E402

FL = i18n.resolve_lang()


def tr(key, **kw):
    return i18n.ft(FL, key, **kw)


TZ_LOCAL = ZoneInfo(os.environ.get("DIGEST_TZ") or "UTC")


def _fmt_ts(ts) -> str:
    if ts is None or ts == "":
        return "?"
    try:
        if isinstance(ts, (int, float)) or (isinstance(ts, str) and ts.isdigit()):
            val = float(ts)
            if val > 1e11:
                val /= 1000.0
            return datetime.fromtimestamp(val, tz=timezone.utc).astimezone(TZ_LOCAL).strftime("%m-%d %H:%M")
        return datetime.fromisoformat(str(ts).replace("Z", "+00:00")).astimezone(TZ_LOCAL).strftime("%m-%d %H:%M")
    except Exception:
        return "?"


def load_json(path):
    fs = glob.glob(path) or glob.glob(path + "*")
    if not fs:
        return None
    with open(fs[0], encoding="utf-8", errors="replace") as fh:
        s = fh.read()
    for m in re.finditer(r'\{\s*"(?:data|meta|includes|results|news|errors|error|title)"', s):
        try:
            return json.JSONDecoder(strict=False).raw_decode(s[m.start():])[0]
        except Exception:
            continue
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("items", nargs="+", metavar="LABEL=PATH", help="group label and result file, e.g. A1=agent-tools/a.txt (several allowed)")
    ap.add_argument("--chars", type=int, default=200, help="characters of post text to keep (default 200)")
    ap.add_argument("--out", help="also write the output to this file")
    ap.add_argument("--lang", help="language of output labels: en | zh-CN (default: $DIGEST_LANG, else en)")
    a = ap.parse_args()
    global FL
    try:
        FL = i18n.resolve_lang(a.lang)
    except ValueError as e:
        ap.error(str(e))
    counts, lines, seen = [], [], set()
    trunc = []   # 有 next_token = 只拿到窗口里最新的一段（2026-11-18 review：B1/C1/C2 每次只回 25 条）
    seen_txt = set()
    for it in a.items:
        label, _, path = it.partition("=")
        if not path:
            label, path = f"F{len(counts)+1}", it
        d = load_json(path)
        if not isinstance(d, dict):
            counts.append(f"{label}={tr('xp_parse_fail')}")
            continue
        data = d.get("data") or d.get("results") or []
        if isinstance(data, dict):
            inner = next((v for v in data.values() if isinstance(v, list) and v and isinstance(v[0], dict)), None)
            data = inner if inner is not None else [data]
        if not data:   # 其他形状（如 search_news）：取第一个「字典列表」
            data = next((v for v in d.values() if isinstance(v, list) and v and isinstance(v[0], dict)), [])
        users = {u.get("id"): u.get("username") for u in (d.get("includes") or {}).get("users", [])}
        counts.append(f"{label}={len(data)}")
        nt = (d.get("meta") or {}).get("next_token")
        if nt:
            olds = sorted(str(p.get("created_at")) for p in data if isinstance(p, dict) and p.get("created_at"))
            ot = _fmt_ts(olds[0]) if olds else "?"
            trunc.append(f"{label}({tr('xp_oldest', t=ot)}, next_token={nt})")
        for p in data:
            if not isinstance(p, dict):
                continue
            pid = str(p.get("id") or p.get("rest_id") or p.get("post_id") or "")
            if pid and pid in seen:
                continue
            if pid:
                seen.add(pid)
            text = p.get("text") or p.get("title") or p.get("summary") or ""
            bare = re.sub(r"https?://\S+", "", text).strip()
            if len(bare) < 25 or bare.startswith("@") or bare.startswith("RT @"):   # 纯链接/过短/回复串/转推：跳过（条数仍计入）
                continue
            key = re.sub(r"\W+", "", bare.lower())[:70]   # 同一账号重复发同一段文字（如发了两遍的公告）只留第一条
            if key in seen_txt:
                continue
            seen_txt.add(key)
            ts = p.get("created_at") or p.get("last_updated_at_ms") or ""
            t = _fmt_ts(ts)
            url = (p.get("url") or "").replace("https://x.com/", "x.com/")
            if not url and pid:
                un = users.get(p.get("author_id")) or "i"
                url = f"x.com/{un}/status/{pid}"
            lines.append(f"[{label}] {t} {url} | " + re.sub(r"\s+", " ", bare)[: a.chars])
    warn = (tr("xp_trunc") + "; ".join(trunc)) if trunc else ""
    out = " ".join(counts) + "\n" + (warn + "\n" if warn else "") + "\n".join(lines)
    if a.out:
        out_dir = os.path.dirname(os.path.abspath(a.out))
        if out_dir:
            os.makedirs(out_dir, exist_ok=True)
        with open(a.out, "w", encoding="utf-8") as fh:
            fh.write(out + "\n")
        print(tr("xp_wrote", counts=" ".join(counts), out=a.out, n=len(lines), bytes=len(out.encode("utf-8"))))
        if warn:
            print(warn)
    else:
        print(out)


if __name__ == "__main__":
    main()
