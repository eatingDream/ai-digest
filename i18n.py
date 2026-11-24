"""Tiny i18n layer shared by render_html.py, publish.py and fetch/.

The output language of a digest site is a single setting:

    DIGEST_LANG = en | zh-CN        (default: en)

Resolution order (see :func:`resolve_lang`): CLI flag > ``DIGEST_LANG`` env var >
``LANG`` key in ``site/CONFIG`` > default. Everything user-visible that the code
generates (page chrome, dates, labels, navigation, the password page, triage
headings) comes from the ``STRINGS`` table below. Digest *content* is written by
the agent from the prompts in ``prompts/<lang>/``.
"""
from __future__ import annotations

import os
import re
from datetime import date

DEFAULT_LANG = "en"
LANGS = ("en", "zh-CN")

_ALIASES = {
    "en": "en", "en-us": "en", "en-gb": "en", "english": "en",
    "zh-cn": "zh-CN", "zh_cn": "zh-CN", "zh": "zh-CN", "zh-hans": "zh-CN", "cn": "zh-CN",
    "chinese": "zh-CN", "中文": "zh-CN", "简体中文": "zh-CN",
}


def normalize_lang(value: str | None, *, strict: bool = True) -> str:
    """Return ``en`` or ``zh-CN`` for a user-supplied value (empty -> default)."""
    v = (value or "").strip()
    if not v:
        return DEFAULT_LANG
    got = _ALIASES.get(v.lower().replace(" ", ""))
    if got:
        return got
    if strict:
        raise ValueError(f"unsupported language {value!r}; use one of: {', '.join(LANGS)}")
    return DEFAULT_LANG


def resolve_lang(cli: str | None = None, config: str | None = None) -> str:
    """CLI flag > DIGEST_LANG env > CONFIG ``LANG`` > default."""
    for cand in (cli, os.environ.get("DIGEST_LANG"), config):
        if cand and cand.strip():
            return normalize_lang(cand)
    return DEFAULT_LANG


# --------------------------------------------------------------------------- strings
STRINGS: dict[str, dict[str, str]] = {
    "en": {
        "html_lang": "en",
        "brand_tail": " Digest",          # nameplate = <ai>AI</ai> + tail
        "brand_plain": "AI Digest",
        "motto": "Models · Research · Engineering · Industry",
        # masthead / chrome
        "skip": "Skip to content",
        "appearance": "Appearance",
        "aria_lang_switch": "Language",
        "lang_short": "English",
        "theme_system": "System", "theme_light": "Light", "theme_dark": "Dark",
        "kicker_daily": "Issue", "kicker_weekly": "Week", "kicker_other": "Issue",
        "edition_daily": "Daily", "edition_weekly": "Weekly",
        "daily_title": "AI Daily", "weekly_title": "AI Weekly",
        "toc": "Contents", "toc_open": "Open contents", "toc_close": "Close contents",
        "toc_open_btn": "☰ Contents",
        "issue_sections": "In this issue",
        "items_one": "{n} item", "items_many": "{n} items",
        "must": "Must read this week", "themes": "Themes of the week",
        "front_aria": "Front page",
        "end": "End of issue",
        "to_top": "Back to top",
        "new": "New",
        "label_look": "Why it matters", "label_detail": "Details", "label_followup": "Follow-up",
        "related": "Related: ",
        "sec_default_papers": "Papers", "sec_default_items": "Items",
        # publish.py: navigation + index
        "nav_aria": "Issue navigation",
        "nav_index": "Back to index", "nav_prev": "Previous", "nav_next": "Next",
        "nav_week": "This week's recap",
        "lang_name": "English", "lock": "Lock", "lock_title": "Forget the key saved on this device",
        "index_title": "AI Digest · Archive",
        "index_chip": "Archive", "index_chip_unit": "issues",
        "index_meta": "{d} daily · {w} weekly",
        "index_daily": "Daily · by week", "index_weekly": "Weekly · by month",
        "issues_n": "{n} issues", "issues_one": "{n} issue", "issue_unit_one": "issue", "issue_unit_many": "issues",
        "week_n": "Week {n}",
        "week_link": "This week's recap →",
        "no_daily_week": "No daily issues this week",
        "none_yet": "Nothing yet",
        "month_label": "{month} {y}",
        "index_end": "AI Digest",
        # client-side filter bar (issue + archive)
        "filter_label": "Filter items",
        "filter_placeholder": "Search title, summary, source, or tag…",
        "archive_filter_label": "Filter archive",
        "archive_filter_placeholder": "Search issues, headlines, or tags…",
        "filter_all": "All",
        "filter_clear": "Clear",
        "filter_empty": "No matching items in this issue.",
        "archive_filter_empty": "No matching issues found.",
        "filter_shown": "{shown} / {total}",
        # password page
        "pw_title": "AI Digest", "pw_label": "Password", "pw_remember": "Remember this device",
        "pw_open": "Open", "pw_wrong": "Wrong password",
        "pw_unsupported": "This browser can't open this page", "pw_noscript": "JavaScript is required",
        "pw_demo_hint": "Demo site · password: {pw}",
    },
    "zh-CN": {
        "html_lang": "zh-CN",
        "brand_tail": "精选",
        "brand_plain": "AI精选",
        "motto": "模型 · 研究 · 工程 · 产业",
        "skip": "跳到正文",
        "appearance": "外观",
        "aria_lang_switch": "语言",
        "lang_short": "中文",
        "theme_system": "系统", "theme_light": "浅色", "theme_dark": "深色",
        "kicker_daily": "本期", "kicker_weekly": "本期", "kicker_other": "本期",
        "edition_daily": "日报", "edition_weekly": "周报",
        "daily_title": "AI精选日报", "weekly_title": "AI精选周报",
        "toc": "目录", "toc_open": "打开目录", "toc_close": "关闭目录",
        "toc_open_btn": "☰ 目录",
        "issue_sections": "本期版面",
        "items_one": "{n} 条", "items_many": "{n} 条",
        "must": "本周看点", "themes": "本周主线",
        "front_aria": "头版",
        "end": "（本期完）",
        "to_top": "回到顶部",
        "new": "新",
        "label_look": "值得看", "label_detail": "详情", "label_followup": "本周后续",
        "related": "相关：",
        "sec_default_papers": "论文", "sec_default_items": "条目",
        "nav_aria": "期次导航",
        "nav_index": "返回目录", "nav_prev": "上一期", "nav_next": "下一期",
        "nav_week": "本周周报",
        "lang_name": "简体中文", "lock": "锁定", "lock_title": "清除本设备保存的密钥",
        "index_title": "AI精选 · 目录",
        "index_chip": "目录", "index_chip_unit": "期",
        "index_meta": "日报 {d} · 周报 {w}",
        "index_daily": "日报 · 按周", "index_weekly": "周报 · 按月",
        "issues_n": "{n} 期", "issues_one": "{n} 期", "issue_unit_one": "期", "issue_unit_many": "期",
        "week_n": "第 {n} 周",
        "week_link": "本周周报 →",
        "no_daily_week": "本周暂无日报",
        "none_yet": "暂无",
        "month_label": "{y}年{m}月",
        "index_end": "AI 精选",
        "filter_label": "筛选条目",
        "filter_placeholder": "搜索标题、摘要、来源或标签…",
        "archive_filter_label": "筛选归档",
        "archive_filter_placeholder": "搜索期次、标题或标签…",
        "filter_all": "全部",
        "filter_clear": "清除",
        "filter_empty": "本期没有匹配的条目。",
        "archive_filter_empty": "没有找到匹配的期次。",
        "filter_shown": "{shown} / {total}",
        "pw_title": "AI精选", "pw_label": "密码", "pw_remember": "记住此设备",
        "pw_open": "打开", "pw_wrong": "密码不对",
        "pw_unsupported": "此浏览器无法打开", "pw_noscript": "需要启用 JavaScript",
        "pw_demo_hint": "演示站 · 密码：{pw}",
    },
}

WEEKDAYS = {
    "en": ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    "zh-CN": ["星期一", "星期二", "星期三", "星期四", "星期五", "星期六", "星期日"],
}
WEEKDAYS_SHORT = {
    "en": ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    "zh-CN": ["周一", "周二", "周三", "周四", "周五", "周六", "周日"],
}
MONTHS = ["January", "February", "March", "April", "May", "June", "July",
          "August", "September", "October", "November", "December"]


def t(lang: str, key: str, **kw) -> str:
    s = STRINGS[normalize_lang(lang)][key]
    return s.format(**kw) if kw else s


def count_items(lang: str, n: int) -> str:
    return t(lang, "items_one" if n == 1 else "items_many", n=n)


def count_issues(lang: str, n: int) -> str:
    return t(lang, "issues_one" if n == 1 else "issues_n", n=n)


def issue_unit(lang: str, n: int) -> str:
    return t(lang, "issue_unit_one" if n == 1 else "issue_unit_many")


def weekday(lang: str, d: date) -> str:
    return WEEKDAYS[normalize_lang(lang)][d.weekday()]


def weekday_short(lang: str, d: date) -> str:
    return WEEKDAYS_SHORT[normalize_lang(lang)][d.weekday()]


def date_long(lang: str, d: date) -> str:
    """'January 15, 2026' / '2026 年 1 月 15 日'."""
    if normalize_lang(lang) == "zh-CN":
        return f"{d.year} 年 {d.month} 月 {d.day} 日"
    return f"{MONTHS[d.month - 1]} {d.day}, {d.year}"


def date_range(lang: str, a: date, b: date) -> str:
    if normalize_lang(lang) == "zh-CN":
        if a.year == b.year:
            return f"{a.year} 年 {a.month} 月 {a.day} 日 — {b.month} 月 {b.day} 日"
        return f"{a.year} 年 {a.month} 月 {a.day} 日 — {b.year} 年 {b.month} 月 {b.day} 日"
    if a.year == b.year:
        return f"{MONTHS[a.month - 1][:3]} {a.day} – {MONTHS[b.month - 1][:3]} {b.day}, {b.year}"
    return f"{MONTHS[a.month - 1][:3]} {a.day}, {a.year} – {MONTHS[b.month - 1][:3]} {b.day}, {b.year}"


def month_label(lang: str, y: int, m: int) -> str:
    if normalize_lang(lang) == "zh-CN":
        return t(lang, "month_label", y=y, m=m)
    return t(lang, "month_label", y=y, month=MONTHS[m - 1])


def year_month(lang: str, d: date) -> str:
    return month_label(lang, d.year, d.month)


def year_week(lang: str, iso_year: int, week: int) -> str:
    if normalize_lang(lang) == "zh-CN":
        return f"{iso_year} 年第 {week} 周"
    return f"{iso_year} · Week {week}"


# ------------------------------------------------- markdown vocabulary (both languages)
# Digest markdown may use either language's labels; the parser accepts all of them and
# maps them to canonical keys, so a Chinese and an English issue render identically.
FIELD_LABELS = {
    "dek": ("一句话", "In brief"),
    "look": ("值得看", "Why it matters"),
    "detail": ("详情", "Details"),
    "followup": ("本周后续", "Follow-up"),
}
FIELD_ALIASES = {label: key for key, labels in FIELD_LABELS.items() for label in labels}

FRONT_KEYS = {
    "must": ("本周必看", "🔥本周必看", "Must read this week", "🔥Must read this week", "Must read"),
    "themes": ("本周主线", "🧭本周主线", "Themes of the week", "🧭Themes of the week", "Themes"),
    "toc": ("目录", "📋目录", "Contents", "📋Contents"),
}
FRONT_ALIASES = {label: key for key, labels in FRONT_KEYS.items() for label in labels}

DROPPED_MARKERS = {"**本周剔除**", "🗑本周剔除", "## 本周剔除",
                   "**Dropped this week**", "🗑Dropped this week", "## Dropped this week"}

RELATED_WORDS = ("相关", "Related")
WEEKLY_TITLE_WORDS = ("周报", "weekly")

BADGE_ACCENT_TAGS = {"前沿发布", "开源权重", "一手", "重大", "RSI",
                     "Frontier release", "Open weights", "Primary source", "Major"}

# Section title -> stable anchor. Order matters: more specific words first. Matching is
# case-insensitive substring on the title with leading emoji removed.
SECTION_IDS = [
    (("论文", "paper"), "sec-papers"),
    (("工具", "项目", "开源", "tools", "project", "open source"), "sec-tools"),
    (("算力", "基础设施", "芯片", "硬件", "compute", "infrastructure", "chips", "hardware"), "sec-infra"),
    (("政策", "安全", "治理", "监管", "policy", "safety", "governance", "regulation"), "sec-policy"),
    (("行业", "产品", "商业", "融资", "公司", "industry", "product", "business", "funding"), "sec-industry"),
    (("观点", "长文", "访谈", "播客", "讨论", "opinion", "essay", "interview", "podcast", "voices"), "sec-voices"),
    (("评测", "基准", "数据", "eval", "benchmark"), "sec-evals"),
    (("模型", "model"), "sec-models"),
]


def looks_weekly(title: str) -> bool:
    low = title.lower()
    return any(w in low for w in WEEKLY_TITLE_WORDS)


# ------------------------------------------------------------------ fetch/ output labels
# Labels written into triage*.md, report.md and the CLI helpers under fetch/. The agent
# prompts in prompts/<lang>/ refer to exactly these headings, so keep both in sync.
FETCH: dict[str, dict[str, str]] = {
    "en": {
        "sec_lab": "Lab official", "sec_model": "Models / datasets / Spaces", "sec_tooling": "Tool & framework releases",
        "sec_repo": "New GitHub projects", "sec_research_blog": "Researcher blogs / essays", "sec_newsletter": "Newsletters",
        "sec_podcast": "Podcasts / video", "sec_chips_infra": "Chips / compute / infrastructure",
        "sec_policy": "Policy / safety & governance", "sec_news": "Tech news", "sec_news_zh": "Chinese-language media",
        "sec_community": "Community discussion",
        "big_tech_report": "technical report", "big_frontier": "frontier model name", "big_open": "open weights / model family",
        "big_scale": "scale", "big_bench": "hard benchmark / math", "big_safety": "safety / adversarial",
        "sig_multi": "multi-source:", "sig_mentioned": "mentioned:", "sig_ledger": "ledger", "sig_kw": "keyword:",
        "head_a": "Note: [n] is the ranking weight of the preferred topics, not an inclusion bar; ★ = already pushed (in the ledger), "
                  "☆ = seen but not pushed; (+x) = also seen in other sources; ? = date unknown, open the page to check. "
                  "Popularity numbers are never listed",
        "head_b": "A trailing #xxxxxx is a short id; use `python3 fetch/cand.py <id> …` (several at once) to get URL / summary / authors.",
        "papers_intro_tail": ". Each line = arXiv id + title (·HF = also on HF Daily); fetch abstracts with `cand.py <arxiv id> …`.",
        "papers_group_note": "Grouping: by preference weight first, then topic; each paper appears once. Off-topic weight-0 papers: those with signals "
                             "are listed by title only, the rest are folded (see the note at the end); off-topic major results still count.",
        "title_nonpaper": "# Candidates (non-paper) {start} → {end}", "title_papers": "# Candidate papers {start} → {end} ({n} papers)",
        "pushed_line": "★ Already pushed (same items; do not rewrite without real new progress): ",
        "rep_line": "↺ Trending/search discoveries already listed in recent triage (titles only, {n}): ",
        "group_pref3": "Preferred topics (weight ≥3)", "group_pref12": "Related (weight 1–2)", "weight_n": "Weight {n}",
        "other": "Other", "rest_plain": "Rest ({n}, by topic, titles only)",
        "rest_full": "# All titles in the 'rest' tier ({n} papers; {k} with signals are listed in triage-papers.md, {f} folded)",
        "rest_keep": "Rest · with signals ({n}, titles only; signal = HF / multi-source / mentioned elsewhere / ledger / big-result keyword)",
        "rest_fold": "Rest · folded ({n} papers, off-topic and no HF / multi-source / mention / big-result signal)",
        "fold_note": "All titles are in `{path}` (grouped by topic, one per line). The folded tier does not need a full read; if you suspect a missed "
                     "big result, run `rg -i -n 'kw1|kw2' {path}` (e.g. `'frontier|technical report|open-weight|state-of-the-art'`).",
        "report_title": "# Fetch report {start} → {end}",
        "report_line": "- Took {sec}s; {raw} raw items, {dd} after dedup",
        "note_prefix": "- Note: ",
        "report_header": "| Channel | Source | Items | Time | Status |",
        "arxiv_rolled": "arxiv: batch {label} has rolled out of /pastweek (announced {days} days ago) and cannot be retrieved now; backfill via the export API / old state",
        "arxiv_missing": "arxiv: ⚠️ weekday batch {label} (announced {when} local) is missing from both /pastweek and /new; retried once — "
                         "this is a real fetch failure: check whether arXiv skipped or delayed the batch; rerun with --only arxiv",
        "arxiv_missing_prev": "arxiv: ⚠️ these weekday batches in the window are missing: ",
        "arxiv_from_new": "arxiv: batch {label} taken from /new (/pastweek not refreshed yet)",
        "arxiv_pub": "published {when}",
        "hf_utc": "hf_daily: UTC boards ", "hf_none": "hf_daily: no complete board available to fetch",
        "hf_not_open": "hf_daily: {d} not open yet (HTTP 400), skipped",
        "fetch_lock_wait": "wait: another fetch.py is writing {outdir}, waiting for it to finish...",
        "fetch_lock_timeout": "another fetch.py is still writing {outdir} after 15 minutes; re-run the same command later",
        "fetch_reuse": "reuse: {outdir} already fetched this window at {at}, skipping network (drop --reuse to re-fetch)",
        # weekly_late.py
        "wl_title": "# Weekly: candidates with late signals that were never pushed {m} → {s}",
        "wl_sources": "Data sources (existing fetches): ", "wl_missing": "; missing: ",
        "wl_gap": "gap-fill {start}→{end}",
        "wl_hint": "Abstracts/authors: python3 fetch/cand.py --since {m} <arXiv id|short id>; field meanings: see the script header.",
        "wl_papers": "## Papers: showed up later on HF / multiple sources / mentioned, and never pushed ({n}; ☆ = seen in a daily but not pushed)",
        "wl_weight": "[weight {n}]", "wl_first": "first seen {d}", "wl_late_hf": "HF late {d}", "wl_hf": "HF {d}",
        "wl_big_org": "·top lab/institution", "wl_no_ledger": "·no ledger record",
        "wl_more": "  …{n} more not listed (raise --max)",
        "wl_dropped": "  ({n} off-topic papers that were only HF-late/same-day with weight <2 or a non-top institution were skipped; `--all` lists them all)",
        "wl_nonpaper": "## Lab / policy / researcher blogs (all) + tool releases / newsletters / podcasts (weight ≥3): no ledger record ({n}; {total} of the same kinds in total)",
        "wl_more_np": "  …{n} more not listed (raise `--max-np`, or `--all`)",
        # cand.py
        "c_pushed": "★ pushed", "c_seen": "☆ seen, not pushed", "c_date_unknown": "date unknown", "c_discovery": "discovery only",
        "c_authors": "   authors: ", "c_et_al": " et al.", "c_nomatch": "no match", "c_more": "   …{n} more matches; use a more specific keyword",
        # check_cands.py
        "cc_strong": "strong", "cc_weak": "maybe", "cc_pushed": "★pushed", "cc_unknown": "unknown ids", "cc_dup": "duplicates",
        "cc_bad_unknown": "unknown ids (not in today's candidates; check for typos): ", "cc_bad_dup": "duplicate ids: ",
        "cc_bad_count": "{n} entries is outside 10–60 (target 30–40; err on the wide side)",
        # xparse.py
        "xp_trunc": "⚠️ truncated (only the newest slice of the window was returned; earlier posts were not fetched): ",
        "xp_parse_fail": "parse failed", "xp_oldest": "oldest {t}",
        "xp_wrote": "{counts}  → {out} ({n} items, {bytes} bytes)",
        # ledger.py
        "lg_pushed": "Pushed", "lg_seen": "Seen, not pushed (for the weekly to re-check)", "lg_colon": ": ",
        "lg_hits": "## {k}: {n} match(es)", "lg_recent": "# last {days} days: {n} entries (status={status})",
        "pg_date": "Date: ", "pg_desc": "Description: ", "pg_fallback": "use WebFetch instead",
        "pg_short": "(body is only {n} characters, probably JS-rendered; use WebFetch instead)",
        "pg_hits": "--- {kw}: {n}{plus} hit(s)", "pg_trunc": "… (full text {n} characters, truncated; use --chars or --find)",
        "gh_rate_unauth": "github: ⚠️ HTTP {code} rate limit hit; set GITHUB_TOKEN or GH_TOKEN to raise the GitHub API limit from 60 to 5,000 req/hr",
        "gh_rate_auth": "github: ⚠️ HTTP {code} rate limit hit even with token; check GITHUB_TOKEN / GH_TOKEN quota or secondary rate limits",
    },
    "zh-CN": {
        "sec_lab": "实验室官方", "sec_model": "模型 / 数据集 / Spaces", "sec_tooling": "工具与框架发布",
        "sec_repo": "GitHub 新项目", "sec_research_blog": "研究者博客 / 长文", "sec_newsletter": "Newsletter",
        "sec_podcast": "播客 / 视频", "sec_chips_infra": "芯片 / 算力 / 基础设施",
        "sec_policy": "政策 / 安全治理", "sec_news": "科技新闻", "sec_news_zh": "中文媒体",
        "sec_community": "社区讨论",
        "big_tech_report": "技术报告", "big_frontier": "前沿模型名", "big_open": "开源权重/模型族",
        "big_scale": "规模", "big_bench": "硬基准/数学", "big_safety": "安全对抗",
        "sig_multi": "多源:", "sig_mentioned": "被提及:", "sig_ledger": "台账", "sig_kw": "词:",
        "head_a": "说明：[n] 是 用户偏好主题的排序权重，不是纳入门槛；★ = ledger 已推过，☆ = 看过未推；(+x) = 其他信源也出现；? = 日期未知需打开核对；热度数字一律不列",
        "head_b": "行尾 #xxxxxx 是短 id，用 `python3 fetch/cand.py <id> …`（可一次多个）取 URL / 摘要 / 作者。",
        "papers_intro_tail": "。每行 = arXiv id + 标题（·HF = 也在 HF Daily）；摘要用 `cand.py <arxiv id> …` 取。",
        "papers_group_note": "分组：先按偏好权重，再按主题；同一篇只出现一次。权重 0 的方向外论文：有信号的只列标题，无信号的已折叠（见文末说明），方向外的重大结果同样要收。",
        "title_nonpaper": "# 候选清单（论文以外）{start} → {end}", "title_papers": "# 候选论文 {start} → {end}（{n} 篇）",
        "pushed_line": "★ 已推过（同一条，除非有实质新进展不再写）：",
        "rep_line": "↺ 近日 triage 已列过的 trending/搜索发现（仅标题，{n} 条）：",
        "group_pref3": "偏好主题（权重 ≥3）", "group_pref12": "相关（权重 1–2）", "weight_n": "权重 {n}",
        "other": "其他", "rest_plain": "其余（{n}，按主题，仅标题）",
        "rest_full": "# 论文「其余」档全部标题（{n} 篇；其中 {k} 篇有信号已列在 triage-papers.md，{f} 篇折叠）",
        "rest_keep": "其余·有信号（{n}，仅标题；信号 = HF / 多源 / 被其他信源提及 / 台账 / 大结果关键词）",
        "rest_fold": "其余·已折叠（{n} 篇，方向外且无 HF / 多源 / 被提及 / 大结果关键词信号）",
        "fold_note": "全部标题在 `{path}`（按主题分组，一行一篇）。折叠档不需要通读；怀疑漏了某类大结果时再 "
                     "`rg -i -n '关键词1|关键词2' {path}`（例：`'frontier|technical report|open-weight|state-of-the-art'`）。",
        "report_title": "# 召回报告 {start} → {end}",
        "report_line": "- 用时 {sec}s；原始 {raw} 条，去重后 {dd} 条",
        "note_prefix": "- 注：",
        "report_header": "| 频道 | 信源 | 条数 | 耗时 | 状态 |",
        "arxiv_rolled": "arxiv: {label} 批次已滚出 /pastweek（公告于 {days} 天前），本次无法取回；回填请用 export API / 旧 state",
        "arxiv_missing": "arxiv: ⚠️ 工作日批次 {label}（本地 {when} 公告）在 /pastweek 与 /new 上都没有，已重试一次——这是真实的召回故障，"
                         "需人工核查 arXiv 是否放假/延迟；可用 --only arxiv 重跑",
        "arxiv_missing_prev": "arxiv: ⚠️ 窗口内这些工作日批次缺失：",
        "arxiv_from_new": "arxiv: {label} 批次取自 /new（/pastweek 尚未刷新）",
        "arxiv_pub": "公开于 {when}",
        "hf_utc": "hf_daily: UTC 榜单 ", "hf_none": "hf_daily: 无可取的已收榜日期",
        "hf_not_open": "hf_daily: {d} 尚未开放（HTTP 400），跳过",
        "fetch_lock_wait": "wait: 另一个 fetch.py 正在写 {outdir}，等它完成…",
        "fetch_lock_timeout": "另一个 fetch.py 正在写 {outdir}，已等 15 分钟仍未结束；稍后重跑同一条命令",
        "fetch_reuse": "reuse: {outdir} 已在 {at} 抓取过同一窗口，跳过联网抓取（需要重抓：去掉 --reuse）",
        "wl_title": "# 周报「后来才有信号、未推过」清单 {m} → {s}",
        "wl_sources": "数据来源（已有抓取）：", "wl_missing": "；缺：",
        "wl_gap": "补缺{start}→{end}",
        "wl_hint": "摘要/作者：python3 fetch/cand.py --since {m} <arXiv id|短id>；其余字段含义见脚本顶部说明。",
        "wl_papers": "## 论文：后来出现 HF / 多源 / 被提及，且从没推过（{n}，☆ = 日报看过未推）",
        "wl_weight": "[权重{n}]", "wl_first": "首见{d}", "wl_late_hf": "HF晚到{d}", "wl_hf": "HF{d}",
        "wl_big_org": "·头部机构", "wl_no_ledger": "·台账无记录",
        "wl_more": "  …另有 {n} 条未列（用 --max 调大）",
        "wl_dropped": "  （另有 {n} 篇仅 HF 晚到/同日、但权重 <2 或机构非头部的方向外论文已略；`--all` 可全列）",
        "wl_nonpaper": "## 实验室 / 政策 / 研究者博客（全部）+ 工具发布 / Newsletter / 播客（权重 ≥3）：台账没有任何记录（{n}；同类共 {total}）",
        "wl_more_np": "  …另有 {n} 条未列（`--max-np` 调大，或 `--all`）",
        "c_pushed": "★已推", "c_seen": "☆看过未推", "c_date_unknown": "日期未知", "c_discovery": "发现入口",
        "c_authors": "   作者: ", "c_et_al": " 等", "c_nomatch": "无匹配", "c_more": "   …另有 {n} 条匹配，用更精确的关键词",
        "cc_strong": "强", "cc_weak": "备", "cc_pushed": "★已推", "cc_unknown": "未知id", "cc_dup": "重复",
        "cc_bad_unknown": "未知 id（不在当天候选里，核对是否抄错）：", "cc_bad_dup": "重复 id：",
        "cc_bad_count": "条数 {n} 不在 10–60 之间（目标 30–40，宁宽勿窄）",
        "xp_trunc": "⚠️ 截断（只拿到窗口最新一段，更早的帖子没取到）：",
        "xp_parse_fail": "解析失败", "xp_oldest": "最早 {t}",
        "xp_wrote": "{counts}  → {out}（{n} 条，{bytes} 字节）",
        "lg_pushed": "推送", "lg_seen": "看过未推（供周报复查）", "lg_colon": "：",
        "lg_hits": "## {k}: {n} 条", "lg_recent": "# 近 {days} 天 {n} 条（status={status}）",
        "pg_date": "日期: ", "pg_desc": "描述: ", "pg_fallback": "改用 WebFetch",
        "pg_short": "(正文只有 {n} 字，疑似 JS 渲染 → 改用 WebFetch)",
        "pg_hits": "--- {kw}: {n}{plus} 处", "pg_trunc": "…（全文 {n} 字，已截断；用 --chars 或 --find）",
        "gh_rate_unauth": "github: ⚠️ 触发 HTTP {code} 限流；设置环境变量 GITHUB_TOKEN 或 GH_TOKEN 可将 GitHub API 限额从 60 次/小时提升至 5,000 次/小时",
        "gh_rate_auth": "github: ⚠️ 已配置 token 仍触发 HTTP {code} 限流；请检查 GITHUB_TOKEN / GH_TOKEN 配额或二级限流",
    },
}


def ft(lang: str, key: str, **kw) -> str:
    s = FETCH[normalize_lang(lang)][key]
    return s.format(**kw) if kw else s


def cnt_paren(lang: str, n: int) -> str:
    """'（12）' / ' (12)' appended to a heading."""
    return f"（{n}）" if normalize_lang(lang) == "zh-CN" else f" ({n})"
