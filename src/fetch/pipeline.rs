use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Days, Duration as Delta, NaiveDate, Utc};
use clap::Args;
use futures_util::{stream, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};

use crate::fetch::sources::{
    candidate_key, compute_window, cross_references, fetch_source, fmt_label,
    load_candidates_jsonl, load_preferences, paper_signals, parse_dt, parse_registry, score_topics,
    short, status, SourceContext, Window, CHANNELS,
};
use crate::i18n::{cnt_paren, ft, Lang};
use crate::ledger::load_ledger_keys;
use crate::model::{
    atomic_write_text, norm_title, Cand, DigestPaths, FetchReport, FetchSourceReport, FetchState,
    FileLock, InLedger,
};

#[derive(Args, Debug)]
pub struct FetchArgs {
    #[arg(long, conflicts_with = "weekly", help = "Fetch a local calendar day")]
    pub daily: bool,
    #[arg(long, help = "Fetch the Monday to Sunday week")]
    pub weekly: bool,
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub day: Option<NaiveDate>,
    #[arg(long, value_name = "ISO", help = "Override the inclusive window start")]
    pub start: Option<String>,
    #[arg(long, value_name = "ISO", help = "Override the exclusive window end")]
    pub end: Option<String>,
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    #[arg(long, value_name = "CSV", value_delimiter = ',')]
    pub only: Vec<String>,
    #[arg(long, value_name = "CSV", value_delimiter = ',')]
    pub skip: Vec<String>,
    #[arg(long)]
    pub no_state: bool,
    #[arg(long)]
    pub catchup: bool,
    #[arg(long)]
    pub no_catchup: bool,
    #[arg(long)]
    pub compact: bool,
    #[arg(long, value_name = "TOML")]
    pub registry: Option<PathBuf>,
    #[arg(long, default_value_t = 0, value_name = "MIN")]
    pub wait_arxiv: u64,
    #[arg(long)]
    pub no_fold: bool,
    #[arg(long)]
    pub reuse: bool,
    #[arg(long)]
    pub verbose_sources: bool,
    #[arg(long)]
    pub full_triage: bool,
    #[arg(long, global = true)]
    pub lang: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(
        long,
        value_name = "DIR",
        help = "Regenerate triage from existing candidates without network access"
    )]
    pub rebuild: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct FetchArguments(#[source] anyhow::Error);

pub fn dedupe(mut rows: Vec<Cand>) -> Vec<Cand> {
    const RANK: &[&str] = &[
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
    rows.sort_by_key(|c| RANK.iter().position(|k| *k == c.kind).unwrap_or(99));
    let mut keys = HashMap::<String, usize>::new();
    let mut output: Vec<Cand> = Vec::new();
    for mut row in rows {
        let key = candidate_key(&row);
        if let Some(&hit) = keys.get(&key) {
            let existing = &mut output[hit];
            for source in std::iter::once(&row.source).chain(&row.also_in) {
                if *source != existing.source && !existing.also_in.contains(source) {
                    existing.also_in.push(source.clone());
                }
            }
            if row.source == "hf_daily" && existing.source != "hf_daily" {
                existing.extra.insert("hf_daily".into(), json!(true));
                for key in ["github", "project", "org"] {
                    if let Some(value) = row
                        .extra
                        .get(key)
                        .filter(|v| !v.is_null() && v.as_str() != Some(""))
                    {
                        existing.extra.entry(key).or_insert_with(|| value.clone());
                    }
                }
            }
            if existing.summary.is_empty() {
                existing.summary = row.summary;
            }
            continue;
        }
        row.id = hex::encode(&Sha1::digest(key.as_bytes())[..6]);
        keys.insert(key, output.len());
        output.push(row);
    }
    output
}

fn discovery(c: &Cand) -> bool {
    c.extra.get("discovery_only").and_then(Value::as_bool) == Some(true)
}

type Seen = BTreeMap<String, String>;

fn read_optional<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).with_context(|| format!("read {}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error.into()),
    }
}

fn unlink(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

struct Triage<'a> {
    out: &'a Path,
    lang: Lang,
    start: &'a str,
    end: &'a str,
    day: &'a str,
    seen: &'a Seen,
    compact: bool,
    full: bool,
    fold: bool,
}

impl Triage<'_> {
    fn label(&self, key: &str, fields: &[(&str, &str)]) -> String {
        fmt_label(self.lang, key, fields)
    }

    fn title(&self, key: &str, n: usize) -> String {
        self.label(
            key,
            &[
                ("start", self.start),
                ("end", self.end),
                ("n", &n.to_string()),
            ],
        )
    }

    fn write(&self, rows: &[Cand]) -> Result<()> {
        let lang = self.lang;
        let mut out = format!(
            "{}\n\n{}{}\n\n",
            self.title("title_nonpaper", 0),
            ft(lang, "head_a"),
            if self.full {
                ".".into()
            } else {
                format!("{}{}", lang.choose("; ", "；"), ft(lang, "head_b"))
            }
        );
        for &ch in CHANNELS.iter().filter(|&&ch| ch != "paper") {
            let mut items: Vec<_> = rows.iter().filter(|c| c.channel == ch).collect();
            if items.is_empty() {
                continue;
            }
            items.sort_by(|a, b| {
                (std::cmp::Reverse(a.pref), &a.published, &a.title).cmp(&(
                    std::cmp::Reverse(b.pref),
                    &b.published,
                    &b.title,
                ))
            });
            writeln!(
                out,
                "## {}{}\n",
                ft(lang, &format!("sec_{ch}")),
                cnt_paren(lang, items.len())
            )?;
            let mut pushed = Vec::new();
            let mut repeated = Vec::new();
            for c in items {
                let id = short(&c.id, 6);
                let reference = format!("{} #{id}", c.title);
                if !self.full && status(c) == "pushed" {
                    pushed.push(reference);
                    continue;
                }
                if !self.full
                    && discovery(c)
                    && self
                        .seen
                        .get(&candidate_key(c))
                        .is_some_and(|d| d.as_str() < self.day)
                {
                    repeated.push(reference);
                    continue;
                }
                let star = star(c);
                let when = c
                    .published
                    .as_deref()
                    .and_then(|s| s.get(5..16))
                    .unwrap_or_default()
                    .replace('T', " ");
                let extra = if c.also_in.is_empty() {
                    String::new()
                } else {
                    format!("(+{})", c.also_in.join(","))
                };
                let snip = matches!(ch, "news" | "news_zh" | "community" | "chips_infra");
                let rich = matches!(
                    ch,
                    "lab" | "tooling" | "policy" | "research_blog" | "newsletter" | "podcast"
                ) || matches!(c.kind.as_str(), "lab_post" | "essay" | "release")
                    || (c.pref >= 3 && !snip && !discovery(c));
                let limit = if self.full {
                    writeln!(out, "{}", full_line(c, 0))?;
                    if self.compact && (snip || matches!(ch, "repo" | "model")) {
                        0
                    } else if self.compact {
                        120
                    } else if matches!(ch, "repo" | "model" | "community") {
                        140
                    } else {
                        220
                    }
                } else if rich {
                    writeln!(
                        out,
                        "- {star}[{}] {} — `{}{} · {when}` {} #{id}",
                        c.pref,
                        c.title,
                        c.source,
                        if extra.is_empty() {
                            String::new()
                        } else {
                            format!(" {extra}")
                        },
                        c.url
                    )?;
                    160
                } else {
                    let weight = if c.pref == 0 {
                        String::new()
                    } else {
                        format!("[{}] ", c.pref)
                    };
                    writeln!(
                        out,
                        "- {star}{weight}{} — {}{extra} {when} {} #{id}",
                        c.title,
                        c.source,
                        c.topics[..c.topics.len().min(2)].join(",")
                    )?;
                    if snip && c.pref >= 3 {
                        100
                    } else {
                        0
                    }
                };
                if limit > 0 && !c.summary.is_empty() {
                    writeln!(out, "  - {}", short(&c.summary, limit))?;
                }
            }
            if !pushed.is_empty() {
                writeln!(out, "\n{}{}", ft(lang, "pushed_line"), pushed.join(" ｜ "))?;
            }
            if !repeated.is_empty() {
                writeln!(
                    out,
                    "\n{}{}",
                    self.label("rep_line", &[("n", &repeated.len().to_string())]),
                    repeated.join(" ｜ ")
                )?;
            }
            out.push('\n');
        }
        atomic_write_text(&self.out.join("triage.md"), &out)?;
        self.papers(rows)
    }

    fn papers(&self, rows: &[Cand]) -> Result<()> {
        let lang = self.lang;
        let mut papers: Vec<_> = rows.iter().filter(|c| c.channel == "paper").collect();
        papers.sort_by(|a, b| {
            (std::cmp::Reverse(a.pref), a.also_in.is_empty(), &a.title).cmp(&(
                std::cmp::Reverse(b.pref),
                b.also_in.is_empty(),
                &b.title,
            ))
        });
        let mut out = format!(
            "{}\n\n{}{}\n{}\n\n",
            self.title("title_papers", papers.len()),
            ft(lang, "head_a"),
            if self.full {
                "."
            } else {
                ft(lang, "papers_intro_tail")
            },
            ft(lang, "papers_group_note")
        );
        let preferred: Vec<_> = papers.iter().copied().filter(|c| c.pref >= 3).collect();
        if !preferred.is_empty() {
            writeln!(
                out,
                "## {}{}",
                ft(lang, "group_pref3"),
                cnt_paren(lang, preferred.len())
            )?;
            for c in preferred {
                let mut summary = if self.full { 220 } else { 180 };
                if self.compact && c.pref < 5 {
                    summary = 0;
                }
                if self.full {
                    writeln!(out, "{}", full_line(c, summary))?;
                } else {
                    writeln!(out, "{} [{}]", paper_line(c, summary), c.pref)?;
                }
            }
            out.push('\n');
        }
        let related: Vec<_> = papers
            .iter()
            .copied()
            .filter(|c| (1..=2).contains(&c.pref))
            .collect();
        if !related.is_empty() {
            writeln!(
                out,
                "## {}{}",
                ft(lang, "group_pref12"),
                cnt_paren(lang, related.len())
            )?;
            for weight in [2, 1] {
                if related.iter().any(|c| c.pref == weight) && !self.full {
                    writeln!(
                        out,
                        "### {}",
                        self.label("weight_n", &[("n", &weight.to_string())])
                    )?;
                }
                for c in related.iter().filter(|c| c.pref == weight) {
                    let line = if self.full {
                        full_line(c, 0)
                    } else {
                        paper_line(c, 0)
                    };
                    writeln!(out, "{line}")?;
                }
            }
            out.push('\n');
        }
        let rest: Vec<_> = papers.into_iter().filter(|c| c.pref < 1).collect();
        let rest_file = self.out.join("triage-papers-rest.md");
        if self.fold && !rest.is_empty() {
            let refs = cross_references(rows);
            let mut keep: Vec<_> = rest
                .iter()
                .filter_map(|c| {
                    let tags = paper_signals(c, &refs, lang);
                    (!tags.is_empty()).then_some((*c, tags))
                })
                .collect();
            let folded = rest.len() - keep.len();
            let mut all = format!(
                "{}\n\n",
                self.label(
                    "rest_full",
                    &[
                        ("n", &rest.len().to_string()),
                        ("k", &keep.len().to_string()),
                        ("f", &folded.to_string())
                    ]
                )
            );
            self.by_topic(&mut all, &rest)?;
            atomic_write_text(&rest_file, &all)?;
            if !keep.is_empty() {
                writeln!(
                    out,
                    "## {}",
                    self.label("rest_keep", &[("n", &keep.len().to_string())])
                )?;
                keep.sort_by(|a, b| {
                    (!a.1[0].starts_with("HF"), &a.0.title)
                        .cmp(&(!b.1[0].starts_with("HF"), &b.0.title))
                });
                for (c, tags) in keep {
                    writeln!(out, "{} 〔{}〕", paper_line(c, 0), tags.join("; "))?;
                }
                out.push('\n');
            }
            writeln!(
                out,
                "## {}\n{}",
                self.label("rest_fold", &[("n", &folded.to_string())]),
                self.label("fold_note", &[("path", &rest_file.to_string_lossy())])
            )?;
        } else {
            unlink(&rest_file)?;
            if !rest.is_empty() {
                writeln!(
                    out,
                    "## {}",
                    self.label("rest_plain", &[("n", &rest.len().to_string())])
                )?;
                self.by_topic(&mut out, &rest)?;
            }
        }
        atomic_write_text(&self.out.join("triage-papers.md"), &out)?;
        Ok(())
    }

    fn by_topic(&self, out: &mut String, papers: &[&Cand]) -> Result<()> {
        let mut groups = BTreeMap::<&str, Vec<&Cand>>::new();
        for p in papers {
            groups
                .entry(
                    p.topics
                        .first()
                        .map_or_else(|| ft(self.lang, "other"), String::as_str),
                )
                .or_default()
                .push(p);
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_by_key(|(_, group)| std::cmp::Reverse(group.len()));
        for (name, mut group) in groups {
            writeln!(out, "### {name}{}", cnt_paren(self.lang, group.len()))?;
            group.sort_by(|a, b| a.title.cmp(&b.title));
            for c in group {
                let line = if self.full {
                    full_line(c, 0)
                } else {
                    paper_line(c, 0)
                };
                writeln!(out, "{line}")?;
            }
        }
        Ok(())
    }
}

fn star(c: &Cand) -> &str {
    match status(c) {
        "pushed" => "★ ",
        "seen" => "☆ ",
        _ => "",
    }
}

fn paper_line(c: &Cand, summary: usize) -> String {
    let id = c
        .extra
        .get("arxiv_id")
        .and_then(Value::as_str)
        .unwrap_or(&c.url);
    let hf = if c.source == "hf_daily" || c.also_in.iter().any(|s| s == "hf_daily") {
        " ·HF"
    } else {
        ""
    };
    let mut line = format!("- {id} {}{}{hf}", star(c), c.title);
    if summary > 0 && !c.summary.is_empty() {
        line.push_str(&format!("\n  - {}", short(&c.summary, summary)));
    }
    line
}

fn full_line(c: &Cand, summary: usize) -> String {
    let tops = c.topics[..c.topics.len().min(4)].join(",");
    let star = star(c);
    let details = if let Some(aid) = c
        .extra
        .get("arxiv_id")
        .and_then(Value::as_str)
        .filter(|_| c.channel == "paper")
    {
        let hf = if c.source == "hf_daily" || c.also_in.iter().any(|s| s == "hf_daily") {
            " · HF"
        } else {
            ""
        };
        format!("`{aid}{hf} · {tops}`")
    } else {
        let when = c
            .published
            .as_deref()
            .and_then(|s| s.get(5..16))
            .unwrap_or_default()
            .replace('T', " ");
        let extra = if c.also_in.is_empty() {
            String::new()
        } else {
            format!(" (+{})", c.also_in.join(","))
        };
        format!("`{}{extra} · {when} · {tops}` {}", c.source, c.url)
    };
    let mut line = format!("- {star}[{}] {} — {details}", c.pref, c.title);
    if summary > 0 && !c.summary.is_empty() {
        line.push_str(&format!("\n  - {}", short(&c.summary, summary)));
    }
    line
}

fn read_report(path: &Path) -> Result<FetchReport> {
    let mut value: Value = read_optional(path)?;
    if let Some(meta) = value.as_object_mut() {
        for (k, v) in [
            ("window_start", json!("")),
            ("window_end", json!("")),
            ("generated_at", json!("")),
            ("elapsed_sec", json!(0)),
            ("total_raw", json!(0)),
            ("total_dedup", json!(0)),
            ("notes", json!([])),
            ("sources", json!([])),
        ] {
            meta.entry(k).or_insert(v);
        }
        if let Some(sources) = meta.get_mut("sources").and_then(Value::as_array_mut) {
            for source in sources.iter_mut().filter_map(Value::as_object_mut) {
                for (k, v) in [
                    ("name", json!("")),
                    ("channel", json!("")),
                    ("type", json!("")),
                    ("n", json!(0)),
                    ("sec", json!(0)),
                ] {
                    source.entry(k).or_insert(v);
                }
            }
        }
        Ok(serde_json::from_value(value)?)
    } else {
        Ok(FetchReport::default())
    }
}

fn rebuild(args: &FetchArgs, paths: &DigestPaths, lang: Lang, input: &Path) -> Result<()> {
    let out = args.out.as_deref().unwrap_or(input);
    let rows = load_candidates_jsonl(input)?;
    let mut report = read_report(&input.join("report.json"))?;
    if report.window_start.is_empty() {
        report.window_start = "2000-01-01T00:00+00:00".into();
    }
    if report.window_end.is_empty() {
        report.window_end = "2000-01-02T00:00+00:00".into();
    }
    let date = parse_dt(&report.window_end)
        .and_then(|d| d.checked_sub_signed(Delta::microseconds(1)))
        .map(|d| d.date_naive().to_string())
        .unwrap_or_default();
    let seen = if args.no_state {
        Seen::new()
    } else {
        read_optional(&paths.state.join("triage-seen.json")).unwrap_or_default()
    };
    Triage {
        out,
        lang,
        start: &report.window_start,
        end: &report.window_end,
        day: &date,
        seen: &seen,
        compact: false,
        full: false,
        fold: !args.no_fold,
    }
    .write(&rows)?;
    eprintln!("rebuilt triage → {}", out.display());
    if args.json {
        println!("{}", summary(&report, out, false));
    }
    Ok(())
}

fn summary(report: &FetchReport, out: &Path, reused: bool) -> Value {
    let errors: Vec<_> = report
        .sources
        .iter()
        .filter(|r| r.status != "ok")
        .map(|r| json!({"id": r.id, "status": r.status}))
        .collect();
    let files: BTreeMap<_, _> = ["candidates.jsonl", "triage.md", "report.json", "report.md"]
        .into_iter()
        .map(|name| (name, out.join(name)))
        .collect();
    let mut value = json!({
        "window": [&report.window_start, &report.window_end], "sources": report.sources.len(),
        "raw": report.total_raw, "dedup": report.total_dedup,
        "errors": errors, "out": out, "files": files
    });
    if reused {
        value["reused"] = json!(true);
    }
    value
}

fn archive(out: &Path, paths: &DigestPaths) -> Result<()> {
    let modified = DateTime::<Utc>::from(fs::metadata(out.join("candidates.jsonl"))?.modified()?);
    let prefix = format!(
        "fetch-prev-{}",
        modified.with_timezone(&paths.timezone).format("%m%d-%H%M")
    );
    let mut target = out.with_file_name(&prefix);
    let mut n = 2;
    while target.try_exists()? {
        target = out.with_file_name(format!("{prefix}-{n}"));
        n += 1;
    }
    fs::rename(out, &target)?;
    eprintln!("archived previous fetch → {}", target.display());
    Ok(())
}

fn report_md(report: &FetchReport, lang: Lang) -> Result<String> {
    let mut text = format!(
        "{}\n\n{}\n\n",
        fmt_label(
            lang,
            "report_title",
            &[("start", &report.window_start), ("end", &report.window_end)]
        ),
        fmt_label(
            lang,
            "report_line",
            &[
                ("sec", &format!("{:.1}", report.elapsed_sec)),
                ("raw", &report.total_raw.to_string()),
                ("dd", &report.total_dedup.to_string())
            ]
        )
    );
    for note in &report.notes {
        writeln!(text, "{}{note}", ft(lang, "note_prefix"))?;
    }
    writeln!(
        text,
        "\n{}\n|---|---|---:|---:|---|",
        ft(lang, "report_header")
    )?;
    let mut sources: Vec<_> = report.sources.iter().collect();
    sources.sort_by(|a, b| (&a.channel, &a.id).cmp(&(&b.channel, &b.id)));
    for s in sources {
        writeln!(
            text,
            "| {} | {} | {} | {:.1}s | {} |",
            s.channel,
            s.id,
            s.n,
            s.sec,
            s.status.replace('|', "\\|").replace('\n', " ")
        )?;
    }
    Ok(text)
}

pub async fn run(
    args: &FetchArgs,
    paths: &DigestPaths,
    lang: Lang,
    now: DateTime<Utc>,
) -> Result<()> {
    if let Some(input) = &args.rebuild {
        return rebuild(args, paths, lang, input);
    }
    let window = compute_window(
        paths.timezone,
        now,
        args.weekly,
        args.daily,
        args.day,
        args.start.as_deref(),
        args.end.as_deref(),
    )
    .map_err(FetchArguments)?;
    let automatic = args.only.is_empty() && args.out.is_none();
    let out = args.out.clone().unwrap_or_else(|| {
        paths
            .runs
            .join(&window.label)
            .join(if args.only.is_empty() {
                "fetch"
            } else {
                "fetch-extra"
            })
    });
    let lock_path = out.parent().unwrap_or(Path::new(".")).join(".fetch.lock");
    let _lock = FileLock::acquire(&lock_path)?;
    if args.reuse && automatic {
        if let Ok(r) = read_report(&out.join("report.json")) {
            let fresh = r.window_start == window.iso(window.start)
                && r.window_end == window.iso(window.end)
                && parse_dt(&r.generated_at).is_some_and(|at| at >= window.end)
                && out.join("candidates.jsonl").is_file()
                && out.join("triage.md").is_file();
            if fresh {
                eprintln!(
                    "{}",
                    fmt_label(
                        lang,
                        "fetch_reuse",
                        &[("outdir", &out.to_string_lossy()), ("at", &r.generated_at)]
                    )
                );
                print_notes(&r, lang);
                if args.json {
                    println!("{}", summary(&r, &out, true));
                }
                return Ok(());
            }
        }
    }
    let registry = args.registry.as_deref().unwrap_or(&paths.registry);
    let input =
        fs::read_to_string(registry).with_context(|| format!("read {}", registry.display()))?;
    let (settings, mut sources) = parse_registry(&input)?;
    sources.retain(|s| {
        s.enabled
            && (args.only.is_empty() || args.only.contains(&s.id))
            && !args.skip.contains(&s.id)
    });
    let ledger = load_ledger_keys(&paths.ledger)?;
    if automatic && out.join("candidates.jsonl").is_file() {
        archive(&out, paths)?;
    }
    let state_file = paths.state.join("state.json");
    let seen_file = paths.state.join("triage-seen.json");
    let _state_lock = (!args.no_state)
        .then(|| FileLock::acquire(&paths.state.join(".state.lock")))
        .transpose()?;
    let state: FetchState = if args.no_state {
        FetchState::default()
    } else {
        read_optional(&state_file)?
    };
    let mut ctx = SourceContext::new(window, &settings, lang, state);
    ctx.now = now;
    ctx.use_state = !args.no_state;
    ctx.catchup = args.catchup && !args.no_catchup;
    ctx.daily_board_state = !(args.weekly
        || (!args.daily && args.day.is_none() && args.start.is_some() && args.end.is_some()))
        && args.only.is_empty();
    ctx.wait_arxiv = Duration::from_secs(args.wait_arxiv.saturating_mul(60));
    let full = args.weekly || args.full_triage || args.compact;
    let seen: Seen = if full || args.no_state {
        Seen::new()
    } else {
        read_optional(&seen_file).unwrap_or_default()
    };
    let Window {
        start, end, label, ..
    } = &ctx.window;
    eprintln!(
        "window {} → {}  sources={}  out={}",
        ctx.window.iso(*start),
        ctx.window.iso(*end),
        sources.len(),
        out.display()
    );
    let timer = Instant::now();
    let semaphore = tokio::sync::Semaphore::new(settings.workers);
    let mut results = stream::iter(sources.iter().enumerate())
        .map(|(index, src)| {
            let (ctx, semaphore) = (&ctx, &semaphore);
            async move {
                let _permit = semaphore
                    .acquire()
                    .await
                    .expect("fetch semaphore stays open");
                let started = Instant::now();
                let (cands, status) = match fetch_source(ctx, src).await {
                    Ok(rows) => (rows, "ok".into()),
                    Err(e) => (Vec::new(), format!("ERROR {}", short(&e.to_string(), 160))),
                };
                let report = FetchSourceReport {
                    id: src.id.clone(),
                    name: src.name.clone(),
                    channel: src.channel.clone(),
                    source_type: src.source_type.clone(),
                    n: cands.len(),
                    sec: (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
                    status,
                };
                if args.verbose_sources || report.status != "ok" {
                    eprintln!(
                        "  {:<24} {:>4} {:>5.1}s  {}",
                        report.id, report.n, report.sec, report.status
                    );
                }
                (index, cands, report)
            }
        })
        .buffer_unordered(settings.workers)
        .collect::<Vec<_>>()
        .await;
    results.sort_by_key(|(index, _, _)| *index);
    let (raw, report_sources): (Vec<_>, Vec<_>) =
        results.into_iter().map(|(_, rows, r)| (rows, r)).unzip();
    let mut raw: Vec<_> = raw.into_iter().flatten().collect();
    let total_raw = raw.len();
    let preferences = load_preferences(&paths.preferences);
    for row in &mut raw {
        score_topics(row, &preferences);
    }
    let mut rows = dedupe(raw);
    for row in &mut rows {
        if let Some(s) = ledger
            .get(&candidate_key(row))
            .or_else(|| ledger.get(&format!("t:{}", norm_title(&row.title))))
        {
            row.in_ledger = InLedger::Status((*s).into());
        }
    }
    let report = FetchReport {
        window_start: ctx.window.iso(*start),
        window_end: ctx.window.iso(*end),
        generated_at: ctx.window.iso(now + Delta::from_std(timer.elapsed())?),
        elapsed_sec: (timer.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        total_raw,
        total_dedup: rows.len(),
        notes: ctx.notes.lock().unwrap().clone(),
        sources: report_sources,
    };
    let mut jsonl = String::new();
    for row in &rows {
        writeln!(jsonl, "{}", serde_json::to_string(row)?)?;
    }
    unlink(&out.join("report.json"))?;
    atomic_write_text(&out.join("candidates.jsonl"), &jsonl)?;
    Triage {
        out: &out,
        lang,
        start: &report.window_start,
        end: &report.window_end,
        day: label,
        seen: &seen,
        compact: args.compact || args.weekly,
        full,
        fold: !full && !args.no_fold,
    }
    .write(&rows)?;
    atomic_write_text(&out.join("report.md"), &report_md(&report, lang)?)?;
    let partial = out.join("report.json.partial");
    atomic_write_text(&partial, &serde_json::to_string_pretty(&report)?)?;
    if !args.no_state {
        if !full && args.only.is_empty() {
            update_seen(&seen_file, seen, &rows, label)?;
        }
        atomic_write_text(
            &state_file,
            &serde_json::to_string_pretty(&*ctx.state.lock().unwrap())?,
        )?;
    }
    fs::rename(partial, out.join("report.json"))?;
    eprintln!(
        "done in {:.1}s: raw={} dedup={} errors={} → {}",
        report.elapsed_sec,
        report.total_raw,
        report.total_dedup,
        report.sources.iter().filter(|r| r.status != "ok").count(),
        out.join("candidates.jsonl").display()
    );
    print_notes(&report, lang);
    if args.json {
        println!("{}", summary(&report, &out, false));
    }
    Ok(())
}

fn print_notes(report: &FetchReport, lang: Lang) {
    for r in report.sources.iter().filter(|r| r.status != "ok") {
        eprintln!("  ! {}: {}", r.id, r.status);
    }
    for note in &report.notes {
        eprintln!("{}{note}", ft(lang, "note_prefix"));
    }
}

fn update_seen(file: &Path, mut seen: Seen, rows: &[Cand], label: &str) -> Result<()> {
    for c in rows.iter().filter(|c| discovery(c)) {
        seen.entry(candidate_key(c)).or_insert_with(|| label.into());
    }
    let cutoff = label
        .parse::<NaiveDate>()?
        .checked_sub_days(Days::new(45))
        .unwrap_or(NaiveDate::MIN)
        .to_string();
    seen.retain(|_, day| *day >= cutoff);
    atomic_write_text(file, &serde_json::to_string(&seen)?)?;
    Ok(())
}
