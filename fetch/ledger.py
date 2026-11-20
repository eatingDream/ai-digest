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
to logs/ledger-backup/ledger-<day>.json. Ledger path: $LEDGER_PATH, else <repo>/ledger.json.
Exit codes: 0 ok; 2 ledger.json missing or unreadable (the message says where it looked).
"""
import argparse, json, os, re, sys, shutil
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.dirname(_os.path.abspath(__file__))))
import i18n
FL = i18n.resolve_lang()
def tr(key, **kw):
    return i18n.ft(FL, key, **kw)
from datetime import datetime
from zoneinfo import ZoneInfo

TZ_LOCAL = ZoneInfo(os.environ.get("DIGEST_TZ") or "UTC")
from datetime import datetime, timedelta
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
P = os.environ.get("LEDGER_PATH") or os.path.join(ROOT, "ledger.json")


def load():
    try:
        return json.load(open(P))
    except (OSError, ValueError) as e:
        print(f"error: cannot read ledger {P}: {e} (set LEDGER_PATH or create it with: echo '{{\"items\": []}}' > ledger.json)", file=sys.stderr)
        sys.exit(2)


def st(it):
    s = f"{it.get('status') or ''} {it.get('category') or ''}"
    return "seen" if "seen" in s else "pushed"


def line(it):
    pushed = it.get("pushed") or it.get("last_pushed") or ""
    return f"{it.get('first_seen','?')} | {st(it):6} | {it.get('id','')} | {(it.get('name') or '')[:70]}" + (f" | pushed {pushed}" if pushed else "")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
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
        cut = (datetime.now() - timedelta(days=a.days)).strftime("%Y-%m-%d")
        rows = [it for it in items if (it.get("first_seen") or it.get("day") or "") >= cut and (a.status == "all" or st(it) == a.status) and (not a.cat or it.get("category") == a.cat)]
        rows.sort(key=lambda it: it.get("first_seen") or "")
        if a.json:
            print(json.dumps({"days": a.days, "status": a.status, "count": len(rows), "items": rows}, ensure_ascii=False))
        else:
            print(tr("lg_recent", days=a.days, n=len(rows), status=a.status))
            for it in rows:
                print(line(it))
    else:
        bdir = os.path.join(ROOT, "logs", "ledger-backup"); os.makedirs(bdir, exist_ok=True)
        bp = os.path.join(bdir, f"ledger-{a.day}.json")
        if not os.path.exists(bp):
            shutil.copy(P, bp)
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
                    hit["status"] = "pushed"; hit["pushed"] = a.day
                elif st(hit) == "pushed":
                    pass   # 已推过的不降级
                else:
                    hit["status"] = status
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
        json.dump(d, open(tmp, "w"), ensure_ascii=False, indent=1)
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
            open(a.log, "a").write("\n".join(out))
            print(f"filter-log: +{len(logP)} {tr('lg_pushed')}, +{len(logS)} {tr('lg_seen')} → {a.log}", file=sys.stderr if a.json else sys.stdout)


if __name__ == "__main__":
    main()
