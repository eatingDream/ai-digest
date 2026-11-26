use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

pub const STRINGS: &[(&str, &str, &str)] = &[
    ("html_lang", "en", "zh-CN"),
    ("brand_tail", " Digest", " Digest"),
    ("brand_plain", "AI Digest", "AI精选"),
    (
        "motto",
        "Models · Research · Engineering · Industry",
        "模型 · 研究 · 工程 · 产业",
    ),
    ("skip", "Skip to content", "跳到正文"),
    ("appearance", "Appearance", "外观"),
    ("aria_lang_switch", "Language", "语言"),
    ("lang_short", "English", "中文"),
    ("lang_code", "EN", "中文"),
    ("search_btn", "Search", "搜索"),
    (
        "search_toggle_title",
        "Search & filter (/)",
        "搜索与标签筛选（/）",
    ),
    ("search_close", "Close search", "收起搜索"),
    (
        "lang_toggle_title",
        "Switch language (L)",
        "切换中英文（L）",
    ),
    (
        "theme_toggle_title",
        "Switch appearance (T)",
        "切换外观（T）",
    ),
    ("theme_system", "System", "系统"),
    ("theme_light", "Light", "浅色"),
    ("theme_dark", "Dark", "深色"),
    ("copy_link", "Copy link", "复制链接"),
    ("copied", "Copied", "已复制"),
    ("read_time", "{m} min", "约 {m} 分钟"),
    ("latest_daily", "Latest daily", "最新日报"),
    ("latest_weekly", "Latest weekly", "最新周报"),
    ("kicker_daily", "Issue", "本期"),
    ("kicker_weekly", "Week", "本期"),
    ("kicker_other", "Issue", "本期"),
    ("edition_daily", "Daily", "日报"),
    ("edition_weekly", "Weekly", "周报"),
    ("daily_title", "AI Daily", "AI精选日报"),
    ("weekly_title", "AI Weekly", "AI精选周报"),
    ("toc", "Contents", "目录"),
    ("toc_open", "Open contents", "打开目录"),
    ("toc_close", "Close contents", "关闭目录"),
    ("toc_open_btn", "☰ Contents", "☰ 目录"),
    ("issue_sections", "In this issue", "本期版面"),
    ("items_one", "{n} item", "{n} 条"),
    ("items_many", "{n} items", "{n} 条"),
    ("must", "Must read this week", "本周看点"),
    ("themes", "Themes of the week", "本周主线"),
    ("front_aria", "Front page", "头版"),
    ("end", "End of issue", "（本期完）"),
    ("to_top", "Back to top", "回到顶部"),
    ("new", "New", "新"),
    ("label_look", "Why it matters", "值得看"),
    ("label_detail", "Details", "详情"),
    ("label_followup", "Follow-up", "本周后续"),
    ("related", "Related: ", "相关："),
    ("sec_default_papers", "Papers", "论文"),
    ("sec_default_items", "Items", "条目"),
    ("nav_aria", "Issue navigation", "期次导航"),
    ("nav_index", "Back to index", "返回目录"),
    ("nav_prev", "Previous", "上一期"),
    ("nav_next", "Next", "下一期"),
    ("nav_week", "This week's recap", "本周周报"),
    ("lang_name", "English", "简体中文"),
    ("lock", "Lock", "锁定"),
    (
        "lock_title",
        "Forget the key saved on this device",
        "清除本设备保存的密钥",
    ),
    ("index_title", "AI Digest", "AI Digest"),
    ("index_chip", "Archive", "目录"),
    ("index_chip_unit", "issues", "期"),
    (
        "index_meta",
        "{d} daily · {w} weekly",
        "日报 {d} · 周报 {w}",
    ),
    ("index_daily", "Daily · by week", "日报 · 按周"),
    ("index_weekly", "Weekly · by month", "周报 · 按月"),
    ("issues_n", "{n} issues", "{n} 期"),
    ("issues_one", "{n} issue", "{n} 期"),
    ("issue_unit_one", "issue", "期"),
    ("issue_unit_many", "issues", "期"),
    ("week_n", "Week {n}", "第 {n} 周"),
    ("week_link", "This week's recap →", "本周周报 →"),
    ("no_daily_week", "No daily issues this week", "本周暂无日报"),
    ("none_yet", "Nothing yet", "暂无"),
    ("month_label", "{month} {y}", "{y}年{m}月"),
    ("index_end", "AI Digest", "AI Digest"),
    ("filter_label", "Filter items", "筛选条目"),
    (
        "filter_placeholder",
        "Search title, summary, source, or tag…",
        "搜索标题、摘要、来源或标签…",
    ),
    ("archive_filter_label", "Filter archive", "筛选归档"),
    (
        "archive_filter_placeholder",
        "Search issues, headlines, or tags…",
        "搜索期次、标题或标签…",
    ),
    ("filter_all", "All", "全部"),
    ("filter_clear", "Clear", "清除"),
    (
        "filter_empty",
        "No matching items in this issue.",
        "本期没有匹配的条目。",
    ),
    (
        "archive_filter_empty",
        "No matching issues found.",
        "没有找到匹配的期次。",
    ),
    ("filter_shown", "{shown} / {total}", "{shown} / {total}"),
    ("pw_title", "AI Digest", "AI Digest"),
    ("pw_label", "Password", "密码"),
    ("pw_remember", "Remember this device", "记住此设备"),
    ("pw_open", "Open", "打开"),
    ("pw_opening", "Unlocking…", "正在解锁…"),
    ("pw_wrong", "Wrong password", "密码不对"),
    (
        "pw_unsupported",
        "This browser can't open this page",
        "此浏览器无法打开",
    ),
    (
        "pw_noscript",
        "JavaScript is required",
        "需要启用 JavaScript",
    ),
    (
        "pw_demo_hint",
        "Demo site · password: {pw}",
        "演示站 · 密码：{pw}",
    ),
];

pub const FETCH: &[(&str, &str, &str)] = &[
    ("sec_lab", "Lab official", "实验室官方"),
    ("sec_model", "Models / datasets / Spaces", "模型 / 数据集 / Spaces"),
    ("sec_tooling", "Tool & framework releases", "工具与框架发布"),
    ("sec_repo", "New GitHub projects", "GitHub 新项目"),
    ("sec_research_blog", "Researcher blogs / essays", "研究者博客 / 长文"),
    ("sec_newsletter", "Newsletters", "Newsletter"),
    ("sec_podcast", "Podcasts / video", "播客 / 视频"),
    ("sec_chips_infra", "Chips / compute / infrastructure", "芯片 / 算力 / 基础设施"),
    ("sec_policy", "Policy / safety & governance", "政策 / 安全治理"),
    ("sec_news", "Tech news", "科技新闻"),
    ("sec_news_zh", "Chinese-language media", "中文媒体"),
    ("sec_community", "Community discussion", "社区讨论"),
    ("big_tech_report", "technical report", "技术报告"),
    ("big_frontier", "frontier model name", "前沿模型名"),
    ("big_open", "open weights / model family", "开源权重/模型族"),
    ("big_scale", "scale", "规模"),
    ("big_bench", "hard benchmark / math", "硬基准/数学"),
    ("big_safety", "safety / adversarial", "安全对抗"),
    ("sig_multi", "multi-source:", "多源:"),
    ("sig_mentioned", "mentioned:", "被提及:"),
    ("sig_ledger", "ledger", "台账"),
    ("sig_kw", "keyword:", "词:"),
    ("head_a", "Note: [n] is the ranking weight of the preferred topics, not an inclusion bar; ★ = already pushed (in the ledger), ☆ = seen but not pushed; (+x) = also seen in other sources; ? = date unknown, open the page to check. Popularity numbers are never listed", "说明：[n] 是 用户偏好主题的排序权重，不是纳入门槛；★ = ledger 已推过，☆ = 看过未推；(+x) = 其他信源也出现；? = 日期未知需打开核对；热度数字一律不列"),
    ("head_b", "A trailing #xxxxxx is a short id; use `ai-digest cand <id> …` (several at once) to get URL / summary / authors.", "行尾 #xxxxxx 是短 id，用 `ai-digest cand <id> …`（可一次多个）取 URL / 摘要 / 作者。"),
    ("papers_intro_tail", ". Each line = arXiv id + title (·HF = also on HF Daily); fetch abstracts with `ai-digest cand <arxiv id> …`.", "。每行 = arXiv id + 标题（·HF = 也在 HF Daily）；摘要用 `ai-digest cand <arxiv id> …` 取。"),
    ("papers_group_note", "Grouping: by preference weight first, then topic; each paper appears once. Off-topic weight-0 papers: those with signals are listed by title only, the rest are folded (see the note at the end); off-topic major results still count.", "分组：先按偏好权重，再按主题；同一篇只出现一次。权重 0 的方向外论文：有信号的只列标题，无信号的已折叠（见文末说明），方向外的重大结果同样要收。"),
    ("title_nonpaper", "# Candidates (non-paper) {start} → {end}", "# 候选清单（论文以外）{start} → {end}"),
    ("title_papers", "# Candidate papers {start} → {end} ({n} papers)", "# 候选论文 {start} → {end}（{n} 篇）"),
    ("pushed_line", "★ Already pushed (same items; do not rewrite without real new progress): ", "★ 已推过（同一条，除非有实质新进展不再写）："),
    ("rep_line", "↺ Trending/search discoveries already listed in recent triage (titles only, {n}): ", "↺ 近日 triage 已列过的 trending/搜索发现（仅标题，{n} 条）："),
    ("group_pref3", "Preferred topics (weight ≥3)", "偏好主题（权重 ≥3）"),
    ("group_pref12", "Related (weight 1–2)", "相关（权重 1–2）"),
    ("weight_n", "Weight {n}", "权重 {n}"),
    ("other", "Other", "其他"),
    ("rest_plain", "Rest ({n}, by topic, titles only)", "其余（{n}，按主题，仅标题）"),
    ("rest_full", "# All titles in the 'rest' tier ({n} papers; {k} with signals are listed in triage-papers.md, {f} folded)", "# 论文「其余」档全部标题（{n} 篇；其中 {k} 篇有信号已列在 triage-papers.md，{f} 篇折叠）"),
    ("rest_keep", "Rest · with signals ({n}, titles only; signal = HF / multi-source / mentioned elsewhere / ledger / big-result keyword)", "其余·有信号（{n}，仅标题；信号 = HF / 多源 / 被其他信源提及 / 台账 / 大结果关键词）"),
    ("rest_fold", "Rest · folded ({n} papers, off-topic and no HF / multi-source / mention / big-result signal)", "其余·已折叠（{n} 篇，方向外且无 HF / 多源 / 被提及 / 大结果关键词信号）"),
    ("fold_note", "All titles are in `{path}` (grouped by topic, one per line). The folded tier does not need a full read; if you suspect a missed big result, run `rg -i -n 'kw1|kw2' {path}` (e.g. `'frontier|technical report|open-weight|state-of-the-art'`).", "全部标题在 `{path}`（按主题分组，一行一篇）。折叠档不需要通读；怀疑漏了某类大结果时再 `rg -i -n '关键词1|关键词2' {path}`（例：`'frontier|technical report|open-weight|state-of-the-art'`）。"),
    ("report_title", "# Fetch report {start} → {end}", "# 召回报告 {start} → {end}"),
    ("report_line", "- Took {sec}s; {raw} raw items, {dd} after dedup", "- 用时 {sec}s；原始 {raw} 条，去重后 {dd} 条"),
    ("note_prefix", "- Note: ", "- 注："),
    ("report_header", "| Channel | Source | Items | Time | Status |", "| 频道 | 信源 | 条数 | 耗时 | 状态 |"),
    ("arxiv_rolled", "arxiv: batch {label} has rolled out of /pastweek (announced {days} days ago) and cannot be retrieved now; backfill via the export API / old state", "arxiv: {label} 批次已滚出 /pastweek（公告于 {days} 天前），本次无法取回；回填请用 export API / 旧 state"),
    ("arxiv_missing", "arxiv: ⚠️ weekday batch {label} (announced {when} local) is missing from both /pastweek and /new; retried once — this is a real fetch failure: check whether arXiv skipped or delayed the batch; rerun with --only arxiv", "arxiv: ⚠️ 工作日批次 {label}（本地 {when} 公告）在 /pastweek 与 /new 上都没有，已重试一次——这是真实的召回故障，需人工核查 arXiv 是否放假/延迟；可用 --only arxiv 重跑"),
    ("arxiv_missing_prev", "arxiv: ⚠️ these weekday batches in the window are missing: ", "arxiv: ⚠️ 窗口内这些工作日批次缺失："),
    ("arxiv_from_new", "arxiv: batch {label} taken from /new (/pastweek not refreshed yet)", "arxiv: {label} 批次取自 /new（/pastweek 尚未刷新）"),
    ("arxiv_pub", "published {when}", "公开于 {when}"),
    ("hf_utc", "hf_daily: UTC boards ", "hf_daily: UTC 榜单 "),
    ("hf_none", "hf_daily: no complete board available to fetch", "hf_daily: 无可取的已收榜日期"),
    ("hf_not_open", "hf_daily: {d} not open yet (HTTP 400), skipped", "hf_daily: {d} 尚未开放（HTTP 400），跳过"),
    ("fetch_lock_wait", "wait: another ai-digest fetch is writing {outdir}, waiting for it to finish...", "wait: 另一个 ai-digest fetch 正在写 {outdir}，等它完成…"),
    ("fetch_lock_timeout", "another ai-digest fetch is still writing {outdir} after 15 minutes; re-run the same command later", "另一个 ai-digest fetch 正在写 {outdir}，已等 15 分钟仍未结束；稍后重跑同一条命令"),
    ("fetch_reuse", "reuse: {outdir} already fetched this window at {at}, skipping network (drop --reuse to re-fetch)", "reuse: {outdir} 已在 {at} 抓取过同一窗口，跳过联网抓取（需要重抓：去掉 --reuse）"),
    ("wl_title", "# Weekly: candidates with late signals that were never pushed {m} → {s}", "# 周报「后来才有信号、未推过」清单 {m} → {s}"),
    ("wl_sources", "Data sources (existing fetches): ", "数据来源（已有抓取）："),
    ("wl_missing", "; missing: ", "；缺："),
    ("wl_gap", "gap-fill {start}→{end}", "补缺{start}→{end}"),
    ("wl_hint", "Abstracts/authors: ai-digest cand --since {m} <arXiv id|short id>; field meanings: see `ai-digest weekly-late --help`.", "摘要/作者：ai-digest cand --since {m} <arXiv id|短id>；其余字段含义见 `ai-digest weekly-late --help`。"),
    ("wl_papers", "## Papers: showed up later on HF / multiple sources / mentioned, and never pushed ({n}; ☆ = seen in a daily but not pushed)", "## 论文：后来出现 HF / 多源 / 被提及，且从没推过（{n}，☆ = 日报看过未推）"),
    ("wl_weight", "[weight {n}]", "[权重{n}]"),
    ("wl_first", "first seen {d}", "首见{d}"),
    ("wl_late_hf", "HF late {d}", "HF晚到{d}"),
    ("wl_hf", "HF {d}", "HF{d}"),
    ("wl_big_org", "·top lab/institution", "·头部机构"),
    ("wl_no_ledger", "·no ledger record", "·台账无记录"),
    ("wl_more", "  …{n} more not listed (raise --max)", "  …另有 {n} 条未列（用 --max 调大）"),
    ("wl_dropped", "  ({n} off-topic papers that were only HF-late/same-day with weight <2 or a non-top institution were skipped; `--all` lists them all)", "  （另有 {n} 篇仅 HF 晚到/同日、但权重 <2 或机构非头部的方向外论文已略；`--all` 可全列）"),
    ("wl_nonpaper", "## Lab / policy / researcher blogs (all) + tool releases / newsletters / podcasts (weight ≥3): no ledger record ({n}; {total} of the same kinds in total)", "## 实验室 / 政策 / 研究者博客（全部）+ 工具发布 / Newsletter / 播客（权重 ≥3）：台账没有任何记录（{n}；同类共 {total}）"),
    ("wl_more_np", "  …{n} more not listed (raise `--max-np`, or `--all`)", "  …另有 {n} 条未列（`--max-np` 调大，或 `--all`）"),
    ("c_pushed", "★ pushed", "★已推"),
    ("c_seen", "☆ seen, not pushed", "☆看过未推"),
    ("c_date_unknown", "date unknown", "日期未知"),
    ("c_discovery", "discovery only", "发现入口"),
    ("c_authors", "   authors: ", "   作者: "),
    ("c_et_al", " et al.", " 等"),
    ("c_nomatch", "no match", "无匹配"),
    ("c_more", "   …{n} more matches; use a more specific keyword", "   …另有 {n} 条匹配，用更精确的关键词"),
    ("cc_strong", "strong", "强"),
    ("cc_weak", "maybe", "备"),
    ("cc_pushed", "★pushed", "★已推"),
    ("cc_unknown", "unknown ids", "未知id"),
    ("cc_dup", "duplicates", "重复"),
    ("cc_bad_unknown", "unknown ids (not in today's candidates; check for typos): ", "未知 id（不在当天候选里，核对是否抄错）："),
    ("cc_bad_dup", "duplicate ids: ", "重复 id："),
    ("cc_bad_count", "{n} entries is outside 10–60 (target 30–40; err on the wide side)", "条数 {n} 不在 10–60 之间（目标 30–40，宁宽勿窄）"),
    ("xp_trunc", "⚠️ truncated (only the newest slice of the window was returned; earlier posts were not fetched): ", "⚠️ 截断（只拿到窗口最新一段，更早的帖子没取到）："),
    ("xp_parse_fail", "parse failed", "解析失败"),
    ("xp_oldest", "oldest {t}", "最早 {t}"),
    ("xp_wrote", "{counts}  → {out} ({n} items, {bytes} bytes)", "{counts}  → {out}（{n} 条，{bytes} 字节）"),
    ("lg_pushed", "Pushed", "推送"),
    ("lg_seen", "Seen, not pushed (for the weekly to re-check)", "看过未推（供周报复查）"),
    ("lg_colon", ": ", "："),
    ("lg_hits", "## {k}: {n} match(es)", "## {k}: {n} 条"),
    ("lg_recent", "# last {days} days: {n} entries (status={status})", "# 近 {days} 天 {n} 条（status={status}）"),
    ("pg_date", "Date: ", "日期: "),
    ("pg_desc", "Description: ", "描述: "),
    ("pg_fallback", "use WebFetch instead", "改用 WebFetch"),
    ("pg_short", "(body is only {n} characters, probably JS-rendered; use WebFetch instead)", "(正文只有 {n} 字，疑似 JS 渲染 → 改用 WebFetch)"),
    ("pg_hits", "--- {kw}: {n}{plus} hit(s)", "--- {kw}: {n}{plus} 处"),
    ("pg_trunc", "… (full text {n} characters, truncated; use --chars or --find)", "…（全文 {n} 字，已截断；用 --chars 或 --find）"),
    ("gh_rate_unauth", "github: ⚠️ HTTP {code} rate limit hit; set GITHUB_TOKEN or GH_TOKEN to raise the GitHub API limit from 60 to 5,000 req/hr", "github: ⚠️ 触发 HTTP {code} 限流；设置环境变量 GITHUB_TOKEN 或 GH_TOKEN 可将 GitHub API 限额从 60 次/小时提升至 5,000 次/小时"),
    ("gh_rate_auth", "github: ⚠️ HTTP {code} rate limit hit even with token; check GITHUB_TOKEN / GH_TOKEN quota or secondary rate limits", "github: ⚠️ 已配置 token 仍触发 HTTP {code} 限流；请检查 GITHUB_TOKEN / GH_TOKEN 配额或二级限流"),
];

pub const LABEL_MAP: &[(&str, &str)] = &[
    ("一句话", "dek"),
    ("In brief", "dek"),
    ("值得看", "look"),
    ("Why it matters", "look"),
    ("详情", "detail"),
    ("Details", "detail"),
    ("本周后续", "followup"),
    ("Follow-up", "followup"),
];
pub const FRONT_BLOCKS: &[(&str, &str)] = &[
    ("本周必看", "must"),
    ("🔥本周必看", "must"),
    ("Must read this week", "must"),
    ("🔥Must read this week", "must"),
    ("Must read", "must"),
    ("本周主线", "themes"),
    ("🧭本周主线", "themes"),
    ("Themes of the week", "themes"),
    ("🧭Themes of the week", "themes"),
    ("Themes", "themes"),
    ("目录", "toc"),
    ("📋目录", "toc"),
    ("Contents", "toc"),
    ("📋Contents", "toc"),
];
pub const SECTION_IDS: &[(&[&str], &str)] = &[
    (&["论文", "paper"], "sec-papers"),
    (
        &["工具", "项目", "开源", "tools", "project", "open source"],
        "sec-tools",
    ),
    (
        &[
            "算力",
            "基础设施",
            "芯片",
            "硬件",
            "compute",
            "infrastructure",
            "chips",
            "hardware",
        ],
        "sec-infra",
    ),
    (
        &[
            "政策",
            "安全",
            "治理",
            "监管",
            "policy",
            "safety",
            "governance",
            "regulation",
        ],
        "sec-policy",
    ),
    (
        &[
            "行业", "产品", "商业", "融资", "公司", "industry", "product", "business", "funding",
        ],
        "sec-industry",
    ),
    (
        &[
            "观点",
            "长文",
            "访谈",
            "播客",
            "讨论",
            "opinion",
            "essay",
            "interview",
            "podcast",
            "voices",
        ],
        "sec-voices",
    ),
    (&["评测", "基准", "数据", "eval", "benchmark"], "sec-evals"),
    (&["模型", "model"], "sec-models"),
];
pub const DROPPED_MARKERS: &[&str] = &[
    "## Dropped this week",
    "## 本周剔除",
    "**Dropped this week**",
    "**本周剔除**",
    "🗑Dropped this week",
    "🗑本周剔除",
];
pub const BADGE_ACCENT_TAGS: &[&str] = &[
    "Frontier release",
    "Major",
    "Open weights",
    "Primary source",
    "RSI",
    "一手",
    "前沿发布",
    "开源权重",
    "重大",
];
pub const RELATED_WORDS: &[&str] = &["相关", "Related"];
pub const WEEKLY_TITLE_WORDS: &[&str] = &["周报", "weekly"];
pub const CHECK_CANDS_STRONG_TIER_WORDS: &[&str] = &["强", "必看", "strong", "must"];
pub const MONTHS: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [[&str; 7]; 2] = [
    [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ],
    [
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
        "星期日",
    ],
];
const WEEKDAYS_SHORT: [[&str; 7]; 2] = [
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    ["周一", "周二", "周三", "周四", "周五", "周六", "周日"],
];

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub enum Lang {
    #[default]
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

impl Lang {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }
    pub const fn other(self) -> Self {
        match self {
            Self::En => Self::ZhCn,
            Self::ZhCn => Self::En,
        }
    }
    pub const fn choose<T: Copy>(self, en: T, zh: T) -> T {
        match self {
            Self::En => en,
            Self::ZhCn => zh,
        }
    }
}

impl std::fmt::Display for Lang {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unsupported language {0:?}; use one of: en, zh-CN")]
pub struct LangError(String);

pub fn normalize_lang(value: &str) -> Result<Lang, LangError> {
    match value.trim().to_lowercase().replace(' ', "").as_str() {
        "" | "en" | "en-us" | "en-gb" | "english" => Ok(Lang::En),
        "zh-cn" | "zh_cn" | "zh" | "zh-hans" | "cn" | "chinese" | "中文" | "简体中文" => {
            Ok(Lang::ZhCn)
        }
        _ => Err(LangError(value.to_owned())),
    }
}

pub fn is_valid_lang(value: &str) -> bool {
    normalize_lang(value).is_ok()
}

pub fn resolve_lang(cli: Option<&str>, config: Option<&str>) -> Result<Lang, LangError> {
    resolve_lang_with_env(cli, std::env::var("DIGEST_LANG").ok().as_deref(), config)
}

pub fn resolve_lang_with_env(
    cli: Option<&str>,
    env: Option<&str>,
    config: Option<&str>,
) -> Result<Lang, LangError> {
    normalize_lang(
        [cli, env, config]
            .into_iter()
            .flatten()
            .find(|v| !v.trim().is_empty())
            .unwrap_or(""),
    )
}

pub fn string(lang: Lang, key: &str) -> Option<&'static str> {
    lookup(STRINGS, lang, key)
}
pub fn fetch_string(lang: Lang, key: &str) -> Option<&'static str> {
    lookup(FETCH, lang, key)
}

fn lookup(
    table: &[(&'static str, &'static str, &'static str)],
    lang: Lang,
    key: &str,
) -> Option<&'static str> {
    table
        .iter()
        .find(|row| row.0 == key)
        .map(|row| lang.choose(row.1, row.2))
}

pub fn t(lang: Lang, key: &str) -> &'static str {
    string(lang, key).expect("unknown UI string key")
}
pub fn ft(lang: Lang, key: &str) -> &'static str {
    fetch_string(lang, key).expect("unknown fetch string key")
}
pub fn all_string_keys() -> impl Iterator<Item = &'static str> {
    STRINGS.iter().map(|row| row.0)
}
pub fn all_fetch_keys() -> impl Iterator<Item = &'static str> {
    FETCH.iter().map(|row| row.0)
}

pub fn count_items(lang: Lang, n: usize) -> String {
    t(lang, if n == 1 { "items_one" } else { "items_many" }).replace("{n}", &n.to_string())
}
pub fn count_issues(lang: Lang, n: usize) -> String {
    t(lang, if n == 1 { "issues_one" } else { "issues_n" }).replace("{n}", &n.to_string())
}
pub fn issue_unit(lang: Lang, n: usize) -> &'static str {
    t(
        lang,
        if n == 1 {
            "issue_unit_one"
        } else {
            "issue_unit_many"
        },
    )
}
pub fn cnt_paren(lang: Lang, n: usize) -> String {
    match lang {
        Lang::En => format!(" ({n})"),
        Lang::ZhCn => format!("（{n}）"),
    }
}
pub fn weekday(lang: Lang, d: NaiveDate) -> &'static str {
    WEEKDAYS[lang.choose(0, 1)][d.weekday().num_days_from_monday() as usize]
}
pub fn weekday_short(lang: Lang, d: NaiveDate) -> &'static str {
    WEEKDAYS_SHORT[lang.choose(0, 1)][d.weekday().num_days_from_monday() as usize]
}
pub fn date_long(lang: Lang, d: NaiveDate) -> String {
    match lang {
        Lang::En => d.format("%B %-d, %Y").to_string(),
        Lang::ZhCn => d.format("%Y 年 %-m 月 %-d 日").to_string(),
    }
}
pub fn date_range(lang: Lang, a: NaiveDate, b: NaiveDate) -> String {
    match (lang, a.year() == b.year()) {
        (Lang::En, true) => format!("{} – {}", a.format("%b %-d"), b.format("%b %-d, %Y")),
        (Lang::En, false) => format!("{} – {}", a.format("%b %-d, %Y"), b.format("%b %-d, %Y")),
        (Lang::ZhCn, true) => format!("{} — {}", date_long(lang, a), b.format("%-m 月 %-d 日")),
        (Lang::ZhCn, false) => format!("{} — {}", date_long(lang, a), date_long(lang, b)),
    }
}
pub fn month_label(lang: Lang, y: i32, m: u32) -> String {
    let label = t(lang, "month_label").replace("{y}", &y.to_string());
    match lang {
        Lang::En => label.replace(
            "{month}",
            MONTHS.get(m.wrapping_sub(1) as usize).unwrap_or(&""),
        ),
        Lang::ZhCn => label.replace("{m}", &m.to_string()),
    }
}
pub fn year_month(lang: Lang, date: NaiveDate) -> String {
    month_label(lang, date.year(), date.month())
}
pub fn year_week(lang: Lang, year: i32, week: u32) -> String {
    match lang {
        Lang::En => format!("{year} · Week {week}"),
        Lang::ZhCn => format!("{year} 年第 {week} 周"),
    }
}
pub fn looks_weekly(title: &str) -> bool {
    let title = title.to_lowercase();
    WEEKLY_TITLE_WORDS.iter().any(|word| title.contains(word))
}
