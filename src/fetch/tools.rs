use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Days, Duration as Delta, NaiveDate, Utc};
use clap::Args;
use futures_util::{stream, StreamExt};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};
use url::Url;

use crate::fetch::sources::{
    array, candidate_key, fmt_label, load_candidates_jsonl, parse_dt, short, status, str_field,
    Document, ARXIV_ID,
};
use crate::i18n::{ft, Lang, CHECK_CANDS_STRONG_TIER_WORDS};
use crate::ledger::load_ledger_keys;
use crate::model::{
    atomic_write_text, canon_url, clean_text, norm_title, Cand, DigestPaths, InLedger,
};
use crate::net::{read_text_limited, validate_target, SafeHttpClient, MAX_BODY_BYTES};

#[derive(Args, Debug)]
pub struct CandArgs {
    #[arg(required = true, value_name = "KEY")]
    pub keys: Vec<String>,
    #[arg(long = "dir", value_name = "DIR")]
    pub dirs: Vec<PathBuf>,
    #[arg(long)]
    pub since: Option<NaiveDate>,
    #[arg(long, default_value_t = 500)]
    pub chars: usize,
    #[arg(long, default_value_t = 3)]
    pub max: usize,
    #[arg(long, global = true)]
    pub lang: Option<String>,
}

fn dirs_under(path: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort_unstable();
    dirs
}

fn day_fetch_dirs(path: &Path, archives: bool) -> Vec<PathBuf> {
    let mut dirs = vec![path.join("fetch"), path.join("fetch-extra")];
    if archives {
        dirs.extend(dirs_under(path).into_iter().filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("fetch-prev-"))
        }));
    }
    dirs
}

fn rows_if_present(path: &Path) -> Result<Vec<Cand>> {
    if path.join("candidates.jsonl").try_exists()? {
        load_candidates_jsonl(path)
    } else {
        Ok(Vec::new())
    }
}

pub fn candidate_lookup(args: &CandArgs, paths: &DigestPaths, lang: Lang) -> Result<String> {
    use std::fmt::Write;
    let mut dirs = args.dirs.clone();
    if dirs.is_empty() || args.since.is_some() {
        let runs = dirs_under(&paths.runs);
        let days: Vec<_> = runs
            .iter()
            .filter_map(|p| {
                let name = p.file_name()?.to_str()?;
                let day = name.parse::<NaiveDate>().ok()?;
                Some((day, p))
            })
            .collect();
        dirs = if let Some(since) = args.since {
            let mut dirs: Vec<_> = days
                .into_iter()
                .filter(|(day, _)| *day >= since)
                .flat_map(|(_, p)| {
                    let mut dirs = day_fetch_dirs(p, true);
                    // Archived runs predate the active run on the same date.
                    dirs.rotate_left(2);
                    dirs
                })
                .collect();
            let weekly = runs.iter().rev().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix("-weekly"))
                    .is_some_and(|n| n.parse::<NaiveDate>().is_ok())
                    && p.join("fetch").is_dir()
            });
            if let Some(w) = weekly {
                dirs.push(w.join("fetch"));
            }
            dirs
        } else {
            days.last()
                .map(|(_, p)| day_fetch_dirs(p, false))
                .unwrap_or_default()
        };
    }
    let mut data = Vec::new();
    for dir in &dirs {
        match rows_if_present(dir) {
            Ok(rows) => data.extend(rows),
            Err(error) => eprintln!("error: {error:#}"),
        }
    }
    let mut out = String::new();
    for key in &args.keys {
        let k = key.trim_start_matches('#').to_lowercase();
        let mut hits: Vec<_> = data
            .iter()
            .filter(|r| {
                r.id.starts_with(&k) || r.extra.get("arxiv_id").and_then(Value::as_str) == Some(&k)
            })
            .collect();
        if hits.is_empty() {
            hits = data
                .iter()
                .filter(|r| {
                    r.title.to_lowercase().contains(&k) || r.url.to_lowercase().contains(&k)
                })
                .collect();
        }
        if dirs.len() > 2 {
            let mut seen = HashSet::new();
            hits.reverse();
            hits.retain(|r| seen.insert(&r.title));
            hits.reverse();
        }
        if hits.is_empty() {
            writeln!(out, "## {key}: {}", ft(lang, "c_nomatch"))?;
        }
        for r in hits.iter().take(args.max) {
            let mut flags = vec![
                format!("[{}]", r.pref),
                if r.also_in.is_empty() {
                    r.source.clone()
                } else {
                    format!("{}+{}", r.source, r.also_in.join(","))
                },
                candidate_time(r),
            ];
            if status(r) == "pushed" {
                flags.push(ft(lang, "c_pushed").into());
            } else if !status(r).is_empty() || matches!(r.in_ledger, InLedger::Flag(true)) {
                flags.push(ft(lang, "c_seen").into());
            }
            for (field, key) in [
                ("date_unknown", "c_date_unknown"),
                ("discovery_only", "c_discovery"),
            ] {
                if r.extra.get(field).and_then(Value::as_bool) == Some(true) {
                    flags.push(ft(lang, key).into());
                }
            }
            writeln!(
                out,
                "## {}\n   {}\n   {}",
                r.title,
                flags.join(" · "),
                r.url
            )?;
            for key in ["github", "project", "hn"] {
                if let Some(value) = r
                    .extra
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    writeln!(out, "   {key}: {value}")?;
                }
            }
            if !r.authors.is_empty() {
                writeln!(
                    out,
                    "{}{}{}",
                    ft(lang, "c_authors"),
                    r.authors[..r.authors.len().min(4)].join(", "),
                    if r.authors.len() > 4 {
                        ft(lang, "c_et_al")
                    } else {
                        ""
                    }
                )?;
            }
            if !r.summary.is_empty() {
                writeln!(
                    out,
                    "   {}",
                    short(&r.summary, args.chars).replace('\n', " ")
                )?;
            }
        }
        if hits.len() > args.max {
            writeln!(
                out,
                "{}",
                fmt_label(
                    lang,
                    "c_more",
                    &[("n", &(hits.len() - args.max).to_string())]
                )
            )?;
        }
    }
    Ok(out)
}

fn candidate_time(c: &Cand) -> String {
    c.published
        .as_deref()
        .and_then(|s| s.get(5..16))
        .unwrap_or_default()
        .replace('T', " ")
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    pub path: PathBuf,
    #[arg(long)]
    pub finalize: bool,
    #[arg(long = "dir", value_name = "DIR")]
    pub dirs: Vec<PathBuf>,
    #[arg(long)]
    pub json: bool,
    #[arg(long, global = true)]
    pub lang: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct CheckResult {
    pub ok: bool,
    pub n: usize,
    pub strong: usize,
    pub weak: usize,
    pub pushed: usize,
    pub unknown: Vec<String>,
    pub duplicates: Vec<String>,
    pub problems: Vec<String>,
    pub r#final: Option<PathBuf>,
}

static CHECK_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^- (?:[★☆] )?(\d{4}\.\d{4,5})\b").unwrap());
static DATED_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|/)runs/(\d{4}-\d{2}-\d{2})/").unwrap());

pub fn check_candidates(args: &CheckArgs, paths: &DigestPaths, lang: Lang) -> Result<CheckResult> {
    let path = std::path::absolute(&args.path)?;
    let mut result = CheckResult::default();
    let input = match fs::read_to_string(&path) {
        Ok(input) => input,
        Err(e) if args.json => {
            result.problems.push(format!("{}: {e}", path.display()));
            return Ok(result);
        }
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let dirs = if args.dirs.is_empty() {
        DATED_RUN
            .captures(&path.to_string_lossy())
            .map(|m| day_fetch_dirs(&paths.runs.join(&m[1]), false))
            .unwrap_or_default()
    } else {
        args.dirs.clone()
    };
    let mut known = BTreeMap::new();
    let ledger = load_ledger_keys(&paths.ledger).unwrap_or_default();
    for dir in dirs {
        for row in rows_if_present(&dir)? {
            if let Some(aid) = row.extra.get("arxiv_id").and_then(Value::as_str) {
                let pushed = ledger
                    .get(&format!("arxiv:{aid}"))
                    .copied()
                    .map_or_else(|| status(&row) == "pushed", |s| s == "pushed");
                known.insert(aid.to_owned(), pushed);
            }
        }
    }
    let mut strong = false;
    let mut counts = BTreeMap::new();
    for line in input.lines() {
        if let Some(tier) = line.strip_prefix("## ") {
            let tier = tier.to_lowercase();
            strong = CHECK_CANDS_STRONG_TIER_WORDS
                .iter()
                .any(|word| tier.contains(word));
        }
        let Some(m) = CHECK_LINE.captures(line) else {
            continue;
        };
        let aid = m[1].to_owned();
        result.n += 1;
        if strong {
            result.strong += 1;
        } else {
            result.weak += 1;
        }
        match known.get(&aid) {
            Some(true) => result.pushed += 1,
            Some(false) => {}
            None => result.unknown.push(aid.clone()),
        }
        *counts.entry(aid).or_insert(0) += 1;
    }
    result.duplicates = counts
        .into_iter()
        .filter_map(|(id, n)| (n > 1).then_some(id))
        .collect();
    for (key, ids) in [
        ("cc_bad_unknown", &result.unknown),
        ("cc_bad_dup", &result.duplicates),
    ] {
        if !ids.is_empty() {
            result.problems.push(format!(
                "{}{}",
                ft(lang, key),
                ids[..ids.len().min(10)].join(" ")
            ));
        }
    }
    if !(10..=60).contains(&result.n) {
        result.problems.push(fmt_label(
            lang,
            "cc_bad_count",
            &[("n", &result.n.to_string())],
        ));
    }
    result.ok = result.problems.is_empty();
    if result.ok && args.finalize {
        let dest = path
            .to_str()
            .and_then(|s| s.strip_suffix(".tmp"))
            .map_or_else(|| path.clone(), PathBuf::from);
        if dest != path {
            fs::rename(path, &dest)?;
        }
        result.r#final = Some(dest);
    }
    Ok(result)
}

impl CheckResult {
    pub fn display(&self, lang: Lang) -> String {
        let mut msg = format!(
            "{} n={} ({} {} / {} {}) {} {} {} {} {} {}",
            if self.ok { "OK" } else { "PROBLEM" },
            self.n,
            ft(lang, "cc_strong"),
            self.strong,
            ft(lang, "cc_weak"),
            self.weak,
            ft(lang, "cc_pushed"),
            self.pushed,
            ft(lang, "cc_unknown"),
            self.unknown.len(),
            ft(lang, "cc_dup"),
            self.duplicates.len()
        );
        if !self.problems.is_empty() {
            msg.push_str(&format!("\n  {}", self.problems.join("\n  ")));
        }
        if let Some(path) = &self.r#final {
            msg.push_str(&format!(" → {}", path.display()));
        }
        msg
    }
}

#[derive(Args, Debug)]
pub struct PageArgs {
    #[arg(required = true, value_name = "URL[@@KEYWORDS]")]
    pub urls: Vec<String>,
    #[arg(long, default_value_t = 3000)]
    pub chars: usize,
    #[arg(long, value_name = "KW1,KW2")]
    pub find: Option<String>,
    #[arg(long, default_value_t = 160)]
    pub ctx: usize,
    #[arg(long)]
    pub meta: bool,
    #[arg(long, global = true)]
    pub lang: Option<String>,
}

fn page_find(text: &str, keywords: &str, context: usize, lang: Lang) -> String {
    let mut out = Vec::new();
    let positions: Vec<_> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    for kw in keywords.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let re = Regex::new(&format!("(?i){}", regex::escape(kw))).expect("escaped keyword");
        let hits: Vec<_> = re.find_iter(text).take(3).collect();
        out.push(format!(
            "\n{}",
            fmt_label(
                lang,
                "pg_hits",
                &[
                    ("kw", kw),
                    ("n", &hits.len().to_string()),
                    ("plus", if hits.len() == 3 { "+" } else { "" })
                ]
            )
        ));
        for hit in hits {
            let start = positions
                .binary_search(&hit.start())
                .unwrap()
                .saturating_sub(context);
            let end = positions
                .binary_search(&hit.end())
                .unwrap()
                .saturating_add(context)
                .min(positions.len() - 1);
            out.push(format!(
                "…{}…",
                text[positions[start]..positions[end]].replace('\n', " ")
            ));
        }
    }
    out.join("\n")
}

pub fn page_text(
    html: &str,
    chars: usize,
    find: Option<&str>,
    context: usize,
    meta: bool,
    lang: Lang,
) -> String {
    if html.trim().is_empty() {
        return fmt_label(lang, "pg_short", &[("n", "0")]);
    }
    let doc = Document::parse(html);
    let mut out = page_meta(&doc, lang);
    if meta {
        return out;
    }
    const SKIP: &[&str] = &[
        "script", "style", "noscript", "nav", "footer", "header", "aside", "form", "svg", "iframe",
        "button",
    ];
    let mut excluded = vec![false; doc.nodes.len()];
    for (i, element) in doc.nodes.iter().enumerate().skip(1) {
        excluded[i] = excluded[element.parent] || SKIP.contains(&element.tag.as_str());
    }
    let root = doc
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, e)| {
            !excluded[*i]
                && (matches!(e.tag.as_str(), "article" | "main") || e.attr("role") == "main")
        })
        .max_by_key(|(i, _)| doc.text_len(*i))
        .map_or(0, |(i, _)| i);
    let mut seen = HashSet::new();
    let body = doc
        .descendants(root)
        .filter(|(i, _)| !excluded[*i])
        .filter_map(|(i, e)| {
            let line = clean_text(doc.text(i));
            if line.chars().count() < 2 {
                return None;
            }
            match e.tag.as_str() {
                "h1" | "h2" | "h3" => Some(format!("\n## {line}")),
                "p" | "li" | "pre" | "blockquote" | "td" | "figcaption" => Some(line),
                _ => None,
            }
        })
        .filter(|line| seen.insert(line.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    let length = body.chars().count();
    out.push('\n');
    if length < 200 {
        out.push_str(&fmt_label(lang, "pg_short", &[("n", &length.to_string())]));
    } else if let Some(find) = find {
        out.push_str(&page_find(&body, find, context, lang));
    } else {
        out.push_str(short(&body, chars));
        if length > chars {
            out.push_str(&format!(
                "\n{}",
                fmt_label(lang, "pg_trunc", &[("n", &length.to_string())])
            ));
        }
    }
    out
}

fn page_title(doc: &Document) -> &str {
    doc.meta(&["og:title"])
        .or_else(|| {
            doc.nodes
                .iter()
                .enumerate()
                .find(|(_, e)| e.tag == "title")
                .map(|(id, _)| doc.text(id).trim())
        })
        .unwrap_or_default()
}

fn page_meta(doc: &Document, lang: Lang) -> String {
    let mut out = format!("# {}", page_title(doc));
    for (key, names) in [
        (
            "pg_date",
            &[
                "article:published_time",
                "date",
                "datePublished",
                "og:updated_time",
            ][..],
        ),
        (
            "pg_desc",
            &["og:description", "description", "twitter:description"][..],
        ),
    ] {
        if let Some(value) = doc.meta(names) {
            out.push_str(&format!("\n{}{}", ft(lang, key), short(value, 300)));
        }
    }
    out
}

static ARXIV_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?:abs|pdf)/(\d{4}\.\d{4,5})(?:v\d+)?(?:\.pdf)?/?$").unwrap());

async fn page_url(
    http: &SafeHttpClient,
    input: &str,
    args: &PageArgs,
    find: Option<&str>,
    lang: Lang,
) -> Result<String> {
    let url = Url::parse(input)?;
    validate_target(&url)?;
    let fetch = |url: Url| async move {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            reqwest::header::HeaderValue::from_static("en,zh;q=0.8"),
        );
        let resp = http
            .get_with_headers(&url, None, &headers)
            .await?
            .error_for_status()?;
        Ok::<_, anyhow::Error>(read_text_limited(resp, MAX_BODY_BYTES).await?)
    };
    if matches!(url.host_str(), Some("arxiv.org" | "www.arxiv.org")) {
        if let Some(aid) = ARXIV_PATH.captures(url.path()) {
            let html = fetch(Url::parse(&format!("https://arxiv.org/abs/{}", &aid[1]))?).await?;
            if html.trim().is_empty() {
                return Ok(fmt_label(lang, "pg_short", &[("n", "0")]));
            }
            let doc = Document::parse_strict(&html)?;
            let date = doc
                .class_text("dateline")
                .map(clean_text)
                .unwrap_or_default();
            let abs = doc
                .class_text("abstract")
                .map(|t| clean_text(t.trim().trim_start_matches("Abstract:")))
                .unwrap_or_default();
            let authors = doc
                .nodes
                .iter()
                .enumerate()
                .find(|(_, e)| e.has_class("authors"))
                .map(|(i, _)| {
                    doc.descendants(i)
                        .filter(|(_, e)| e.tag == "a")
                        .take(8)
                        .map(|(a_id, _)| clean_text(doc.text(a_id)))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            return Ok(format!(
                "# {}\n{date} | {authors}\n{}",
                page_title(&doc),
                short(&abs, args.chars)
            ));
        }
    }
    if url.host_str() == Some("github.com") && url.query().is_none() && url.fragment().is_none() {
        let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
        if let [owner, repo] = parts.as_slice() {
            if [owner, repo].iter().all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            }) {
                for branch in ["HEAD", "main", "master"] {
                    let raw = Url::parse(&format!(
                        "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/README.md"
                    ))?;
                    if let Ok(text) = fetch(raw).await {
                        let head = format!("# {owner}/{repo} README");
                        return Ok(if args.meta {
                            head
                        } else if let Some(find) = find {
                            format!("{head}\n{}", page_find(&text, find, args.ctx, lang))
                        } else {
                            format!("{head}\n{}", short(&text, args.chars))
                        });
                    }
                }
            }
        }
    }
    let html = fetch(url).await?;
    let _ = Document::parse_strict(&html)?;
    Ok(page_text(
        &html, args.chars, find, args.ctx, args.meta, lang,
    ))
}

pub async fn pages(args: &PageArgs, lang: Lang) -> (bool, String) {
    let http = SafeHttpClient::new(
        Duration::from_secs(25),
        1,
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/124 Safari/537.36",
    );
    pages_with_client(args, lang, &http).await
}

pub async fn pages_with_client(
    args: &PageArgs,
    lang: Lang,
    http: &SafeHttpClient,
) -> (bool, String) {
    let multi = args.urls.len() > 1 || args.urls.first().is_some_and(|s| s.contains("@@"));
    let results = stream::iter(&args.urls)
        .map(|spec| async move {
            let (url, kw) = spec.split_once("@@").unwrap_or((spec, ""));
            let find = if kw.is_empty() {
                args.find.as_deref()
            } else {
                Some(kw)
            };
            let (ok, body) = match page_url(http, url, args, find, lang).await {
                Ok(text) => (true, text),
                Err(e) => (
                    false,
                    format!(
                        "ERROR {} → {}",
                        short(&e.to_string(), 150),
                        ft(lang, "pg_fallback")
                    ),
                ),
            };
            (
                ok,
                if multi {
                    format!("### {url}\n{}\n", body.trim())
                } else {
                    body
                },
            )
        })
        .buffered(8)
        .collect::<Vec<_>>()
        .await;
    (
        multi || results.iter().all(|(ok, _)| *ok),
        results
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

static BIG_ORG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)google|deepmind|openai|anthropic|\bmeta\b|fair\b|microsoft|nvidia|apple|amazon|\baws\b|bytedance|\bseed\b|alibaba|qwen|tongyi|tencent|hunyuan|deepseek|moonshot|kimi|zhipu|z\.ai|minimax|xiaomi|baidu|huawei|mistral|allen|\bai2\b|stanford|\bmit\b|massachusetts institute|berkeley|carnegie|princeton|tsinghua|peking|oxford|cambridge|\beth\b|epfl|toronto|mila|inclusionai|ant group|shanghai ai|sakana|cohere|\bxai\b|\bibm\b|samsung|naver|kuaishou|kling|meituan|sensetime|stepfun|skywork|openbmb|longcat|internlm|thinking machines|together ai|hugging ?face|databricks|snowflake|salesforce|adobe|\bintel\b|\bamd\b|qualcomm|\btii\b|\bntu\b|nanyang|national university of singapore|kaist|tokyo|riken"#).unwrap()
});

#[derive(Args, Debug)]
pub struct WeeklyArgs {
    #[arg(long)]
    pub day: Option<NaiveDate>,
    #[arg(long, requires = "end")]
    pub start: Option<NaiveDate>,
    #[arg(long, requires = "start")]
    pub end: Option<NaiveDate>,
    #[arg(long, default_value_t = 0)]
    pub chars: usize,
    #[arg(long, default_value_t = 40)]
    pub max: usize,
    #[arg(long, default_value_t = 15)]
    pub max_np: usize,
    #[arg(long)]
    pub all: bool,
    #[arg(long, global = true)]
    pub lang: Option<String>,
}

struct WeeklyPaper {
    item: Cand,
    first: NaiveDate,
    hf_days: BTreeSet<NaiveDate>,
    other: BTreeSet<String>,
    ledger: &'static str,
    org: String,
}

fn month_day(date: NaiveDate) -> String {
    date.format("%m-%d").to_string()
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WeeklyArguments(#[source] anyhow::Error);

pub fn weekly_late(
    args: &WeeklyArgs,
    paths: &DigestPaths,
    lang: Lang,
    now: DateTime<Utc>,
) -> Result<String> {
    use std::fmt::Write;
    let (start, end) = match args.start.zip(args.end) {
        Some(pair) => pair,
        None => {
            let day = match args.day {
                Some(d) => d,
                None => now
                    .with_timezone(&paths.timezone)
                    .date_naive()
                    .checked_sub_days(Days::new(7))
                    .context("weekly date out of range")
                    .map_err(WeeklyArguments)?,
            };
            let m = day
                .checked_sub_days(Days::new(u64::from(day.weekday().num_days_from_monday())))
                .context("weekly start date out of range")
                .map_err(WeeklyArguments)?;
            let s = m
                .checked_add_days(Days::new(6))
                .context("weekly end date out of range")
                .map_err(WeeklyArguments)?;
            (m, s)
        }
    };
    if start > end {
        return Err(WeeklyArguments(anyhow::anyhow!("invalid weekly date range")).into());
    }
    let mut files = Vec::new();
    let mut have = Vec::new();
    let mut missing = Vec::new();
    for date in start.iter_days().take_while(|d| *d <= end) {
        let mut found = false;
        for dir in day_fetch_dirs(&paths.runs.join(date.to_string()), true) {
            if dir.join("candidates.jsonl").is_file() {
                files.push((date, dir));
                found = true;
            }
        }
        if found {
            have.push(date.to_string());
        } else {
            missing.push(date.to_string());
        }
    }
    let gap = paths.runs.join(format!("{end}-weekly")).join("fetch");
    if let Some((lo, hi, day)) = fs::read_to_string(gap.join("report.json"))
        .ok()
        .and_then(|s| {
            let value: Value = serde_json::from_str(&s).ok()?;
            let (s, e) = (
                str_field(&value, "window_start"),
                str_field(&value, "window_end"),
            );
            let span = parse_dt(e)? - parse_dt(s)?;
            (span >= Delta::zero() && span < Delta::days(2)).then_some((
                s.get(5..16)?.to_owned(),
                e.get(5..16)?.to_owned(),
                s.get(..10)?.parse().ok()?,
            ))
        })
    {
        files.push((day, gap));
        have.push(format!(
            "{end}-weekly({})",
            fmt_label(lang, "wl_gap", &[("start", &lo), ("end", &hi)])
        ));
    }
    let ledger = load_ledger_keys(&paths.ledger)?;
    let lookup = |row: &Cand| {
        ledger
            .get(&candidate_key(row))
            .or_else(|| ledger.get(&format!("t:{}", norm_title(&row.title))))
            .copied()
            .unwrap_or_default()
    };
    let core = ["lab", "policy", "research_blog"];
    let extra = ["tooling", "newsletter", "podcast"];
    let mut papers = BTreeMap::new();
    let mut nonpapers = BTreeMap::new();
    let mut xref = BTreeMap::<String, Vec<String>>::new();
    for (day, dir) in files {
        for row in rows_if_present(&dir)? {
            let history = lookup(&row);
            if row.channel != "paper" {
                for hit in
                    ARXIV_ID.captures_iter(&format!("{} {} {}", row.url, row.title, row.summary))
                {
                    xref.entry(hit[1].into()).or_default().push(format!(
                        "{}:{}",
                        row.source,
                        short(&row.title, 40)
                    ));
                }
                if core.contains(&row.channel.as_str()) || extra.contains(&row.channel.as_str()) {
                    nonpapers
                        .entry(canon_url(&row.url))
                        .or_insert((day, row, history));
                }
                continue;
            }
            let it = papers
                .entry(candidate_key(&row))
                .or_insert_with(|| WeeklyPaper {
                    item: row.clone(),
                    first: day,
                    hf_days: BTreeSet::new(),
                    other: BTreeSet::new(),
                    ledger: "",
                    org: String::new(),
                });
            it.first = it.first.min(day);
            it.item.pref = it.item.pref.max(row.pref).max(0);
            if it.item.summary.is_empty() {
                it.item.summary = row.summary;
            }
            for source in row.also_in.iter().chain(std::iter::once(&row.source)) {
                match source.as_str() {
                    "hf_daily" => {
                        it.hf_days.insert(day);
                        if it.org.is_empty() {
                            it.org = row
                                .extra
                                .get("org")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .into();
                        }
                    }
                    "arxiv" => {}
                    _ => {
                        it.other.insert(source.clone());
                    }
                }
            }
            if it.ledger.is_empty() || history == "pushed" {
                it.ledger = history;
            }
        }
    }
    let mut dropped = 0;
    let mut ranked = Vec::new();
    for (key, paper) in papers.into_iter().filter(|(_, p)| p.ledger != "pushed") {
        let hf_late = paper.hf_days.iter().find(|&&day| day > paper.first);
        let big = BIG_ORG.is_match(&paper.org);
        let org = if paper.org.is_empty() {
            String::new()
        } else {
            format!("·{}", short(&paper.org, 22))
        };
        let institution = if big { ft(lang, "wl_big_org") } else { "" };
        let mut signals = Vec::new();
        if let Some(&day) = hf_late {
            if paper.item.pref >= 2 || big || args.all {
                signals.push(format!(
                    "{}{org}{institution}",
                    fmt_label(lang, "wl_late_hf", &[("d", &month_day(day))])
                ));
            } else {
                dropped += 1;
            }
        } else if let Some(&day) = paper.hf_days.first() {
            if ((paper.item.pref >= 3 || big) && paper.ledger.is_empty()) || args.all {
                signals.push(format!(
                    "{}{org}{}{institution}",
                    fmt_label(lang, "wl_hf", &[("d", &month_day(day))]),
                    ft(lang, "wl_no_ledger")
                ));
            } else {
                dropped += 1;
            }
        }
        let late = hf_late.is_some() && !signals.is_empty();
        if !paper.other.is_empty() {
            let sources = paper
                .other
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(",");
            signals.push(format!("{}{sources}", ft(lang, "sig_multi")));
        }
        let refs = key.strip_prefix("arxiv:").and_then(|id| xref.get(id));
        if let Some(refs) = refs {
            signals.push(format!(
                "{}{}",
                ft(lang, "sig_mentioned"),
                refs[..refs.len().min(2)].join("; ")
            ));
        }
        if !signals.is_empty()
            && (paper.ledger != "seen"
                || hf_late.is_some()
                || !paper.other.is_empty()
                || refs.is_some())
        {
            ranked.push((late, paper, signals.join("; ")));
        }
    }
    ranked.sort_by(|(a, p, _), (b, q, _)| {
        (
            std::cmp::Reverse(*a),
            std::cmp::Reverse(p.item.pref),
            p.first,
            &p.item.title,
        )
            .cmp(&(
                std::cmp::Reverse(*b),
                std::cmp::Reverse(q.item.pref),
                q.first,
                &q.item.title,
            ))
    });
    let all_np = nonpapers
        .values()
        .filter(|(_, _, ledger)| ledger.is_empty())
        .count();
    let mut nonpapers: Vec<_> = nonpapers
        .into_values()
        .filter(|(_, c, ledger)| {
            ledger.is_empty() && (args.all || c.pref >= 3 || core.contains(&c.channel.as_str()))
        })
        .collect();
    nonpapers.sort_by(|(ad, a, _), (bd, b, _)| {
        (&a.channel, ad, &a.title).cmp(&(&b.channel, bd, &b.title))
    });
    let mut out = String::new();
    writeln!(
        out,
        "{}",
        fmt_label(
            lang,
            "wl_title",
            &[("m", &start.to_string()), ("s", &end.to_string())]
        )
    )?;
    write!(out, "{}{}", ft(lang, "wl_sources"), have.join(", "))?;
    if !missing.is_empty() {
        write!(out, "{}{}", ft(lang, "wl_missing"), missing.join(", "))?;
    }
    writeln!(
        out,
        "\n{}\n",
        fmt_label(lang, "wl_hint", &[("m", &start.to_string())])
    )?;
    writeln!(
        out,
        "{}",
        fmt_label(lang, "wl_papers", &[("n", &ranked.len().to_string())])
    )?;
    for (_, paper, signals) in ranked.iter().take(args.max) {
        let c = &paper.item;
        let id = c
            .extra
            .get("arxiv_id")
            .and_then(Value::as_str)
            .unwrap_or(&c.url);
        let star = if paper.ledger == "seen" { "☆ " } else { "" };
        let weight = fmt_label(lang, "wl_weight", &[("n", &c.pref.to_string())]);
        let first = fmt_label(lang, "wl_first", &[("d", &month_day(paper.first))]);
        writeln!(
            out,
            "- {id} {star}{} {weight} {first}〔{signals}〕",
            short(&c.title, 100)
        )?;
        if args.chars > 0 && !c.summary.is_empty() {
            writeln!(out, "  - {}", short(&clean_text(&c.summary), args.chars))?;
        }
    }
    for (key, count) in [
        ("wl_more", ranked.len().saturating_sub(args.max)),
        ("wl_dropped", dropped),
    ] {
        if count > 0 {
            writeln!(
                out,
                "{}",
                fmt_label(lang, key, &[("n", &count.to_string())])
            )?;
        }
    }
    writeln!(
        out,
        "\n{}",
        fmt_label(
            lang,
            "wl_nonpaper",
            &[
                ("n", &nonpapers.len().to_string()),
                ("total", &all_np.to_string())
            ]
        )
    )?;
    let mut counts = BTreeMap::new();
    let mut cut = 0;
    for (day, c, _) in &nonpapers {
        let count = counts.entry(&c.channel).or_insert(0);
        if *count == 0 {
            writeln!(out, "### {}", c.channel)?;
        }
        *count += 1;
        if *count > args.max_np {
            cut += 1;
            continue;
        }
        let time = candidate_time(c);
        writeln!(
            out,
            "- {} {} | {} | {}",
            if time.is_empty() {
                month_day(*day)
            } else {
                time
            },
            c.source,
            short(&c.title, 90),
            c.url
        )?;
    }
    if cut > 0 {
        writeln!(
            out,
            "{}",
            fmt_label(lang, "wl_more_np", &[("n", &cut.to_string())])
        )?;
    }
    Ok(out)
}

#[derive(Args, Debug)]
pub struct XparseArgs {
    #[arg(required = true, value_name = "LABEL=PATH")]
    pub inputs: Vec<String>,
    #[arg(long, default_value_t = 200)]
    pub chars: usize,
    #[arg(long)]
    pub out: Option<PathBuf>,
    #[arg(long, global = true)]
    pub lang: Option<String>,
}

pub fn x_json(input: &str) -> Option<Value> {
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"\{\s*"(?:data|meta|includes|results|news|errors|error|title)""#).unwrap()
    });
    PREFIX.find_iter(input).find_map(|m| {
        serde_json::Deserializer::from_str(&input[m.start()..])
            .into_iter::<Value>()
            .next()?
            .ok()
    })
}

fn x_load(filename: &str) -> Option<Value> {
    let path = Path::new(filename);
    let open = || {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let prefix = path.file_name()?.to_str()?;
        let mut options: Vec<_> = fs::read_dir(parent)
            .ok()?
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|s| s.starts_with(prefix))
                    && e.path().is_file()
            })
            .map(|e| e.path())
            .collect();
        options.sort_unstable();
        fs::read(options.first()?).ok()
    };
    let bytes = fs::read(path).ok().or_else(open)?;
    x_json(&String::from_utf8_lossy(&bytes))
}

fn x_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| value.is_number().then(|| value.to_string()))
}

fn x_time(value: &Value, timezone: chrono_tz::Tz) -> String {
    let ts = x_value(value).and_then(|s| {
        if let Ok(n) = s.parse::<f64>() {
            if !n.is_finite() || n.abs() >= i64::MAX as f64 {
                return None;
            }
            let n = if n > 1e11 { n / 1000.0 } else { n };
            DateTime::from_timestamp(n.floor() as i64, 0).map(|d| d.fixed_offset())
        } else {
            parse_dt(&s)
        }
    });
    ts.map_or_else(
        || "?".into(),
        |d| d.with_timezone(&timezone).format("%m-%d %H:%M").to_string(),
    )
}

fn first_object_array(obj: &Map<String, Value>) -> Option<&[Value]> {
    obj.values()
        .map(array)
        .find(|a| a.first().is_some_and(Value::is_object))
}

pub fn xparse(args: &XparseArgs, lang: Lang, timezone: chrono_tz::Tz) -> Result<String> {
    static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").unwrap());
    let mut counts = Vec::new();
    let mut seen_id = HashSet::new();
    let mut seen_text = HashSet::new();
    let mut trunc = Vec::new();
    let mut lines = Vec::new();
    for (i, spec) in args.inputs.iter().enumerate() {
        let fallback = format!("F{}", i + 1);
        let (label, path) = spec
            .split_once('=')
            .filter(|(_, p)| !p.is_empty())
            .unwrap_or((&fallback, spec));
        let Some(doc) = x_load(path) else {
            counts.push(format!("{label}={}", ft(lang, "xp_parse_fail")));
            continue;
        };
        let empty = Value::Null;
        let data_val = ["data", "results"]
            .into_iter()
            .filter_map(|k| doc.get(k))
            .find(|v| {
                v.as_array().is_some_and(|a| !a.is_empty())
                    || v.as_object().is_some_and(|m| !m.is_empty())
            })
            .unwrap_or(&empty);
        let mut data = if let Some(object) = data_val.as_object() {
            first_object_array(object).unwrap_or_else(|| std::slice::from_ref(data_val))
        } else {
            array(data_val)
        };
        if data.is_empty() {
            data = doc
                .as_object()
                .and_then(first_object_array)
                .unwrap_or_default();
        }
        let users: BTreeMap<_, _> = array(&doc["includes"]["users"])
            .iter()
            .filter_map(|u| Some((x_value(&u["id"])?, str_field(u, "username"))))
            .collect();
        counts.push(format!("{label}={}", data.len()));
        if let Some(next) = x_value(&doc["meta"]["next_token"]) {
            let oldest = data
                .iter()
                .filter_map(|p| x_value(&p["created_at"]).map(|s| (s, &p["created_at"])))
                .min_by(|a, b| a.0.cmp(&b.0))
                .map_or_else(|| "?".into(), |(_, d)| x_time(d, timezone));
            trunc.push(format!(
                "{label}({}, next_token={next})",
                fmt_label(lang, "xp_oldest", &[("t", &oldest)])
            ));
        }
        for post in data.iter().filter(|p| p.is_object()) {
            let id = ["id", "rest_id", "post_id"]
                .into_iter()
                .find_map(|k| x_value(&post[k]))
                .unwrap_or_default();
            if !id.is_empty() && !seen_id.insert(id.clone()) {
                continue;
            }
            let text = ["text", "title", "summary"]
                .into_iter()
                .map(|k| str_field(post, k))
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            let bare = LINK.replace_all(text, "");
            let bare = bare.trim();
            if bare.chars().count() < 25 || bare.starts_with('@') || bare.starts_with("RT @") {
                continue;
            }
            let norm: String = bare
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .take(70)
                .collect();
            if !seen_text.insert(norm) {
                continue;
            }
            let time = ["created_at", "last_updated_at_ms"]
                .into_iter()
                .find(|k| x_value(&post[*k]).is_some())
                .map_or_else(|| "?".into(), |k| x_time(&post[k], timezone));
            let mut url = str_field(post, "url").replace("https://x.com/", "x.com/");
            if url.is_empty() && !id.is_empty() {
                let user = x_value(&post["author_id"])
                    .and_then(|author| users.get(&author).copied())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("i");
                url = format!("x.com/{user}/status/{id}");
            }
            lines.push(format!(
                "[{label}] {time} {url} | {}",
                short(&clean_text(bare), args.chars)
            ));
        }
    }
    let counts = counts.join(" ");
    let warning = if trunc.is_empty() {
        String::new()
    } else {
        format!("{}{}\n", ft(lang, "xp_trunc"), trunc.join("; "))
    };
    let out = format!("{counts}\n{warning}{}", lines.join("\n"));
    if let Some(file) = &args.out {
        let report = fmt_label(
            lang,
            "xp_wrote",
            &[
                ("counts", &counts),
                ("out", &file.to_string_lossy()),
                ("n", &lines.len().to_string()),
                ("bytes", &out.len().to_string()),
            ],
        );
        if let Err(error) = atomic_write_text(file, &format!("{out}\n")) {
            eprintln!("error: {error:#}");
            return Ok(format!("{out}\n"));
        }
        Ok(format!("{report}\n{warning}"))
    } else {
        Ok(format!("{out}\n"))
    }
}
