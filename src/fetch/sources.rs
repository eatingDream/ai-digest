use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        LazyLock, Mutex,
    },
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use chrono::{
    DateTime, Datelike, Days, Duration as Delta, FixedOffset, NaiveDate, NaiveDateTime, TimeZone,
    Utc,
};
use chrono_tz::Tz;
use html5gum::Token;
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use regex::Regex;
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT},
    StatusCode,
};
use serde_json::{json, Map, Value};
use url::Url;

use crate::i18n::{ft, Lang};
use crate::model::{
    canon_url, clean_text as collapse, html_tokens, html_unescape, norm_title, Cand, FetchState,
    InLedger, SourceSpec, SourcesConfig,
};
use crate::net::{read_bytes_limited, read_text_limited, SafeHttpClient, MAX_BODY_BYTES};

pub const TYPES: &[&str] = &[
    "rss",
    "html_links",
    "arxiv",
    "hf_daily",
    "hf_org_models",
    "hf_trending",
    "github_trending",
    "github_releases",
    "github_org_repos",
    "github_search",
    "hn",
    "reddit",
];
pub const CHANNELS: &[&str] = &[
    "lab",
    "model",
    "tooling",
    "repo",
    "research_blog",
    "newsletter",
    "podcast",
    "chips_infra",
    "policy",
    "news",
    "news_zh",
    "community",
    "paper",
];
pub const KINDS: &[&str] = &[
    "lab_post",
    "paper",
    "model",
    "release",
    "repo",
    "policy",
    "essay",
    "podcast",
    "video",
    "news",
    "dataset",
    "discussion",
];
pub static ARXIV_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(\d{4}\.\d{4,5})(?:v\d+)?\b").unwrap());
const RSS_ACCEPT: &str = "application/rss+xml,application/atom+xml,application/xml;q=0.9,*/*;q=0.8";

#[derive(Debug)]
pub struct Settings {
    pub timeout: u64,
    pub workers: usize,
    pub user_agent: String,
    pub summary_chars: usize,
}

pub fn parse_registry(text: &str) -> Result<(Settings, Vec<SourceSpec>)> {
    let config: SourcesConfig = toml::from_str(text)?;
    let integer = |key: &str, default: i64| -> Result<usize> {
        let n = match config.settings.get(key) {
            Some(value) => value
                .as_integer()
                .with_context(|| format!("invalid settings.{key}"))?,
            None => default,
        };
        ensure!(n > 0, "settings.{key} must be positive");
        Ok(usize::try_from(n)?)
    };
    let settings = Settings {
        timeout: integer("timeout", 20)? as u64,
        workers: integer("workers", 24)?,
        summary_chars: integer("summary_chars", 700)?,
        user_agent: config
            .settings
            .get("user_agent")
            .map(|v| v.as_str().context("invalid settings.user_agent"))
            .transpose()?
            .unwrap_or("Mozilla/5.0")
            .into(),
    };
    ensure!(
        settings.workers <= tokio::sync::Semaphore::MAX_PERMITS,
        "settings.workers exceeds semaphore capacity"
    );
    let mut ids = HashSet::new();
    for s in &config.source {
        ensure!(
            !s.id.trim().is_empty() && !s.name.trim().is_empty() && ids.insert(&s.id),
            "invalid or duplicate source ID: {}",
            s.id
        );
        ensure!(
            TYPES.contains(&s.source_type.as_str())
                && CHANNELS.contains(&s.channel.as_str())
                && KINDS.contains(&s.kind.as_str()),
            "invalid type, channel, or kind for {}",
            s.id
        );
        for (field, values, required) in [
            ("urls", &s.urls, s.source_type == "github_trending"),
            (
                "orgs",
                &s.orgs,
                matches!(s.source_type.as_str(), "hf_org_models" | "github_org_repos"),
            ),
            ("repos", &s.repos, s.source_type == "github_releases"),
            ("categories", &s.categories, s.source_type == "arxiv"),
            ("queries", &s.queries, s.source_type == "github_search"),
        ] {
            let unique: HashSet<_> = values.iter().collect();
            ensure!(
                (!required || !values.is_empty())
                    && unique.len() == values.len()
                    && values.iter().all(|v| !v.trim().is_empty()),
                "invalid {field} for {}",
                s.id
            );
        }
        if matches!(
            s.source_type.as_str(),
            "rss" | "html_links" | "hf_daily" | "hf_trending" | "reddit"
        ) {
            ensure!(s.url.is_some(), "missing URL for {}", s.id);
        }
        for url in s.url.iter().chain(&s.urls) {
            let url = Url::parse(url).with_context(|| format!("invalid URL for {}", s.id))?;
            ensure!(
                matches!(url.scheme(), "http" | "https")
                    && url.has_host()
                    && url.username().is_empty()
                    && url.password().is_none(),
                "invalid HTTP URL for {}",
                s.id
            );
        }
        ensure!(
            s.filter.as_deref().is_none_or(|v| v == "ai"),
            "invalid filter for {}",
            s.id
        );
        if s.source_type == "html_links" {
            ensure!(s.pattern.is_some(), "missing pattern for {}", s.id);
        }
        for pattern in s.pattern.iter().chain(&s.exclude) {
            Regex::new(pattern).with_context(|| format!("invalid link pattern for {}", s.id))?;
        }
    }
    Ok((settings, config.source))
}

pub fn short(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(end, _)| &s[..end])
}

pub fn clean_text(s: &str, limit: usize) -> String {
    let text = if s.contains('<') {
        html_tokens(s)
            .filter_map(|token| match token {
                Token::String(text) => Some(String::from_utf8_lossy(&text).into_owned()),
                _ => None,
            })
            .collect::<String>()
    } else {
        s.to_owned()
    };
    let text = collapse(&html_unescape(&text));
    if limit > 0 && text.chars().count() > limit {
        format!("{}…", short(&text, limit - 1).trim_end())
    } else {
        text
    }
}

static ZONE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*([+-]\d{2}):?(\d{2})$").unwrap());

fn safe_local_datetime(tz: Tz, local: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
    local.checked_sub_signed(Delta::days(2))?;
    local.checked_add_signed(Delta::days(2))?;
    tz.from_local_datetime(&local)
        .earliest()
        .map(|d| d.fixed_offset())
}

fn checked_with_tz<T: TimeZone>(dt: DateTime<T>, tz: Tz) -> Result<DateTime<Tz>> {
    let utc = dt.naive_utc();
    ensure!(
        utc.checked_sub_signed(Delta::days(2)).is_some()
            && utc.checked_add_signed(Delta::days(2)).is_some(),
        "window date out of representable timezone range"
    );
    Ok(dt.with_timezone(&tz))
}

fn parse_iso(s: &str, tz: Tz) -> Option<DateTime<FixedOffset>> {
    let s = ZONE_SUFFIX.replace(s, "$1:$2");
    let s = s.replacen(' ', "T", 1);
    DateTime::parse_from_rfc3339(&s)
        .ok()
        .or_else(|| DateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M%:z").ok())
        .or_else(|| {
            let local = ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
                .iter()
                .find_map(|f| NaiveDateTime::parse_from_str(&s, f).ok())
                .or_else(|| {
                    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                        .ok()?
                        .and_hms_opt(0, 0, 0)
                })?;
            safe_local_datetime(tz, local)
        })
}

pub fn parse_dt(s: &str) -> Option<DateTime<FixedOffset>> {
    let s = s.trim();
    DateTime::parse_from_rfc2822(s)
        .ok()
        .or_else(|| DateTime::parse_from_str(s, "%a, %d %b %Y %H:%M:%S %z").ok())
        .or_else(|| {
            NaiveDateTime::parse_from_str(s, "%a, %d %b %Y %H:%M:%S")
                .ok()
                .map(|dt| dt.and_utc().fixed_offset())
        })
        .or_else(|| parse_iso(s, chrono_tz::UTC))
        .or_else(|| parse_iso(s.get(..10)?, chrono_tz::UTC))
}

static TEXT_DATES: LazyLock<Vec<Regex>> =
    LazyLock::new(|| {
        [
    r"(20\d\d)[./-](\d{1,2})[./-]?/?(\d{1,2})(?:\D|$)",
    r"(20\d\d)年(\d{1,2})月(\d{1,2})日",
    r"(?i)(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.? (\d{1,2}),? (20\d\d)",
    r"(?i)(?:^|\D)(\d{1,2}) (jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]* (20\d\d)",
    r"/news(\d{2})(\d{2})(\d{2})$",
].iter().map(|p| Regex::new(p).unwrap()).collect()
    });

pub fn date_from_text(text: &str) -> Option<DateTime<FixedOffset>> {
    let (i, m) = TEXT_DATES
        .iter()
        .enumerate()
        .find_map(|(i, rx)| Some((i, rx.captures(text)?)))?;
    let month = |s: &str| {
        [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ]
        .iter()
        .position(|v| s.eq_ignore_ascii_case(v))
        .map(|i| i as u32 + 1)
    };
    let day = match i {
        2 => NaiveDate::from_ymd_opt(m[3].parse().ok()?, month(&m[1])?, m[2].parse().ok()?),
        3 => NaiveDate::from_ymd_opt(m[3].parse().ok()?, month(&m[2])?, m[1].parse().ok()?),
        _ => NaiveDate::from_ymd_opt(
            m[1].parse::<i32>().ok()? + if i == 4 { 2000 } else { 0 },
            m[2].parse().ok()?,
            m[3].parse().ok()?,
        ),
    }?;
    Some(day.and_hms_opt(12, 0, 0)?.and_utc().fixed_offset())
}

#[derive(Clone, Debug)]
pub struct Window {
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub label: String,
    pub timezone: Tz,
}

impl Window {
    pub fn contains(&self, dt: DateTime<FixedOffset>) -> bool {
        self.start <= dt && dt < self.end
    }

    pub fn iso<T: TimeZone>(&self, dt: DateTime<T>) -> String {
        dt.with_timezone(&self.timezone)
            .format("%Y-%m-%dT%H:%M%:z")
            .to_string()
    }
}

pub fn compute_window(
    tz: Tz,
    now: DateTime<Utc>,
    weekly: bool,
    daily: bool,
    day: Option<NaiveDate>,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Window> {
    let midnight = |day: NaiveDate| {
        let local = day.and_hms_opt(0, 0, 0).context("invalid date")?;
        safe_local_datetime(tz, local).context("local date has no representable midnight")
    };
    let today = checked_with_tz(now, tz)?.date_naive();
    let date = match day {
        Some(d) => d,
        None => today
            .checked_sub_days(Days::new(if weekly { 7 } else { 1 }))
            .context("default window date out of range")?,
    };
    let first = if weekly {
        date.checked_sub_days(Days::new(u64::from(date.weekday().num_days_from_monday())))
            .context("weekly start date out of range")?
    } else {
        date
    };
    let lo = match start {
        Some(s) => parse_iso(s, tz).with_context(|| format!("invalid window date: {s}"))?,
        None => midnight(first)?,
    };
    let hi = match end {
        Some(s) => parse_iso(s, tz).with_context(|| format!("invalid window date: {s}"))?,
        None => {
            let default_end = first
                .checked_add_days(Days::new(if weekly { 7 } else { 1 }))
                .context("window end date out of range")?;
            midnight(default_end)?
        }
    };
    let _ = checked_with_tz(lo, tz)?;
    let _ = checked_with_tz(hi, tz)?;
    ensure!(lo < hi, "bad window {lo} → {hi}");
    let label = if !weekly && !daily && day.is_none() && start.is_some() && end.is_some() {
        let hi_prev = hi
            .checked_sub_signed(Delta::microseconds(1))
            .context("window end date out of range")?;
        checked_with_tz(hi_prev, tz)?.date_naive().to_string()
    } else if weekly {
        let week_end = first
            .checked_add_days(Days::new(6))
            .context("weekly end date out of range")?;
        format!("{week_end}-weekly")
    } else {
        first.to_string()
    };
    Ok(Window {
        start: lo,
        end: hi,
        label,
        timezone: tz,
    })
}

pub fn load_candidates_jsonl(dir: &Path) -> Result<Vec<Cand>> {
    let file = dir.join("candidates.jsonl");
    fs::read_to_string(&file)
        .with_context(|| format!("read {}", file.display()))?
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(i, line)| {
            serde_json::from_str(line).with_context(|| format!("{} line {}", file.display(), i + 1))
        })
        .collect()
}

pub fn load_preferences(path: &Path) -> BTreeMap<String, i32> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<BTreeMap<String, Value>>(&s).ok())
        .and_then(|map| {
            map.into_iter()
                .map(|(k, v)| {
                    Some((
                        k,
                        v.as_i64()
                            .and_then(|n| i32::try_from(n).ok())
                            .or_else(|| v.as_str()?.parse().ok())?,
                    ))
                })
                .collect()
        })
        .unwrap_or_else(|| {
            ["agents", "training", "reasoning", "evals", "inference"]
                .into_iter()
                .map(|s| (s.into(), 1))
                .collect()
        })
}

pub fn status(c: &Cand) -> &str {
    match &c.in_ledger {
        InLedger::Status(s) => s,
        InLedger::Flag(_) => "",
    }
}

pub fn candidate_key(c: &Cand) -> String {
    c.extra
        .get("arxiv_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map_or_else(|| canon_url(&c.url), |aid| format!("arxiv:{aid}"))
}

pub fn cross_references(cands: &[Cand]) -> BTreeMap<String, String> {
    let mut refs = BTreeMap::new();
    for c in cands.iter().filter(|c| c.channel != "paper") {
        for hit in ARXIV_ID.captures_iter(&format!("{} {} {}", c.url, c.title, c.summary)) {
            refs.entry(hit[1].to_owned())
                .or_insert_with(|| c.source.clone());
        }
    }
    refs
}

pub fn fmt_label(lang: Lang, key: &str, fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .fold(ft(lang, key).to_owned(), |label, (key, value)| {
            label.replace(&format!("{{{key}}}"), value)
        })
}

pub fn paper_signals(c: &Cand, xref: &BTreeMap<String, String>, lang: Lang) -> Vec<String> {
    let mut tags = Vec::new();
    if c.source == "hf_daily" || c.also_in.iter().any(|s| s == "hf_daily") {
        let org = str_field(&Value::Object(c.extra.clone()), "org").to_owned();
        tags.push(if org.is_empty() {
            "HF".into()
        } else {
            format!("HF·{}", short(&org, 24))
        });
    }
    let others: Vec<_> = c
        .also_in
        .iter()
        .chain(std::iter::once(&c.source))
        .filter(|s| !matches!(s.as_str(), "arxiv" | "hf_daily"))
        .take(3)
        .cloned()
        .collect();
    if !others.is_empty() {
        tags.push(format!("{}{}", ft(lang, "sig_multi"), others.join(",")));
    }
    if let Some(source) = c
        .extra
        .get("arxiv_id")
        .and_then(Value::as_str)
        .and_then(|aid| xref.get(aid))
    {
        tags.push(format!(
            "{}{}",
            ft(lang, "sig_mentioned"),
            short(source, 30)
        ));
    }
    if !status(c).is_empty() || matches!(c.in_ledger, InLedger::Flag(true)) {
        tags.push(ft(lang, "sig_ledger").into());
    }
    let text = format!("{} {}", c.title, c.summary);
    let matches: Vec<_> = PAPER_PATTERNS
        .iter()
        .filter(|(_, re)| re.is_match(&text))
        .take(2)
        .map(|(key, _)| ft(lang, key))
        .collect();
    if !matches.is_empty() {
        tags.push(format!("{}{}", ft(lang, "sig_kw"), matches.join("/")));
    }
    tags
}

static AI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(a\.?i\.?|ai's|artificial intelligence|machine learning|deep learning|neural|llms?|gpt|chatgpt|openai|anthropic|claude|gemini|deepmind|grok|xai|mistral|llama|qwen|deepseek|kimi|moonshot|zhipu|glm|minimax|copilot|cursor|agentic|agents?|transformer|diffusion|inference|gpu|gpus|nvidia|tpu|cuda|h100|h200|b200|b300|gb200|gb300|blackwell|rubin|hbm|datacenter|data center|chatbot|reasoning model|frontier model|foundation model|robot|robotics|humanoid|embodied|hugging ?face|alignment|interpretability|superintelligence|agi|rlhf|fine-?tun\w*|pretrain\w*|token|benchmark|multimodal|text-to-\w+|world model|autonomous)\b|人工智能|大模型|智能体|模型|算力|芯片|机器人|具身|生成式|算法|AI|英伟达|推理|训练|开源|多模态|自动驾驶"#).unwrap()
});

static PRERELEASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(alpha|beta|\brc\.?\d*\b|-rc|\.dev\d+|\bdev\d*\b|nightly|-pre\b|preview\.\d+|canary)"#).unwrap()
});

static DERIV_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(gguf|\bawq\b|gptq|exl[23]|-mlx|mlx-|\d+-?bit\b|nvfp4|mxfp4|-fp[48]\b|-int[48]\b|\bq\d_k|\bq\dk\b|-q\d\b|abliterat|uncensor|heretic|\bmerge|frankenmerge|-lora\b|quantiz)"#).unwrap()
});

static SPAM_OWNER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^[a-z]+\d{4,}$|^[a-z]+[0-9]{2,}[a-z]+\d{2,}$"#).unwrap());

static TOPIC_PATTERNS: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
    [
    ("harness", r#"(?i)harness|scaffold|agent framework|agent runtime|coding agent|computer[- ]use|tool[- ]use|tool calling|mcp\b|model context protocol|claude code|codex|skills?\b|workflow"#),
    ("rsi", r#"(?i)self[- ]improv|self[- ]evolv|recursive|rsi\b|automated (ai )?research|ai scientist|self[- ]play|self[- ]refin|meta[- ]learn|open[- ]ended|auto(mated)? ?ml"#),
    ("training", r#"(?i)reinforcement learning|\brl\b|rlvr|rlhf|grpo|ppo\b|dpo\b|reward|post[- ]training|pre[- ]?training|distill|fine[- ]?tun|sft\b|curriculum|scaling law|optimizer|data mixture|synthetic data|on[- ]policy|mixture[- ]of[- ]experts|\bmoe\b"#),
    ("inference", r#"(?i)inference|serving|speculative|kv[- ]?cache|quantiz|decoding|latency|throughput|vllm|sglang|llama\.cpp|long[- ]context|attention kernel"#),
    ("agents", r#"(?i)\bagents?\b|agentic|multi[- ]agent|planning|memory|browser|web agent|swe-bench|terminal-bench|tau.?bench"#),
    ("reasoning", r#"(?i)reasoning|chain[- ]of[- ]thought|\bcot\b|math|theorem|proof|test[- ]time"#),
    ("multimodal", r#"(?i)vision|image|video|audio|speech|tts\b|asr\b|multimodal|vlm|text-to-|diffusion|3d"#),
    ("robotics", r#"(?i)robot|embodied|humanoid|manipulation|locomotion|vla\b|autonomous driving|self-driving"#),
    ("safety", r#"(?i)safety|alignment|interpretab|mechanistic|jailbreak|red[- ]team|deception|sycophan|misalign|monitor|sabotage|honest|reward hacking|control\b|bio(risk|security)|cyber"#),
    ("evals", r#"(?i)benchmark|evaluation|\beval|leaderboard|arena"#),
    ("data", r#"(?i)dataset|corpus|data curation|data quality|benchmark data|annotation"#),
    ("infra", r#"(?i)gpu|tpu|cluster|datacenter|data center|chip|semiconductor|hbm|nvidia|amd|tsmc|compute|power|energy|interconnect|networking|cuda|kernel|distributed training|colossus|stargate"#),
    ("business", r#"(?i)raises?|funding|series [a-h]\b|valuation|acquir|acquisition|merger|ipo\b|revenue|investment|investor|layoff|deal\b|partnership|pricing"#),
    ("policy", r#"(?i)regulat|policy|law\b|legislat|executive order|congress|senate|eu ai act|ai act|copyright|lawsuit|court|export control|ban\b|government|ministry|网信办|监管|政策|法规"#),
    ("people", r#"(?i)joins|hired|hires|departs|leaves|steps down|appointed|ceo\b|chief scientist|co-?founder|离职|加入|任命"#),
    ("opensource", r#"(?i)open[- ]source|open[- ]weight|apache 2\.0|mit license|weights released|开源"#),
    ("product", r#"(?i)launch|rolls out|now available|release[sd]?\b|introduc|app\b|feature|pricing|plan\b|发布|上线"#),
].into_iter().map(|(key, pattern)| (key, Regex::new(pattern).unwrap())).collect()
});

static PAPER_PATTERNS: LazyLock<Vec<(&str, Regex)>> = LazyLock::new(|| {
    [
    ("big_tech_report", r#"(?i)technical report|tech report"#),
    ("big_frontier", r#"(?i)\b(?:gpt-?\d|chatgpt|claude|gemini|llama-?\d|qwen ?\d|deepseek|kimi|glm-?\d|mistral|grok)\b"#),
    ("big_open", r#"(?i)open[- ]weights?|open[- ]source (?:model|llm|foundation model)|model family|we (?:release|open-?source)\b.{0,50}(?:model|weights|checkpoint)"#),
    ("big_scale", r#"(?i)\btrillion\b|scaling laws?"#),
    ("big_bench", r#"(?i)swe-?bench|terminal-?bench|arc-?agi|humanity.s last exam|frontiermath|\bgpqa\b|\bputnam\b|\bimo\b|erd[oő]s|open problem|solves? (?:an )?open"#),
    ("big_safety", r#"(?i)jailbreak|backdoor|alignment[- ]faking|scheming|sabotag|reward hacking|red[- ]team|sleeper agent|prompt injection"#),
].into_iter().map(|(key, pattern)| (key, Regex::new(pattern).unwrap())).collect()
});

pub fn score_topics(c: &mut Cand, preferences: &BTreeMap<String, i32>) {
    let text = format!("{} {}", c.title, short(&c.summary, 400));
    let policy_in_title = c.title.to_lowercase().contains("policy");
    c.topics = TOPIC_PATTERNS
        .iter()
        .filter(|(key, rx)| {
            (c.kind != "paper"
                || (!matches!(*key, "product" | "business" | "people")
                    && (*key != "policy" || policy_in_title)))
                && rx.is_match(&text)
        })
        .map(|(key, _)| (*key).into())
        .collect();
    c.pref = c
        .topics
        .iter()
        .map(|key| preferences.get(key).copied().unwrap_or(0))
        .fold(0i32, i32::saturating_add);
}

fn ai_ok(c: &Cand) -> bool {
    AI_RE.is_match(&format!("{} {}", c.title, short(&c.summary, 300)))
}

pub fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

pub const MAX_DOM_DEPTH: usize = 128;
pub const MAX_DOM_NODES: usize = 100_000;

#[derive(Default)]
pub(crate) struct Element {
    pub tag: String,
    pub attrs: BTreeMap<String, String>,
    pub parent: usize,
    pub text_start: usize,
    pub text_end: usize,
    pub raw_text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map_or("", String::as_str)
    }

    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class").split_whitespace().any(|c| c == class)
    }
}

#[derive(Default)]
pub(crate) struct Document {
    pub nodes: Vec<Element>,
    pub text_buf: String,
}

impl Document {
    pub fn parse(html: &str) -> Self {
        Self::parse_inner(html, false).expect("non-strict HTML parse is infallible")
    }

    pub fn parse_strict(html: &str) -> Result<Self> {
        Self::parse_inner(html, true)
    }

    fn parse_inner(html: &str, strict: bool) -> Result<Self> {
        let mut doc = Self {
            nodes: vec![Element::default()],
            text_buf: String::new(),
        };
        let mut stack = vec![0];
        for token in html_tokens(html) {
            match token {
                Token::StartTag(tag) => {
                    let name = String::from_utf8_lossy(&tag.name).into_owned();
                    let void = matches!(
                        name.as_str(),
                        "area"
                            | "base"
                            | "br"
                            | "col"
                            | "embed"
                            | "hr"
                            | "img"
                            | "input"
                            | "link"
                            | "meta"
                            | "param"
                            | "source"
                            | "track"
                            | "wbr"
                    );
                    if matches!(name.as_str(), "a" | "p" | "li" | "dt" | "dd") {
                        close_html_element(&mut stack, &mut doc, &name);
                    }
                    if doc.nodes.len() > MAX_DOM_NODES {
                        if strict {
                            bail!("HTML node count exceeds {MAX_DOM_NODES} limit");
                        }
                        break;
                    }
                    if stack.len() > MAX_DOM_DEPTH {
                        if strict {
                            bail!("HTML nesting depth exceeds {MAX_DOM_DEPTH} limit");
                        }
                        continue;
                    }
                    let id = doc.nodes.len();
                    let text_pos = doc.text_buf.len();
                    doc.nodes.push(Element {
                        tag: name,
                        attrs: tag
                            .attributes
                            .into_iter()
                            .map(|(k, v)| {
                                (
                                    String::from_utf8_lossy(&k).into_owned(),
                                    String::from_utf8_lossy(&v).into_owned(),
                                )
                            })
                            .collect(),
                        parent: *stack.last().unwrap(),
                        text_start: text_pos,
                        text_end: text_pos,
                        raw_text: String::new(),
                    });
                    if !void && !tag.self_closing {
                        stack.push(id);
                    }
                }
                Token::EndTag(tag) => {
                    close_html_element(&mut stack, &mut doc, &String::from_utf8_lossy(&tag.name))
                }
                Token::String(s) => {
                    let text = String::from_utf8_lossy(&s);
                    if let Some(&id) = stack.last() {
                        if matches!(doc.nodes[id].tag.as_str(), "script" | "style") {
                            doc.nodes[id].raw_text.push_str(&text);
                            continue;
                        }
                    }
                    doc.text_buf.push_str(&text);
                }
                _ => {}
            }
        }
        let end = doc.text_buf.len();
        for &id in &stack {
            doc.nodes[id].text_end = end;
        }
        Ok(doc)
    }

    pub fn text(&self, id: usize) -> &str {
        let e = &self.nodes[id];
        if !e.raw_text.is_empty() || matches!(e.tag.as_str(), "script" | "style") {
            &e.raw_text
        } else {
            &self.text_buf[e.text_start..e.text_end]
        }
    }

    pub fn text_len(&self, id: usize) -> usize {
        self.text(id).len()
    }

    pub fn children(&self, parent: usize) -> impl Iterator<Item = (usize, &Element)> {
        self.nodes
            .iter()
            .enumerate()
            .skip(parent + 1)
            .filter(move |(_, e)| e.parent == parent)
    }

    pub fn descendants(&self, root: usize) -> impl Iterator<Item = (usize, &Element)> {
        self.nodes
            .iter()
            .enumerate()
            .skip(root + 1)
            .take_while(move |(_, e)| e.parent >= root)
    }

    pub fn meta(&self, names: &[&str]) -> Option<&str> {
        names.iter().find_map(|name| {
            self.nodes
                .iter()
                .find(|e| {
                    e.tag == "meta"
                        && ["name", "property", "itemprop"]
                            .iter()
                            .any(|key| e.attr(key).eq_ignore_ascii_case(name))
                        && !e.attr("content").trim().is_empty()
                })
                .map(|e| e.attr("content").trim())
        })
    }

    pub fn class_text(&self, name: &str) -> Option<&str> {
        self.nodes
            .iter()
            .enumerate()
            .find(|(_, e)| e.has_class(name))
            .map(|(id, _)| self.text(id))
    }
}

fn close_html_element(stack: &mut Vec<usize>, doc: &mut Document, name: &str) {
    if let Some(index) = stack.iter().rposition(|&id| {
        let tag = doc.nodes[id].tag.as_str();
        tag == name || (matches!(name, "dt" | "dd") && matches!(tag, "dt" | "dd"))
    }) {
        let end = doc.text_buf.len();
        for &id in &stack[index..] {
            doc.nodes[id].text_end = end;
        }
        stack.truncate(index);
    }
}

fn xml_element(
    start: &BytesStart<'_>,
    reader: &Reader<&[u8]>,
    parent: usize,
    text_pos: usize,
) -> Result<Element> {
    let attrs = start
        .attributes()
        .with_checks(false)
        .map(|attr| {
            let attr = attr?;
            let name = String::from_utf8_lossy(attr.key.local_name().as_ref()).into_owned();
            Ok((
                name,
                attr.decode_and_unescape_value(reader.decoder())?
                    .into_owned(),
            ))
        })
        .collect::<Result<_>>()?;
    Ok(Element {
        tag: String::from_utf8_lossy(start.local_name().as_ref()).into_owned(),
        attrs,
        parent,
        text_start: text_pos,
        text_end: text_pos,
        raw_text: String::new(),
    })
}

fn parse_xml(bytes: &[u8]) -> Result<Document> {
    let mut reader = Reader::from_reader(bytes);
    let mut doc = Document {
        nodes: vec![Element::default()],
        text_buf: String::new(),
    };
    let mut stack = vec![0];
    loop {
        match reader.read_event()? {
            Event::Start(node) => {
                ensure!(
                    doc.nodes.len() <= MAX_DOM_NODES,
                    "XML node count exceeds {MAX_DOM_NODES} limit"
                );
                ensure!(
                    stack.len() <= MAX_DOM_DEPTH,
                    "XML nesting depth exceeds {MAX_DOM_DEPTH} limit"
                );
                let text_pos = doc.text_buf.len();
                let elem = xml_element(&node, &reader, *stack.last().unwrap(), text_pos)?;
                doc.nodes.push(elem);
                stack.push(doc.nodes.len() - 1);
            }
            Event::Empty(node) => {
                ensure!(
                    doc.nodes.len() <= MAX_DOM_NODES,
                    "XML node count exceeds {MAX_DOM_NODES} limit"
                );
                ensure!(
                    stack.len() <= MAX_DOM_DEPTH,
                    "XML nesting depth exceeds {MAX_DOM_DEPTH} limit"
                );
                let text_pos = doc.text_buf.len();
                doc.nodes.push(xml_element(
                    &node,
                    &reader,
                    *stack.last().unwrap(),
                    text_pos,
                )?);
            }
            Event::End(_) => {
                let id = stack.pop().context("unexpected XML end tag")?;
                doc.nodes[id].text_end = doc.text_buf.len();
                ensure!(!stack.is_empty(), "unbalanced XML root end tag");
            }
            Event::Text(s) => {
                let text = s.unescape()?;
                doc.text_buf.push_str(&text);
            }
            Event::CData(s) => {
                let text = s.decode()?;
                doc.text_buf.push_str(&text);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    doc.nodes[0].text_end = doc.text_buf.len();
    ensure!(stack.len() == 1 && doc.nodes.len() > 1, "not XML");
    Ok(doc)
}

pub struct SourceContext {
    pub window: Window,
    pub now: DateTime<Utc>,
    pub lang: Lang,
    pub use_state: bool,
    pub catchup: bool,
    pub daily_board_state: bool,
    pub wait_arxiv: Duration,
    pub summary_chars: usize,
    pub state: Mutex<FetchState>,
    pub notes: Mutex<Vec<String>>,
    pub http: SafeHttpClient,
    github_origin: Url,
    github_token: Option<String>,
    github_warned: AtomicBool,
    reddit: tokio::sync::Mutex<()>,
}

impl SourceContext {
    pub fn new(window: Window, settings: &Settings, lang: Lang, state: FetchState) -> Self {
        Self {
            window,
            now: Utc::now(),
            lang,
            use_state: true,
            catchup: false,
            daily_board_state: true,
            wait_arxiv: Duration::ZERO,
            summary_chars: settings.summary_chars,
            state: Mutex::new(state),
            notes: Mutex::new(Vec::new()),
            http: SafeHttpClient::new(
                Duration::from_secs(settings.timeout),
                2,
                &settings.user_agent,
            ),
            github_origin: Url::parse("https://api.github.com/").unwrap(),
            github_token: ["GITHUB_TOKEN", "GH_TOKEN"]
                .into_iter()
                .filter_map(|k| std::env::var(k).ok())
                .map(|s| s.trim().to_owned())
                .find(|s| !s.is_empty()),
            github_warned: AtomicBool::new(false),
            reddit: tokio::sync::Mutex::new(()),
        }
    }

    pub fn with_github_token(mut self, token: Option<String>) -> Self {
        self.github_token = token.filter(|token| !token.trim().is_empty());
        self
    }

    fn note(&self, note: String) {
        self.notes.lock().unwrap().push(note);
    }

    fn date(&self, date: Option<DateTime<FixedOffset>>) -> Option<String> {
        date.map(|d| self.window.iso(d))
    }

    async fn response(
        &self,
        url: &Url,
        accept: &str,
        user_agent: Option<&str>,
    ) -> Result<reqwest::Response> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_str(accept)?);
        if let Some(ua) = user_agent {
            headers.insert(reqwest::header::USER_AGENT, HeaderValue::from_str(ua)?);
        }
        for attempt in 0..3 {
            let response = self.http.get_with_headers(url, None, &headers).await?;
            if attempt == 2
                || !(response.status() == StatusCode::TOO_MANY_REQUESTS
                    || response.status().is_server_error())
            {
                return Ok(response);
            }
            tokio::time::sleep(Duration::from_secs(2 + 3 * attempt)).await;
        }
        unreachable!("the last attempt always returns")
    }

    async fn json(&self, url: &Url) -> Result<Value> {
        let resp = require_ok(self.response(url, "application/json", None).await?)?;
        let bytes = read_bytes_limited(resp, MAX_BODY_BYTES).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn html(&self, url: &Url, ua: Option<&str>) -> Result<Document> {
        let response = require_ok(self.response(url, "text/html", ua).await?)?;
        let text = read_text_limited(response, MAX_BODY_BYTES).await?;
        Document::parse_strict(&text)
    }

    async fn gh_api(&self, path: &str, params: &[(&str, &str)]) -> Result<Value> {
        let url = endpoint(self.github_origin.join(path)?, params);
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "x-github-api-version",
            HeaderValue::from_static("2022-11-28"),
        );
        let bearer = self
            .github_token
            .as_deref()
            .map(|token| (&self.github_origin, token));
        for attempt in 0..2 {
            let response = self.http.get_with_headers(&url, bearer, &headers).await?;
            let code = response.status();
            if matches!(code.as_u16(), 403 | 429) {
                let remaining = response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .is_some_and(|r| r == "0");
                let body = read_text_limited(response, 64 * 1024)
                    .await
                    .unwrap_or_default();
                let body = short(&body, 300).to_ascii_lowercase();
                let limited = code.as_u16() == 429
                    || remaining
                    || body.contains("rate limit")
                    || body.contains("secondary rate");
                let authenticated = bearer.is_some();
                if !self.github_warned.swap(true, Ordering::Relaxed) {
                    self.note(fmt_label(
                        self.lang,
                        if authenticated {
                            "gh_rate_auth"
                        } else {
                            "gh_rate_unauth"
                        },
                        &[("code", code.as_str())],
                    ));
                }
                let detail = if limited {
                    "rate limit"
                } else {
                    "forbidden/rate limit"
                };
                let hint = if authenticated {
                    "check GITHUB_TOKEN/GH_TOKEN"
                } else {
                    "set GITHUB_TOKEN or GH_TOKEN"
                };
                bail!("HTTP {} ({detail}; {hint})", code.as_u16());
            }
            if attempt == 0 && code.is_server_error() {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            let bytes = read_bytes_limited(require_ok(response)?, MAX_BODY_BYTES).await?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
        unreachable!("the last attempt always returns")
    }
}

fn require_ok(response: reqwest::Response) -> Result<reqwest::Response> {
    ensure!(
        response.status() == StatusCode::OK,
        "HTTP {}",
        response.status().as_u16()
    );
    Ok(response)
}

fn endpoint(mut url: Url, params: &[(&str, &str)]) -> Url {
    if !params.is_empty() {
        url.query_pairs_mut().extend_pairs(params);
    }
    url
}

fn source_url(source: &SourceSpec) -> Result<Url> {
    Ok(Url::parse(
        source.url.as_deref().context("missing source URL")?,
    )?)
}

fn candidate(
    ctx: &SourceContext,
    src: &SourceSpec,
    title: impl Into<String>,
    url: impl Into<String>,
    date: Option<DateTime<FixedOffset>>,
    summary: impl Into<String>,
) -> Cand {
    Cand {
        title: title.into(),
        url: url.into(),
        source: src.id.clone(),
        channel: src.channel.clone(),
        kind: src.kind.clone(),
        published: ctx.date(date),
        summary: summary.into(),
        ..Cand::default()
    }
}

fn extra(c: &mut Cand, data: Value) {
    c.extra = data.as_object().expect("candidate metadata object").clone();
}

pub async fn fetch_source(ctx: &SourceContext, source: &SourceSpec) -> Result<Vec<Cand>> {
    let mut rows = match source.source_type.as_str() {
        "rss" => feed(ctx, source, false).await?,
        "html_links" => html_links(ctx, source).await?,
        "arxiv" => arxiv(ctx, source).await?,
        "hf_daily" => hf_daily(ctx, source).await?,
        "hf_org_models" => hf_models(ctx, source, true).await?,
        "hf_trending" => hf_models(ctx, source, false).await?,
        "github_trending" => github_trending(ctx, source).await?,
        "github_releases" => github_releases(ctx, source).await?,
        "github_org_repos" => github_repositories(ctx, source, true).await?,
        "github_search" => github_repositories(ctx, source, false).await?,
        "hn" => hn(ctx, source).await?,
        "reddit" => feed(ctx, source, true).await?,
        ty => bail!("unsupported source type {ty}"),
    };
    if source.filter.as_deref() == Some("ai") {
        rows.retain(ai_ok);
    }
    Ok(rows)
}

async fn feed(ctx: &SourceContext, source: &SourceSpec, reddit: bool) -> Result<Vec<Cand>> {
    let base = source_url(source)?;
    let response = if reddit {
        let _guard = ctx.reddit.lock().await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        require_ok(
            ctx.response(&base, "application/atom+xml", source.user_agent.as_deref())
                .await?,
        )?
    } else {
        require_ok(
            ctx.response(&base, RSS_ACCEPT, source.user_agent.as_deref())
                .await?,
        )?
    };
    let bytes = read_bytes_limited(response, MAX_BODY_BYTES).await?;
    let doc = parse_xml(&bytes)?;
    let entries = feed_entries(&doc);
    ensure!(
        reddit || !entries.is_empty(),
        "feed has 0 items (dead or format changed)"
    );
    static SUBMITTED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"submitted by /u/\S+.*$").unwrap());
    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let date = entry.date.filter(|d| ctx.window.contains(*d))?;
            let link = base.join(entry.link).ok()?;
            if entry.link.is_empty()
                || entry.title.is_empty()
                || !matches!(link.scheme(), "http" | "https")
            {
                return None;
            }
            let mut summary = clean_text(entry.summary, ctx.summary_chars);
            if reddit {
                summary = SUBMITTED.replace(&summary, "").trim().into();
            }
            let mut c = candidate(
                ctx,
                source,
                clean_text(entry.title, 300),
                link.as_str(),
                Some(date),
                summary,
            );
            if !reddit {
                c.authors = entry.authors.into_iter().take(6).collect();
            }
            Some(c)
        })
        .collect())
}

#[derive(Default)]
struct FeedEntry<'a> {
    title: &'a str,
    link: &'a str,
    summary: &'a str,
    date: Option<DateTime<FixedOffset>>,
    authors: Vec<String>,
}

fn feed_entries(doc: &Document) -> Vec<FeedEntry<'_>> {
    doc.nodes
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e.tag.as_str(), "item" | "entry"))
        .map(|(id, _)| {
            let mut entry = FeedEntry::default();
            for (child, e) in doc.children(id) {
                let text = doc.text(child).trim();
                match e.tag.as_str() {
                    "title" => entry.title = text,
                    "link" if entry.link.is_empty() => {
                        let (href, rel) = (e.attr("href"), e.attr("rel"));
                        if !href.is_empty() && matches!(rel, "" | "alternate") {
                            entry.link = href;
                        } else if !text.is_empty() {
                            entry.link = text;
                        }
                    }
                    "guid" if entry.link.is_empty() && text.starts_with("http") => {
                        entry.link = text
                    }
                    "description" | "summary" | "encoded" | "content"
                        if entry.summary.is_empty() =>
                    {
                        entry.summary = text
                    }
                    "group" if entry.summary.is_empty() => {
                        if let Some((desc_id, _)) =
                            doc.children(child).find(|(_, e)| e.tag == "description")
                        {
                            entry.summary = doc.text(desc_id).trim();
                        }
                    }
                    "pubDate" | "published" | "date" | "issued" | "updated"
                        if entry.date.is_none() =>
                    {
                        entry.date = parse_dt(text)
                    }
                    "author" | "creator" if !text.is_empty() => {
                        entry.authors.push(clean_text(text, 80))
                    }
                    _ => {}
                }
            }
            entry
        })
        .collect()
}

static JSON_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""datePublished"\s*:\s*"([^"]+)""#).unwrap());

async fn html_links(ctx: &SourceContext, src: &SourceSpec) -> Result<Vec<Cand>> {
    use futures_util::{stream, StreamExt};
    let base = source_url(src)?;
    let doc = ctx.html(&base, src.user_agent.as_deref()).await?;
    let pattern = Regex::new(src.pattern.as_deref().context("missing link pattern")?)?;
    let exclude = src.exclude.as_deref().map(Regex::new).transpose()?;
    let mut found = Map::new();
    let mut dates = HashMap::new();
    for (a_id, a) in doc.nodes.iter().enumerate().filter(|(_, e)| e.tag == "a") {
        let href = a.attr("href").trim();
        if href.is_empty()
            || !pattern.is_match(href)
            || exclude.as_ref().is_some_and(|rx| rx.is_match(href))
        {
            continue;
        }
        let Ok(url) = base.join(href) else { continue };
        let url = url.as_str();
        if !url.starts_with("http") || canon_url(url) == canon_url(base.as_str()) {
            continue;
        }
        let title = clean_text(doc.text(a_id), 200);
        if found
            .get(url)
            .and_then(Value::as_str)
            .is_none_or(|old| old.chars().count() < title.chars().count())
        {
            if let Some(date) = date_from_text(&title)
                .or_else(|| date_from_text(url))
                .or_else(|| date_from_text(short(doc.text(a.parent), 300)))
            {
                dates.insert(url.to_owned(), date);
            }
            found.insert(url.into(), json!(title));
        }
    }
    ensure!(!found.is_empty(), "0 links matched pattern (page changed?)");
    let key = format!("html:{}", src.id);
    let timestamp = ctx.now.min(ctx.window.end.with_timezone(&Utc)).timestamp();
    let (first, new) = {
        let mut state = ctx.state.lock().unwrap();
        let entry = state.0.entry(key).or_insert(Value::Null);
        let mut seen = match entry.take() {
            Value::Object(map) => map,
            Value::Array(list) => list
                .into_iter()
                .filter_map(|v| Some((v.as_str()?.to_owned(), json!(0))))
                .collect(),
            _ => Map::new(),
        };
        let first = seen.is_empty();
        let new: Vec<_> = if first || !ctx.use_state {
            found
                .keys()
                .take(src.first_run_keep.unwrap_or(6))
                .cloned()
                .collect()
        } else {
            found
                .keys()
                .filter(|url| {
                    seen.get(*url)
                        .is_none_or(|v| v.as_i64().unwrap_or(0) > ctx.window.start.timestamp())
                })
                .cloned()
                .collect()
        };
        for url in found.keys() {
            seen.entry(url)
                .or_insert(json!(if first { 0 } else { timestamp }));
        }
        if seen.len() > 3000 {
            let mut entries: Vec<_> = seen.into_iter().collect();
            entries.sort_by_key(|(_, v)| v.as_i64().unwrap_or(0));
            seen = entries.into_iter().rev().take(3000).rev().collect();
        }
        *entry = Value::Object(seen);
        (first, new)
    };
    let results = stream::iter(
        new.into_iter()
            .filter(|url| {
                dates
                    .get(url)
                    .is_none_or(|&d| d >= ctx.window.start - Delta::days(1))
            })
            .take(src.resolve_max.unwrap_or(15)),
    )
    .map(|url| async move {
        let details = if src.resolve {
            match Url::parse(&url) {
                Ok(target) => tokio::time::timeout(
                    Duration::from_secs(12),
                    ctx.html(&target, src.user_agent.as_deref()),
                )
                .await
                .ok()
                .and_then(Result::ok),
                Err(_) => None,
            }
        } else {
            None
        };
        (url, details)
    })
    .buffered(6)
    .collect::<Vec<_>>()
    .await;
    Ok(results.into_iter().filter_map(|(url, doc)| {
        let mut dt = doc.as_ref().and_then(article_date);
        if dt.is_none() {
            if let Some(&hint) = dates.get(&url) {
                if hint <= ctx.window.start - Delta::hours(12) || hint > ctx.window.end + Delta::hours(12) { return None; }
                dt = Some(hint);
            }
        }
        if dt.is_some_and(|d| !ctx.window.contains(d) && (first || !ctx.use_state || d > ctx.window.end)) || (dt.is_none() && first && ctx.use_state) {
            return None;
        }
        let mut title = doc.as_ref().and_then(|d| d.meta(&["og:title", "twitter:title"])).map(|s| clean_text(s, 300)).unwrap_or_default();
        if title.is_empty() {
            title = found.get(&url).and_then(Value::as_str).unwrap_or_default().to_owned();
        }
        if title.is_empty() { title = url.trim_end_matches('/').rsplit('/').next().unwrap_or_default().replace('-', " "); }
        let desc = doc.as_ref().and_then(|d| d.meta(&["og:description", "description", "twitter:description"])).unwrap_or_default();
        let mut c = candidate(ctx, src, title, url, dt, clean_text(desc, ctx.summary_chars));
        extra(&mut c, json!({"date_unknown": dt.is_none(), "baseline_run": first, "date_from": dt.map(|_| "page")}));
        Some(c)
    }).collect())
}

fn article_date(doc: &Document) -> Option<DateTime<FixedOffset>> {
    doc.meta(&[
        "article:published_time",
        "og:published_time",
        "datePublished",
        "publish_date",
        "date",
        "pubdate",
        "parsely-pub-date",
    ])
    .and_then(parse_dt)
    .or_else(|| {
        doc.nodes
            .iter()
            .find(|e| e.tag == "time" && !e.attr("datetime").is_empty())
            .and_then(|e| parse_dt(e.attr("datetime")))
    })
    .or_else(|| {
        doc.nodes
            .iter()
            .enumerate()
            .filter(|(_, e)| e.tag == "script" && e.attr("type") == "application/ld+json")
            .find_map(|(id, _)| parse_dt(JSON_DATE.captures(doc.text(id))?.get(1)?.as_str()))
    })
}

async fn hf_daily(ctx: &SourceContext, src: &SourceSpec) -> Result<Vec<Cand>> {
    let base = source_url(src)?;
    let start = ctx.window.start.with_timezone(&Utc);
    let end = ctx.window.end.with_timezone(&Utc);
    let gap = ctx.use_state && ctx.daily_board_state;
    let age = ctx.now - end;
    let cutoff = if gap && age >= Delta::zero() && age < Delta::hours(12) {
        ctx.now
    } else {
        end
    }
    .min(ctx.now);
    let close = |d: NaiveDate| -> Option<DateTime<Utc>> {
        Some(d.succ_opt()?.and_hms_opt(0, 0, 0)?.and_utc())
    };
    let mut dates = BTreeSet::new();
    let mut cur = start.date_naive().pred_opt();
    while let Some(d) = cur {
        let Some(c) = close(d) else { break };
        if c > cutoff {
            break;
        }
        if start <= c && c < end {
            dates.insert(d);
        }
        cur = d.succ_opt();
    }
    if gap {
        let last = ctx
            .state
            .lock()
            .unwrap()
            .0
            .get("hf_daily:last")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<NaiveDate>().ok());
        if let Some(last) = last {
            let floor = cutoff
                .date_naive()
                .checked_sub_days(Days::new(14))
                .unwrap_or(NaiveDate::MIN);
            let mut cur = last.checked_add_days(Days::new(1)).map(|d| d.max(floor));
            while let Some(d) = cur {
                let Some(c) = close(d) else { break };
                if c > cutoff {
                    break;
                }
                dates.insert(d);
                cur = d.succ_opt();
            }
            dates.retain(|d| *d > last);
        }
    }
    let mut out = Vec::new();
    let mut counts = Vec::new();
    let mut last_fetched = None;
    for day in dates {
        let Some(close_time) = close(day) else {
            continue;
        };
        let day = day.to_string();
        let url = endpoint(base.clone(), &[("date", &day), ("limit", "100")]);
        let response = ctx
            .response(&url, "application/json", src.user_agent.as_deref())
            .await?;
        if response.status() == StatusCode::BAD_REQUEST {
            ctx.note(fmt_label(ctx.lang, "hf_not_open", &[("d", &day)]));
            break;
        }
        let bytes = read_bytes_limited(require_ok(response)?, MAX_BODY_BYTES).await?;
        let data: Value = serde_json::from_slice(&bytes)?;
        let papers = data.as_array().context("expected HF Daily JSON array")?;
        counts.push(format!("{day}({})", papers.len()));
        last_fetched = Some(day.clone());
        let date = close_time.fixed_offset();
        for item in papers {
            let p = &item["paper"];
            let aid = str_field(p, "id");
            if aid.is_empty() {
                continue;
            }
            let title = first_value(str_field(p, "title"), str_field(item, "title"));
            let summary = first_value(str_field(p, "ai_summary"), str_field(p, "summary"));
            let mut c = candidate(
                ctx,
                src,
                clean_text(title, 0),
                format!("https://huggingface.co/papers/{aid}"),
                Some(date),
                clean_text(summary, ctx.summary_chars),
            );
            c.authors = array(&p["authors"])
                .iter()
                .take(12)
                .map(|a| str_field(a, "name").to_owned())
                .collect();
            extra(
                &mut c,
                json!({"arxiv_id": aid, "github": p["githubRepo"], "project": p["projectPage"], "org": p["organization"]["fullname"], "hf_date": day}),
            );
            out.push(c);
        }
    }
    if let Some(day) = last_fetched.filter(|_| gap) {
        ctx.state
            .lock()
            .unwrap()
            .0
            .insert("hf_daily:last".into(), json!(day));
    }
    ctx.note(if counts.is_empty() {
        ft(ctx.lang, "hf_none").into()
    } else {
        format!("{}{}", ft(ctx.lang, "hf_utc"), counts.join(", "))
    });
    Ok(out)
}

fn first_value<'a>(first: &'a str, second: &'a str) -> &'a str {
    if first.is_empty() {
        second
    } else {
        first
    }
}

async fn hf_models(ctx: &SourceContext, src: &SourceSpec, org_mode: bool) -> Result<Vec<Cand>> {
    use futures_util::{stream, StreamExt};
    let results = if org_mode {
        stream::iter(src.orgs.iter())
            .map(|org| async move {
                let url = endpoint(
                    Url::parse("https://huggingface.co/api/models").unwrap(),
                    &[
                        ("author", org),
                        ("sort", "createdAt"),
                        ("direction", "-1"),
                        ("limit", "25"),
                        ("full", "false"),
                    ],
                );
                (org.as_str(), ctx.json(&url).await)
            })
            .buffered(12)
            .collect::<Vec<_>>()
            .await
    } else {
        vec![("", Ok(ctx.json(&source_url(src)?).await?))]
    };
    let mut errors = Vec::new();
    let mut out = Vec::new();
    let cutoff = ctx.window.end - Delta::days(src.max_age_days.unwrap_or(21).into());
    let kind_path = match src.kind.as_str() {
        "dataset" => "datasets/",
        _ if src
            .url
            .as_deref()
            .is_some_and(|url| url.contains("/spaces")) =>
        {
            "spaces/"
        }
        _ => "",
    };
    for (org, result) in results {
        let data = match result {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{org}: {e}"));
                continue;
            }
        };
        for m in data.as_array().context("expected HF models JSON array")? {
            let Some(dt) = parse_dt(str_field(m, "createdAt")) else {
                continue;
            };
            if (org_mode && !ctx.window.contains(dt))
                || (!org_mode && (dt < cutoff || dt > ctx.window.end))
            {
                continue;
            }
            let id = first_value(str_field(m, "id"), str_field(m, "modelId"));
            if id.is_empty() || DERIV_RE.is_match(id) {
                continue;
            }
            let tags = array(&m["tags"])
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| {
                    !s.starts_with("region:")
                        && !s.starts_with("endpoints")
                        && (!org_mode || !s.starts_with("autotrain"))
                })
                .take(if org_mode { 12 } else { 10 })
                .collect::<Vec<_>>()
                .join(", ");
            let pipeline = first_value(str_field(m, "pipeline_tag"), "None");
            let desc = if org_mode {
                format!(
                    "pipeline={pipeline}; library={}; tags={tags}",
                    first_value(str_field(m, "library_name"), "None")
                )
            } else {
                clean_text(
                    first_value(
                        str_field(&m["cardData"], "short_description"),
                        &format!("pipeline={pipeline}; tags={tags}"),
                    ),
                    300,
                )
            };
            let mut c = candidate(
                ctx,
                src,
                id,
                format!("https://huggingface.co/{kind_path}{id}"),
                Some(dt),
                desc,
            );
            extra(
                &mut c,
                if org_mode {
                    json!({"org": org, "pipeline": m["pipeline_tag"]})
                } else {
                    json!({"created_in_window": ctx.window.contains(dt), "discovery_only": true})
                },
            );
            out.push(c);
        }
    }
    record_errors(ctx, "hf_org_models", &errors, 8);
    Ok(out)
}

fn record_errors(ctx: &SourceContext, label: &str, errors: &[String], limit: usize) {
    if !errors.is_empty() {
        ctx.note(format!(
            "{label} errors ({}): {}",
            errors.len(),
            errors[..errors.len().min(limit)].join("; ")
        ));
    }
}

async fn github_trending(ctx: &SourceContext, src: &SourceSpec) -> Result<Vec<Cand>> {
    let mut out = Vec::new();
    for url in &src.urls {
        let doc = match ctx.html(&Url::parse(url)?, src.user_agent.as_deref()).await {
            Ok(doc) => doc,
            Err(e) => {
                ctx.note(format!("gh_trending {url}: {e}"));
                continue;
            }
        };
        for (id, _) in doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, e)| e.tag == "article" && e.has_class("Box-row"))
        {
            let Some((heading, _)) = doc.descendants(id).find(|(_, e)| e.tag == "h2") else {
                continue;
            };
            let Some((_, a)) = doc.descendants(heading).find(|(_, e)| e.tag == "a") else {
                continue;
            };
            let repo: String = a
                .attr("href")
                .trim_matches('/')
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if repo.is_empty() {
                continue;
            }
            let desc = doc
                .descendants(id)
                .find(|(_, e)| e.tag == "p")
                .map(|(p_id, _)| clean_text(doc.text(p_id), 0))
                .unwrap_or_default();
            let language = doc
                .descendants(id)
                .find(|(_, e)| e.attr("itemprop") == "programmingLanguage")
                .map(|(l_id, _)| clean_text(doc.text(l_id), 0));
            let mut c = candidate(
                ctx,
                src,
                &repo,
                format!("https://github.com/{repo}"),
                Some(ctx.window.end - Delta::minutes(1)),
                desc,
            );
            extra(
                &mut c,
                json!({"language": language, "trending_list": url.split_once("github.com/").map_or(url.as_str(), |(_, p)| p), "discovery_only": true}),
            );
            out.push(c);
        }
    }
    Ok(out)
}

async fn github_releases(ctx: &SourceContext, src: &SourceSpec) -> Result<Vec<Cand>> {
    use futures_util::{stream, StreamExt};
    let results = stream::iter(src.repos.iter())
        .map(|repo| async move {
            let result = ctx
                .gh_api(&format!("repos/{repo}/releases"), &[("per_page", "30")])
                .await;
            (repo, result)
        })
        .buffered(8)
        .collect::<Vec<_>>()
        .await;
    let mut errors = Vec::new();
    let mut out = Vec::new();
    for (repo, result) in results {
        let result = match result {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{repo}: {e}"));
                continue;
            }
        };
        let mut releases = Vec::new();
        for release in array(&result) {
            let dt = parse_dt(first_value(
                str_field(release, "published_at"),
                str_field(release, "created_at"),
            ));
            if !dt.is_some_and(|d| ctx.window.contains(d)) || release["draft"] == true {
                continue;
            }
            let tag = str_field(release, "tag_name");
            let title = first_value(str_field(release, "name"), tag);
            if repo != "ggml-org/llama.cpp"
                && (release["prerelease"] == true
                    || PRERELEASE_RE.is_match(&format!("{title} {tag}")))
            {
                continue;
            }
            if title.is_empty() {
                continue;
            }
            let link = str_field(release, "html_url");
            let mut c = candidate(
                ctx,
                src,
                format!("{repo} {}", clean_text(title, 120)),
                first_value(link, &format!("https://github.com/{repo}/releases")),
                dt,
                clean_text(str_field(release, "body"), ctx.summary_chars),
            );
            extra(&mut c, json!({"repo": repo}));
            releases.push(c);
        }
        releases.sort_by_key(|c| std::cmp::Reverse(c.published.clone()));
        let count = releases.len();
        if let Some(mut c) = releases.into_iter().next() {
            if count > 1 {
                c.extra.insert("releases_in_window".into(), json!(count));
            }
            out.push(c);
        }
    }
    record_errors(ctx, "gh_releases", &errors, 10);
    if !src.repos.is_empty() && errors.len() == src.repos.len() {
        bail!("{}", errors[0]);
    }
    Ok(out)
}

async fn github_repositories(
    ctx: &SourceContext,
    src: &SourceSpec,
    org_mode: bool,
) -> Result<Vec<Cand>> {
    use futures_util::{stream, StreamExt};
    let mut results = Vec::new();
    if org_mode {
        results = stream::iter(src.orgs.iter())
            .map(|org| async move {
                (
                    org.clone(),
                    ctx.gh_api(
                        &format!("orgs/{org}/repos"),
                        &[
                            ("sort", "created"),
                            ("direction", "desc"),
                            ("per_page", "8"),
                        ],
                    )
                    .await,
                )
            })
            .buffered(6)
            .collect()
            .await;
    } else {
        let since = (ctx.window.end - Delta::days(3))
            .with_timezone(&Utc)
            .date_naive()
            .to_string();
        for (i, query) in src.queries.iter().enumerate() {
            let query = query.replace("{since3}", &since);
            results.push((
                format!("q{}", i + 1),
                ctx.gh_api(
                    "search/repositories",
                    &[
                        ("q", &query),
                        ("sort", "stars"),
                        ("order", "desc"),
                        ("per_page", "50"),
                    ],
                )
                .await,
            ));
            if i + 1 < src.queries.len() {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    let count = results.len();
    for (name, result) in results {
        let value = match result {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        let list = if org_mode {
            array(&value)
        } else {
            array(&value["items"])
        };
        for it in list {
            let title = str_field(it, "full_name");
            let url = str_field(it, "html_url");
            if title.is_empty() || url.is_empty() {
                continue;
            }
            let dt = parse_dt(str_field(it, "created_at"));
            let desc = str_field(it, "description");
            let tags: Vec<_> = array(&it["topics"])
                .iter()
                .filter_map(Value::as_str)
                .take(8)
                .collect();
            if org_mode {
                if !dt.is_some_and(|d| ctx.window.contains(d)) || it["fork"] == true {
                    continue;
                }
            } else {
                let owner = title.split('/').next().unwrap_or_default();
                let key = norm_title(desc);
                if SPAM_OWNER_RE.is_match(owner)
                    || (!key.is_empty() && !seen.insert(short(&key, 40).to_owned()))
                {
                    continue;
                }
                if !AI_RE.is_match(&format!("{title} {} {}", tags.join(" "), short(desc, 300))) {
                    continue;
                }
            }
            let mut c = candidate(ctx, src, title, url, dt, clean_text(desc, 400));
            extra(
                &mut c,
                if org_mode {
                    json!({"org": name})
                } else {
                    json!({"language": it["language"], "topics": tags, "discovery_only": true})
                },
            );
            out.push(c);
        }
    }
    record_errors(
        ctx,
        if org_mode {
            "gh_org_repos"
        } else {
            "gh_search"
        },
        &errors,
        if org_mode { 6 } else { 4 },
    );
    if count > 0 && count == errors.len() {
        bail!("{}", errors[0]);
    }
    Ok(out)
}

async fn hn(ctx: &SourceContext, src: &SourceSpec) -> Result<Vec<Cand>> {
    let base = Url::parse(
        src.url
            .as_deref()
            .unwrap_or("https://hn.algolia.com/api/v1/search_by_date"),
    )?;
    let filter = format!(
        "created_at_i>={},created_at_i<{},points>={}",
        ctx.window.start.timestamp(),
        ctx.window.end.timestamp(),
        src.min_points.unwrap_or(25)
    );
    let mut out = Vec::new();
    for page in 0..10 {
        let url = endpoint(
            base.clone(),
            &[
                ("tags", "story"),
                ("numericFilters", &filter),
                ("hitsPerPage", "500"),
                ("page", &page.to_string()),
            ],
        );
        let value = ctx.json(&url).await?;
        for h in array(&value["hits"]) {
            let id = str_field(h, "objectID");
            let discussion = format!("https://news.ycombinator.com/item?id={id}");
            let url = first_value(str_field(h, "url"), &discussion);
            let title = clean_text(str_field(h, "title"), 0);
            let host = Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            let date = h["created_at_i"]
                .as_i64()
                .and_then(|n| DateTime::from_timestamp(n, 0))
                .map(|dt| dt.fixed_offset());
            if !date.is_some_and(|d| ctx.window.contains(d))
                || !AI_RE.is_match(&format!("{title} {host}"))
            {
                continue;
            }
            let mut c = candidate(
                ctx,
                src,
                title,
                url,
                date,
                clean_text(str_field(h, "story_text"), 300),
            );
            extra(&mut c, json!({"hn": discussion}));
            out.push(c);
        }
        if page + 1 >= value["nbPages"].as_u64().unwrap_or(0) {
            break;
        }
    }
    Ok(out)
}

static ARXIV_DAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:mon|tue|wed|thu|fri|sat|sun)\w*,\s*(\d{1,2} [a-z]+ \d{4})").unwrap()
});
static ARXIV_ABS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/abs/(\d{4}\.\d{4,5})(?:v\d+)?\b").unwrap());

fn listing_day(text: &str) -> Option<NaiveDate> {
    let text = ARXIV_DAY.captures(text)?.get(1)?.as_str();
    NaiveDate::parse_from_str(text, "%d %B %Y")
        .or_else(|_| NaiveDate::parse_from_str(text, "%d %b %Y"))
        .ok()
}

fn announce(day: NaiveDate) -> Option<DateTime<FixedOffset>> {
    let prev = day.pred_opt()?;
    let dt = prev.and_hms_opt(20, 0, 0)?;
    chrono_tz::America::New_York
        .from_local_datetime(&dt)
        .earliest()
        .map(|d| d.fixed_offset())
}

fn listing_dates(doc: &Document) -> BTreeSet<NaiveDate> {
    doc.nodes
        .iter()
        .enumerate()
        .filter(|(_, e)| e.tag == "h3")
        .filter_map(|(id, _)| listing_day(doc.text(id)))
        .collect()
}

#[derive(Default)]
struct Paper {
    title: String,
    authors: Vec<String>,
    subjects: String,
    abstract_text: String,
    categories: Vec<String>,
    listing: Option<NaiveDate>,
}

fn listing_papers(doc: &Document, new_day: Option<NaiveDate>) -> Vec<(String, Paper)> {
    let mut rows = Vec::new();
    let preferred = doc
        .nodes
        .iter()
        .any(|e| e.tag == "dl" && e.attr("id") == "articles");
    for (dl, _) in doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, e)| e.tag == "dl" && (!preferred || e.attr("id") == "articles"))
    {
        let mut day = if new_day.is_some() {
            new_day
        } else {
            doc.nodes[..dl]
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, e)| e.tag == "h3")
                .find_map(|(h_id, _)| listing_day(doc.text(h_id)))
        };
        let mut allowed = new_day.is_none();
        let mut aid = None;
        for (id, node) in doc.children(dl) {
            match node.tag.as_str() {
                "h3" => {
                    let text = doc.text(id).to_ascii_lowercase();
                    allowed = new_day.is_none()
                        || (text.contains("new") || text.contains("cross"))
                            && !text.contains("replacement");
                    day = listing_day(&text).or(day);
                }
                "dt" => {
                    aid = doc
                        .descendants(id)
                        .filter(|(_, e)| e.tag == "a")
                        .find_map(|(_, e)| {
                            ARXIV_ABS.captures(e.attr("href")).map(|m| m[1].to_owned())
                        });
                }
                "dd" if allowed => {
                    let Some(aid) = aid.take() else { continue };
                    let field = |class: &str| doc.descendants(id).find(|(_, e)| e.has_class(class));
                    let clean_field = |class: &str, prefix: &str| {
                        field(class)
                            .map(|(f_id, _)| {
                                let text = doc.text(f_id).trim();
                                clean_text(text.strip_prefix(prefix).unwrap_or(text), 0)
                            })
                            .unwrap_or_default()
                    };
                    let authors = field("list-authors")
                        .map(|(position, _)| {
                            doc.descendants(position)
                                .filter(|(_, e)| e.tag == "a")
                                .take(12)
                                .map(|(a_id, _)| clean_text(doc.text(a_id), 0))
                                .collect()
                        })
                        .unwrap_or_default();
                    rows.push((
                        aid,
                        Paper {
                            title: clean_field("list-title", "Title:"),
                            subjects: clean_field("list-subjects", "Subjects:"),
                            abstract_text: clean_field("mathjax", ""),
                            authors,
                            listing: day,
                            ..Paper::default()
                        },
                    ));
                }
                _ => {}
            }
        }
    }
    rows
}

async fn arxiv_listing(ctx: &SourceContext, cat: &str, name: &str) -> Result<Document> {
    let url = Url::parse(&format!(
        "https://arxiv.org/list/{cat}/{name}?skip=0&show=2000"
    ))?;
    let timeout = Duration::from_secs(60);
    tokio::time::timeout(timeout, ctx.html(&url, None)).await?
}

async fn listing_pages<'a>(
    ctx: &SourceContext,
    cats: &'a [String],
    name: &str,
) -> Vec<(&'a str, Result<Document>)> {
    use futures_util::{stream, StreamExt};
    stream::iter(cats)
        .map(|cat| async move { (cat.as_str(), arxiv_listing(ctx, cat, name).await) })
        .buffered(4)
        .collect()
        .await
}

async fn arxiv(ctx: &SourceContext, src: &SourceSpec) -> Result<Vec<Cand>> {
    let first = src.categories.first().context("missing arXiv categories")?;
    let mut past = listing_pages(ctx, &src.categories, "pastweek").await;
    let days_from = |entries: &[(&str, Result<Document>)]| {
        entries
            .first()
            .and_then(|(_, result)| result.as_ref().ok())
            .map(listing_dates)
            .unwrap_or_default()
    };
    let mut days = days_from(&past);
    let scan_start = ctx
        .window
        .start
        .date_naive()
        .checked_sub_days(Days::new(1))
        .unwrap_or(NaiveDate::MIN);
    let scan_end = ctx
        .window
        .end
        .date_naive()
        .checked_add_days(Days::new(2))
        .unwrap_or(NaiveDate::MAX);
    let expected: Vec<_> = scan_start
        .iter_days()
        .take_while(|d| *d <= scan_end)
        .filter(|d| {
            d.weekday().num_days_from_monday() < 5
                && announce(*d).is_some_and(|dt| ctx.window.contains(dt))
        })
        .collect();
    let mut new = Vec::new();
    if let Some(&latest) = expected.last() {
        let latest_ann = announce(latest);
        if !days.contains(&latest) {
            let now = ctx.now.fixed_offset();
            let retries = latest_ann.is_some_and(|ann| now >= ann)
                && ctx
                    .window
                    .end
                    .checked_add_signed(Delta::days(1))
                    .is_some_and(|limit| now < limit);
            let wait = if retries {
                ctx.wait_arxiv.max(Duration::from_secs(20))
            } else {
                Duration::ZERO
            };
            let elapsed = std::time::Instant::now();
            loop {
                match arxiv_listing(ctx, first, "new").await {
                    Ok(doc) if listing_dates(&doc).contains(&latest) => {
                        new.push((first.as_str(), Ok(doc)));
                        new.extend(listing_pages(ctx, &src.categories[1..], "new").await);
                        days.insert(latest);
                        ctx.note(fmt_label(
                            ctx.lang,
                            "arxiv_from_new",
                            &[("label", &latest.format("%a %d %b").to_string())],
                        ));
                        break;
                    }
                    Err(e) => ctx.note(format!("arxiv /new check: {e}")),
                    _ => {}
                }
                if elapsed.elapsed() >= wait {
                    break;
                }
                tokio::time::sleep(
                    wait.saturating_sub(elapsed.elapsed())
                        .min(Duration::from_secs(20)),
                )
                .await;
                let fresh = arxiv_listing(ctx, first, "pastweek").await;
                if let Ok(doc) = fresh {
                    days = listing_dates(&doc);
                    past[0] = (first, Ok(doc));
                    if days.contains(&latest) {
                        past.truncate(1);
                        past.extend(listing_pages(ctx, &src.categories[1..], "pastweek").await);
                        break;
                    }
                }
            }
        }
        let label = latest.format("%a %d %b").to_string();
        if !days.contains(&latest) {
            if let Some(ann) = latest_ann {
                let age = (ctx.now.fixed_offset() - ann).num_days();
                ctx.note(if age > 5 {
                    fmt_label(
                        ctx.lang,
                        "arxiv_rolled",
                        &[("label", &label), ("days", &age.to_string())],
                    )
                } else {
                    fmt_label(
                        ctx.lang,
                        "arxiv_missing",
                        &[("label", &label), ("when", &ctx.window.iso(ann))],
                    )
                });
            }
        }
        let missing: Vec<_> = expected[..expected.len() - 1]
            .iter()
            .filter(|d| {
                !days.contains(d)
                    && announce(**d)
                        .is_some_and(|ann| ctx.now.fixed_offset() - ann <= Delta::days(5))
            })
            .map(|d| d.format("%a %d %b").to_string())
            .collect();
        if !missing.is_empty() {
            ctx.note(format!(
                "{}{}",
                ft(ctx.lang, "arxiv_missing_prev"),
                missing.join(", ")
            ));
        }
    }
    let mut ingested: BTreeSet<NaiveDate> = {
        let state = ctx.state.lock().unwrap();
        state
            .0
            .get("arxiv:ingested")
            .map(array)
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str()?.parse().ok())
            .collect()
    };
    let targets: BTreeMap<_, _> = days
        .iter()
        .filter_map(|&day| {
            let dt = announce(day)?;
            if ctx.window.contains(dt) {
                Some((day, "window"))
            } else if ctx.catchup
                && ctx.use_state
                && !ingested.is_empty()
                && !ingested.contains(&day)
                && dt < ctx.window.start
                && ctx
                    .window
                    .start
                    .checked_sub_signed(Delta::days(4))
                    .is_some_and(|floor| dt > floor)
            {
                Some((day, "catchup"))
            } else {
                None
            }
        })
        .collect();
    if targets.is_empty() {
        if past.iter().all(|(_, result)| result.is_err()) && new.is_empty() {
            bail!(
                "arXiv listing requests failed: {}",
                past[0].1.as_ref().err().unwrap()
            );
        }
        ctx.note(
            days.last()
                .and_then(|d| Some((*d, announce(*d)?)))
                .map_or_else(
                    || "arxiv: no listing".into(),
                    |(d, ann)| {
                        format!(
                            "arxiv: no announcement in window; latest listing {} announced {}",
                            d.format("%a %d %b"),
                            ctx.window.iso(ann)
                        )
                    },
                ),
        );
        return Ok(Vec::new());
    }
    let latest = days.last().copied();
    let mut papers = BTreeMap::<String, Paper>::new();
    for (dataset, is_new) in [(&new, true), (&past, false)] {
        for (cat, doc) in dataset {
            let doc = match doc {
                Ok(doc) => doc,
                Err(e) => {
                    ctx.note(format!("arxiv {cat}: {e}"));
                    continue;
                }
            };
            if is_new && !latest.is_some_and(|d| listing_dates(doc).contains(&d)) {
                continue;
            }
            for (id, mut paper) in listing_papers(doc, if is_new { latest } else { None }) {
                if !paper.listing.is_some_and(|d| targets.contains_key(&d)) {
                    continue;
                }
                paper.categories.clear();
                let entry = papers.entry(id).or_insert(paper);
                if !entry.categories.iter().any(|c| c == cat) {
                    entry.categories.push((*cat).into());
                }
            }
        }
    }
    if new.is_empty() && targets.last_key_value().map(|(d, _)| *d) == latest {
        for (_, doc) in listing_pages(ctx, &src.categories, "new").await {
            if let Ok(doc) = doc {
                for (id, row) in listing_papers(&doc, latest) {
                    if let Some(p) = papers.get_mut(&id).filter(|p| p.abstract_text.is_empty()) {
                        p.abstract_text = row.abstract_text;
                    }
                }
            }
        }
    }
    let missing: Vec<_> = papers
        .iter()
        .filter(|(_, m)| m.abstract_text.is_empty())
        .map(|(id, _)| id.clone())
        .collect();
    for (i, chunk) in missing.chunks(150).enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        let url = endpoint(
            Url::parse("https://export.arxiv.org/api/query")?,
            &[
                ("id_list", &chunk.join(",")),
                ("max_results", &chunk.len().to_string()),
            ],
        );
        let fetched = async {
            let response = require_ok(ctx.response(&url, RSS_ACCEPT, None).await?)?;
            let bytes = read_bytes_limited(response, MAX_BODY_BYTES).await?;
            parse_xml(&bytes)
        }
        .await;
        let doc = match fetched {
            Ok(doc) => doc,
            Err(e) => {
                ctx.note(format!("arxiv export API chunk {i}: {e}"));
                continue;
            }
        };
        for (i, _) in doc
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, e)| e.tag == "entry")
        {
            let id = doc
                .children(i)
                .find(|(_, e)| e.tag == "id")
                .and_then(|(id_node, _)| ARXIV_ID.captures(doc.text(id_node)));
            let summary = doc.children(i).find(|(_, e)| e.tag == "summary");
            if let (Some(id), Some((sum_node, _))) = (id, summary) {
                if let Some(p) = papers.get_mut(&id[1]) {
                    p.abstract_text = clean_text(doc.text(sum_node), 0);
                }
            }
        }
    }
    ingested.extend(targets.keys());
    let stored: Vec<_> = ingested
        .iter()
        .rev()
        .take(60)
        .rev()
        .map(ToString::to_string)
        .collect();
    ctx.state
        .lock()
        .unwrap()
        .0
        .insert("arxiv:ingested".into(), json!(stored));
    ctx.note(format!(
        "arxiv batches: {}",
        targets
            .iter()
            .filter_map(|(d, why)| {
                let ann = announce(*d)?;
                Some(format!(
                    "{d}({why}, {})",
                    fmt_label(ctx.lang, "arxiv_pub", &[("when", &ctx.window.iso(ann))])
                ))
            })
            .collect::<Vec<_>>()
            .join(", ")
    ));
    Ok(papers
        .into_iter()
        .map(|(id, p)| {
            let day = p.listing.unwrap();
            let mut c = candidate(
                ctx,
                src,
                first_value(&p.title, &id),
                format!("https://arxiv.org/abs/{id}"),
                announce(day),
                clean_text(&p.abstract_text, ctx.summary_chars),
            );
            c.authors = p.authors;
            extra(
                &mut c,
                json!({"arxiv_id": id, "cats": p.categories, "subjects": p.subjects, "listing": day, "batch": targets[&day]}),
            );
            c
        })
        .collect())
}

#[cfg(test)]
mod parser_limit_tests {
    use super::*;

    #[test]
    fn html_and_xml_parsers_enforce_depth_node_limits_and_linear_text_storage() {
        let deep_open = "<div>".repeat(MAX_DOM_DEPTH + 10);
        let deep_close = "</div>".repeat(MAX_DOM_DEPTH + 10);
        let deep_html = format!("{deep_open}hello world{deep_close}");
        assert!(Document::parse_strict(&deep_html).is_err());

        // Non-strict parse must stay bounded and avoid O(depth) text duplication.
        let doc = Document::parse(&deep_html);
        assert!(doc.nodes.len() <= MAX_DOM_DEPTH + 2);
        assert_eq!(doc.text_buf, "hello world");

        let ok_open = "<div>".repeat(64);
        let ok_close = "</div>".repeat(64);
        let ok_html = format!("{ok_open}<p>linear text</p>{ok_close}");
        let doc = Document::parse_strict(&ok_html).unwrap();
        assert_eq!(doc.text_buf, "linear text");
        assert_eq!(doc.text(1), "linear text");

        let deep_xml = format!(
            "{}payload{}",
            "<a>".repeat(MAX_DOM_DEPTH + 5),
            "</a>".repeat(MAX_DOM_DEPTH + 5)
        );
        assert!(parse_xml(deep_xml.as_bytes()).is_err());

        let too_many_xml = format!("<r>{}</r>", "<x/>".repeat(MAX_DOM_NODES + 5));
        assert!(parse_xml(too_many_xml.as_bytes()).is_err());

        let too_many_html = "<span></span>".repeat(MAX_DOM_NODES + 5);
        assert!(Document::parse_strict(&too_many_html).is_err());
    }
}
