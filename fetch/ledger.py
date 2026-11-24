#!/usr/bin/env python3
"""Small ledger.json tool: targeted lookups and one-shot batch writes (never read the whole file into the context).

  python3 fetch/ledger.py find <keyword|arxiv-id|URL fragment> [...]          # one line per hit
  python3 fetch/ledger.py recent [--days 21] [--status pushed|seen|all] [--cat paper]   # one line per entry (name cut at 70 chars)
  python3 fetch/ledger.py add --day 2026-01-15 <<'EOF'                         # record the whole issue in one write
  P|name|id|category|url1 url2|one-line reason for pushing
  S|name|id|category|url1|why it was seen but not pushed
  EOF

  --log runs/<P>/filter-log.md  also appends those rows to the filter log as the "Pushed" and
  "Seen, not pushed (for the weekly to re-check)" sections (the note column IS the reason, so write it
  only here; add source status separately with cat >>).
  --log-heading "Addendum (...)" first writes a `## Addendum (...)` line and demotes the two sections to `###`.
  --json (find / recent / add) prints one JSON object on stdout instead of text lines.

P = pushed (in this issue), S = seen-not-pushed. An empty id becomes a slug of the name; an existing entry with
the same id or URL gets its status updated instead of a duplicate. The first write of a day backs the ledger up
to logs/ledger-backup/ledger-<day>.json. Ledger path: $LEDGER_PATH, else $DIGEST_ROOT/ledger.json, else <repo>/ledger.json.
When ledger.json does not exist yet, read commands (find / recent) return empty results (exit 0) and add creates it automatically.
Exit codes: 0 ok; 2 ledger.json exists but is corrupt or unreadable.
"""
import argparse
import json
import os
import re
import shutil
import sys
from datetime import datetime, timedelta
from zoneinfo import ZoneInfo

CODE_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if CODE_ROOT not in sys.path:
    sys.path.insert(0, CODE_ROOT)
import i18n  # noqa: E402

FL = i18n.resolve_lang()


def tr(key, **kw):
    return i18n.ft(FL, key, **kw)


TZ_LOCAL = ZoneInfo(os.environ.get("DIGEST_TZ") or "UTC")
ROOT = os.environ.get("DIGEST_ROOT", "").strip() or CODE_ROOT
P = os.environ.get("LEDGER_PATH", "").strip() or os.path.join(ROOT, "ledger.json")


def load():
    if not os.path.exists(P):
        return {"items": []}
    try:
        with open(P, encoding="utf-8") as fh:
            d = json.load(fh)
        if not isinstance(d, dict) or not isinstance(d.get("items"), list):
            raise ValueError("expected JSON object with 'items' list")
        return d
    except (OSError, ValueError) as e:
        print(f"error: cannot read ledger {P}: {e}", file=sys.stderr)
        sys.exit(2)


def st(it):
    s = f"{it.get('status') or ''} {it.get('category') or ''}"
    return "seen" if "seen" in s else "pushed"


def line(it):
    pushed = it.get("pushed") or it.get("last_pushed") or ""
    return f"{it.get('first_seen','?')} | {st(it):6} | {it.get('id','')} | {(it.get('name') or '')[:70]}" + (f" | pushed {pushed}" if pushed else "")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lang", help="language of output labels: en | zh-CN (default: $DIGEST_LANG, else en)")
    sub = ap.add_subparsers(dest="cmd", required=True, metavar="{find,recent,add}")
    f = sub.add_parser("find", help="look entries up by keyword / arXiv id / URL fragment")
    f.add_argument("keys", nargs="+", metavar="KEY")
    f.add_argument("--json", action="store_true", help="print JSON")
    r = sub.add_parser("recent", help="list entries first seen in the last N days")
    r.add_argument("--days", type=int, default=21, help="look-back in days (default 21)")
    r.add_argument("--status", default="all", choices=["all", "pushed", "seen"], help="filter by status (default all)")
    r.add_argument("--cat", help="filter by category, e.g. paper")
    r.add_argument("--json", action="store_true", help="print JSON")
    a_ = sub.add_parser("add", help="record entries read from stdin (P|... pushed, S|... seen)")
    a_.add_argument("--day", required=True, metavar="YYYY-MM-DD", help="issue day the rows belong to")
    a_.add_argument("--log", metavar="FILE", help="also append the rows to this filter-log file")
    a_.add_argument("--log-heading", help="heading for an addendum block in --log")
    a_.add_argument("--json", action="store_true", help="print JSON")
    a = ap.parse_args()
    global FL
    try:
        FL = i18n.resolve_lang(a.lang)
    except ValueError as e:
        ap.error(str(e))
    d = load()
    items = d["items"]
    if a.cmd == "find":
        out = {}
        for k in a.keys:
            kk = k.lower()
            hits = [it for it in items if kk in json.dumps([it.get("name"), it.get("id"), it.get("urls")], ensure_ascii=False).lower()]
            out[k] = hits
            if not a.json:
                print(tr("lg_hits", k=k, n=len(hits)))
                for it in hits[:8]:
                    print("  " + line(it))
        if a.json:
            print(json.dumps({"query": out}, ensure_ascii=False))
    elif a.cmd == "recent":
        cut = (datetime.now(TZ_LOCAL) - timedelta(days=a.days)).strftime("%Y-%m-%d")
        rows = [it for it in items if (it.get("first_seen") or it.get("day") or "") >= cut and (a.status == "all" or st(it) == a.status) and (not a.cat or it.get("category") == a.cat)]
        rows.sort(key=lambda it: it.get("first_seen") or "")
        if a.json:
            print(json.dumps({"days": a.days, "status": a.status, "count": len(rows), "items": rows}, ensure_ascii=False))
        else:
            print(tr("lg_recent", days=a.days, n=len(rows), status=a.status))
            for it in rows:
                print(line(it))
    else:
        p_abs = os.path.abspath(P)
        os.makedirs(os.path.dirname(p_abs), exist_ok=True)
        backup_root = os.path.dirname(p_abs) if os.environ.get("LEDGER_PATH", "").strip() else ROOT
        bdir = os.path.join(backup_root, "logs", "ledger-backup")
        os.makedirs(bdir, exist_ok=True)
        bp = os.path.join(bdir, f"ledger-{a.day}.json")
        if not os.path.exists(bp):
            if os.path.exists(P):
                shutil.copy(P, bp)
            else:
                with open(bp, "w", encoding="utf-8") as bfh:
                    json.dump({"items": []}, bfh, ensure_ascii=False, indent=1)
        added = upd = 0
        logP, logS = [], []
        for ln in sys.stdin.read().splitlines():
            ln = ln.strip()
            if not ln or ln.startswith("#"):
                continue
            p = [x.strip() for x in ln.split("|")]
            p += [""] * (6 - len(p))
            kind, name, iid, cat, urls, note = p[:6]
            (logP if kind.upper() == "P" else logS).append(f"- {name}{tr('lg_colon')}{note}" if note else f"- {name}")
            urls = urls.split()
            iid = iid or re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")[:40]
            hit = next((it for it in items if it.get("id") == iid or (urls and set(urls) & set(it.get("urls") or []))), None)
            status = "pushed" if kind.upper() == "P" else "seen-not-pushed"
            if hit:
                if kind.upper() == "P":
                    hit["status"] = "pushed"
                    hit["pushed"] = a.day
                elif st(hit) == "pushed":
                    pass   # 已推过的不降级
                else:
                    hit["status"] = status
                if urls:
                    existing_urls = list(hit.get("urls") or [])
                    for u in urls:
                        if u not in existing_urls:
                            existing_urls.append(u)
                    hit["urls"] = existing_urls
                if note:
                    hit["notes"] = note
                upd += 1
            else:
                it = {"day": a.day, "name": name, "id": iid, "urls": urls, "first_seen": a.day, "category": cat or "other", "status": status}
                if kind.upper() == "P":
                    it["pushed"] = a.day
                if note:
                    it["notes"] = note
                items.append(it)
                added += 1
        d["item_count"] = len(items)
        d["previous_digest_at"] = datetime.now(TZ_LOCAL).isoformat(timespec="minutes")
        tmp = P + ".tmp"
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump(d, fh, ensure_ascii=False, indent=1)
        os.replace(tmp, P)
        if a.json:
            print(json.dumps({"added": added, "updated": upd, "total": len(items)}))
        else:
            print(f"ledger: +{added} new, {upd} updated, total {len(items)}")
        if a.log:
            h = "###" if a.log_heading else "##"
            out = ["", f"## {a.log_heading}", ""] if a.log_heading else [""]
            if logP:
                out += [f"{h} {tr('lg_pushed')}"] + logP + [""]
            if logS:
                out += [f"{h} {tr('lg_seen')}"] + logS + [""]
            os.makedirs(os.path.dirname(os.path.abspath(a.log)), exist_ok=True)
            with open(a.log, "a", encoding="utf-8") as fh:
                fh.write("\n".join(out))
            print(f"filter-log: +{len(logP)} {tr('lg_pushed')}, +{len(logS)} {tr('lg_seen')} → {a.log}", file=sys.stderr if a.json else sys.stdout)


if __name__ == "__main__":
    main()
