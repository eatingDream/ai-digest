#!/usr/bin/env python3
"""Weekly helper: candidates that only got a signal later in the week but were never pushed
(script-generated; the weekly run reads this one compact output instead of re-reading the week's raw triage).

Examples:
  python3 fetch/weekly_late.py                                   # last full Monday-Sunday week (local time)
  python3 fetch/weekly_late.py --day 2026-01-15                  # the week containing that day
  python3 fetch/weekly_late.py --start 2026-01-12 --end 2026-01-18 --max 40

Data: every day's runs/<D>/fetch/candidates.jsonl and runs/<D>/fetch-extra/ (--only top-ups); the first
weekly run's gap-fill fetch runs/<S>-weekly/fetch/ is merged in too (windows under 2 days only).
No network, no triage files, no full ledger.json read (only fetch.py's ledger key matching).

Selection (always: not "pushed" in the ledger):
  papers: (1) late on the HF daily list  (2) later seen on other sources  (3) mentioned by this week's
          lab/news/blog candidates (arXiv id in text or links)  (4) same-day HF with preference weight >= 3
          and no ledger record at all. Seen-but-not-pushed papers are only listed for (1)-(3).
  non-papers: lab / policy / researcher-blog items with no ledger record, plus tool releases / newsletters /
          podcasts with preference weight >= 3 and no record.
  filter: HF late/same-day papers are listed only with weight >= 2 or a top institution (--all lists everything).
One line per item; fetch abstracts with `python3 fetch/cand.py --since <Monday> <arXiv id>`.
"""
import argparse, glob, importlib.util, json, os, re, sys
from datetime import datetime, timedelta

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("DIGEST_ROOT") or os.path.dirname(HERE)
spec = importlib.util.spec_from_file_location("fetchmod", os.path.join(HERE, "fetch.py"))
fx = importlib.util.module_from_spec(spec)
sys.modules["fetchmod"] = fx
spec.loader.exec_module(fx)


def tr(key, **kw):
    return fx.i18n.ft(fx.FL, key, **kw)


NONPAPER_CH = {"lab", "policy", "tooling", "research_blog", "newsletter", "podcast"}
CORE_CH = {"lab", "policy", "research_blog"}          # 这几类台账没记录的都列；其余（工具发布/Newsletter/播客）只列偏好权重 ≥3 的
# 头部实验室 / 研究机构（HF 机构字段命中 = 机构信号）
BIG_ORG = re.compile(r"google|deepmind|openai|anthropic|\bmeta\b|fair\b|microsoft|nvidia|apple|amazon|\baws\b|bytedance|\bseed\b|alibaba|qwen|tongyi|tencent|hunyuan|deepseek|moonshot|kimi|zhipu|z\.ai|minimax|xiaomi|baidu|huawei|mistral|allen|\bai2\b|stanford|\bmit\b|massachusetts institute|berkeley|carnegie|princeton|tsinghua|peking|oxford|cambridge|\beth\b|epfl|toronto|mila|inclusionai|ant group|shanghai ai|sakana|cohere|\bxai\b|\bibm\b|samsung|naver|kuaishou|kling|meituan|sensetime|stepfun|skywork|openbmb|longcat|internlm|thinking machines|together ai|hugging ?face|databricks|snowflake|salesforce|adobe|\bintel\b|\bamd\b|qualcomm|\btii\b|\bntu\b|nanyang|national university of singapore|kaist|tokyo|riken", re.I)


def week_days(a):
    if a.start and a.end:
        s = datetime.strptime(a.start, "%Y-%m-%d").date()
        e = datetime.strptime(a.end, "%Y-%m-%d").date()
    else:
        d0 = datetime.strptime(a.day, "%Y-%m-%d").date() if a.day else (datetime.now(fx.TZ_LOCAL).date() - timedelta(days=7))
        s = d0 - timedelta(days=d0.weekday())
        e = s + timedelta(days=6)
    out, d = [], s
    while d <= e:
        out.append(d.strftime("%Y-%m-%d"))
        d += timedelta(days=1)
    return out


def load_dir(p):
    f = os.path.join(p, "candidates.jsonl")
    if not os.path.exists(f):
        return []
    with open(f, encoding="utf-8") as fh:
        return [json.loads(l) for l in fh if l.strip()]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--day", metavar="YYYY-MM-DD", help="produce the week containing this day (default: last full week)")
    ap.add_argument("--start", metavar="YYYY-MM-DD", help="override the window start")
    ap.add_argument("--end", metavar="YYYY-MM-DD", help="override the window end")
    ap.add_argument("--chars", type=int, default=0, help="attach the first N chars of each abstract (default 0 = none)")
    ap.add_argument("--max", type=int, default=40, help="max papers listed")
    ap.add_argument("--max-np", type=int, default=15, help="max non-paper items per channel (by weight, then time)")
    ap.add_argument("--all", action="store_true", help="no filtering: list every HF-late paper (default: weight >=2 or a top institution only) and every non-paper item")
    ap.add_argument("--lang", help="en | zh-CN (default: $DIGEST_LANG, else en)")
    a = ap.parse_args()
    if a.lang:
        try:
            fx.FL = fx.i18n.resolve_lang(a.lang)
        except ValueError as e:
            ap.error(str(e))
    root = os.environ.get("DIGEST_ROOT") or ROOT
    days = week_days(a)
    M, S = days[0], days[-1]
    srcs, have, missing = [], [], []
    for d in days:
        got = False
        subs = ["fetch", "fetch-extra"] + sorted(os.path.basename(x) for x in glob.glob(os.path.join(root, "runs", d, "fetch-prev-*")))
        for sub in subs:   # fetch-prev-* = 同一天被重抓时挪走的旧抓取（过渡期 11/17 的旧日程那一次就在里面）
            p = os.path.join(root, "runs", d, sub)
            if os.path.exists(os.path.join(p, "candidates.jsonl")):
                srcs.append((d, p))
                got = True
        (have if got else missing).append(d)
    gap = os.path.join(root, "runs", f"{S}-weekly", "fetch")
    try:
        with open(os.path.join(gap, "report.json"), encoding="utf-8") as fh:
            meta = json.load(fh)
        span = datetime.fromisoformat(meta["window_end"]) - datetime.fromisoformat(meta["window_start"])
        if span < timedelta(days=2):
            srcs.append((meta["window_start"][:10], gap))
            have.append(f"{S}-weekly({tr('wl_gap', start=meta['window_start'][5:16], end=meta['window_end'][5:16])})")
    except Exception:
        pass
    ledger = fx.load_ledger_keys()

    def lstat(r):
        aid = (r.get("extra") or {}).get("arxiv_id")
        k = f"arxiv:{aid}" if aid else fx.canon_url(r["url"])
        return ledger.get(k) or ledger.get("t:" + fx.norm_title(r["title"])) or ""

    items, xref = {}, {}
    for day, p in srcs:
        for r in load_dir(p):
            ex = r.get("extra") or {}
            aid = ex.get("arxiv_id")
            if r["channel"] != "paper":
                for m in fx.ARXIV_ID_RE.finditer(f"{r['url']} {r['title']} {r.get('summary') or ''}"):
                    xref.setdefault(m.group(1), []).append((day, r["source"], r["title"][:40]))
                if r["channel"] in NONPAPER_CH:
                    k = fx.canon_url(r["url"])
                    items.setdefault(k, {"kind": "np", "title": r["title"], "url": r["url"], "source": r["source"], "channel": r["channel"],
                                         "day": day, "pref": r.get("pref", 0), "pub": (r.get("published") or "")[5:16].replace("T", " "), "led": lstat(r)})
                continue
            k = f"arxiv:{aid}" if aid else fx.canon_url(r["url"])
            it = items.setdefault(k, {"kind": "paper", "id": aid or r["url"], "title": r["title"], "first": day, "pref": 0, "hf_days": set(),
                                      "others": set(), "led": "", "org": "", "sid": r["id"][:6], "days": set(), "summary": ""})
            it["days"].add(day)
            it["first"] = min(it["first"], day)
            it["pref"] = max(it["pref"], r.get("pref", 0))
            if not it["summary"] and r.get("summary"):
                it["summary"] = r["summary"]
            srcset = [r["source"]] + list(r.get("also_in") or [])
            if "hf_daily" in srcset:
                it["hf_days"].add(day)
                it["org"] = it["org"] or ex.get("org") or ""
            it["others"].update(x for x in srcset if x not in ("arxiv", "hf_daily"))
            it["led"] = it["led"] or lstat(r)

    papers, dropped = [], [0]
    for k, it in items.items():
        if it["kind"] != "paper" or it["led"] == "pushed":
            continue
        sig = []
        late_hf = [d for d in sorted(it["hf_days"]) if d > it["first"]]
        big = bool(it["org"] and BIG_ORG.search(it["org"]))
        org_s = f"·{it['org'][:22]}" if it["org"] else ""
        if late_hf:
            if it["pref"] >= 2 or big or a.all:
                sig.append(tr("wl_late_hf", d=late_hf[0][5:]) + org_s + (tr("wl_big_org") if big else ""))
            else:
                dropped[0] += 1
        elif it["hf_days"]:
            if ((it["pref"] >= 3 or big) and not it["led"]) or a.all:
                sig.append(tr("wl_hf", d=sorted(it['hf_days'])[0][5:]) + org_s + tr("wl_no_ledger") + (tr("wl_big_org") if big else ""))
            else:
                dropped[0] += 1
        if it["others"]:
            sig.append(tr("sig_multi") + ",".join(sorted(it["others"])[:3]))
        refs = xref.get(it["id"]) if isinstance(it["id"], str) else None
        if refs:
            sig.append(tr("sig_mentioned") + "; ".join(f"{s}:{t}" for _, s, t in refs[:2]))
        if it["led"] == "seen" and not (late_hf or it["others"] or refs):
            continue
        if sig:
            it["sig"] = sig
            papers.append(it)
    papers.sort(key=lambda it: (not any(s.startswith(tr("wl_late_hf", d="")) for s in it["sig"]), -it["pref"], it["first"], it["title"]))
    nonp_all = [it for it in items.values() if it["kind"] == "np" and not it["led"]]
    nonp = sorted([it for it in nonp_all if a.all or it["channel"] in CORE_CH or it["pref"] >= 3], key=lambda it: (it["channel"], it["day"], it["title"]))

    print(tr("wl_title", m=M, s=S))
    print(tr("wl_sources") + ", ".join(have) + (tr("wl_missing") + ", ".join(missing) if missing else ""))
    print(tr("wl_hint", m=M) + "\n")
    print(tr("wl_papers", n=len(papers)))
    for it in papers[: a.max]:
        star = "☆ " if it["led"] == "seen" else ""
        print(f"- {it['id']} {star}{it['title'][:100]} {tr('wl_weight', n=it['pref'])} {tr('wl_first', d=it['first'][5:])}〔{'; '.join(it['sig'])}〕")
        if a.chars > 0 and it.get("summary"):
            print("  - " + re.sub(r"\s+", " ", it["summary"])[: a.chars])
    if len(papers) > a.max:
        print(tr("wl_more", n=len(papers) - a.max))
    if dropped[0]:
        print(tr("wl_dropped", n=dropped[0]))
    print("\n" + tr("wl_nonpaper", n=len(nonp), total=len(nonp_all)))
    cur, cnt, cut = None, 0, 0
    for it in nonp:
        if it["channel"] != cur:
            cur, cnt = it["channel"], 0
            print(f"### {cur}")
        cnt += 1
        if cnt > a.max_np:
            cut += 1
            continue
        print(f"- {it['pub'] or it['day'][5:]} {it['source']} | {it['title'][:90]} | {it['url']}")
    if cut:
        print(tr("wl_more_np", n=cut))


if __name__ == "__main__":
    main()
