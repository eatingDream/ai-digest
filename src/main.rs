use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::LazyLock,
};

use ai_digest::{
    fetch::{pipeline, tools},
    i18n::{self, ft, Lang},
    ledger::{self, AddOptions, LedgerReadError},
    model::{atomic_write_text, DigestPaths},
    parser::parse_digest_md,
    publish, render, verify, version,
};
use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, Utc};
use clap::{error::ErrorKind, Args, Parser, Subcommand};
use regex::Regex;
use serde_json::{json, Map, Value};

#[derive(Parser)]
#[command(name = "ai-digest", version = version::VERSION, disable_help_subcommand = true,
    about = "Fetch, render, publish, and verify AI digests",
    after_help = "Examples:\n  ai-digest fetch --daily --reuse\n  ai-digest publish --demo --out dist\n  ai-digest verify --local")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(version = version::VERSION, about = "Fetch candidates and write triage reports",
        after_help = "Examples:\n  ai-digest fetch --daily --day 2026-01-15 --reuse\n  ai-digest fetch --weekly --json")]
    Fetch(pipeline::FetchArgs),
    #[command(version = version::VERSION, alias = "render-html", about = "Render a Markdown digest as standalone HTML",
        after_help = "Examples:\n  ai-digest render digest-2026-01-15.md issue.html --lang en\n  ai-digest render issue.md issue.html --md-en issue.en.md --json")]
    Render(RenderArgs),
    #[command(version = version::VERSION, about = "Prepare and deploy the static site",
        after_help = "Examples:\n  ai-digest publish --demo --out dist\n  ai-digest publish --issue daily/2026-01-15 --md digest-2026-01-15.md\n  ai-digest publish --rebuild --prepare-only")]
    Publish(publish::PublishArgs),
    #[command(version = version::VERSION, about = "Read or update the digest ledger",
        after_help = "Examples:\n  ai-digest ledger find 2601.01234\n  ai-digest ledger add --day 2026-01-15 < batch.txt")]
    Ledger(LedgerArgs),
    #[command(version = version::VERSION, about = "Look up fetched candidates",
        after_help = "Examples:\n  ai-digest cand 2601.01234 --since 2026-01-12\n  ai-digest cand '#abcdef' --dir runs/2026-01-15/fetch")]
    Cand(tools::CandArgs),
    #[command(version = version::VERSION, alias = "check_cands", about = "Validate the paper shortlist",
        after_help = "Examples:\n  ai-digest check-cands runs/2026-01-15/paper-cands.md.tmp --finalize --json")]
    CheckCands(tools::CheckArgs),
    #[command(version = version::VERSION, about = "Fetch concise primary-source excerpts",
        after_help = "Examples:\n  ai-digest page https://example.com/article --meta\n  ai-digest page https://example.com/article@@benchmark --ctx 80")]
    Page(tools::PageArgs),
    #[command(version = version::VERSION, alias = "weekly_late", about = "List late signals from existing weekly runs",
        after_help = "Examples:\n  ai-digest weekly-late --day 2026-01-15\n  ai-digest weekly-late --start 2026-01-12 --end 2026-01-18 --all")]
    WeeklyLate(tools::WeeklyArgs),
    #[command(version = version::VERSION, about = "Compact saved X search results",
        after_help = "Examples:\n  ai-digest xparse A1=results.txt B1=more.json --out x-raw.md")]
    Xparse(tools::XparseArgs),
    #[command(version = version::VERSION, alias = "verify-decrypt", about = "Compare public or encrypted pages with plaintext",
        after_help = "Examples:\n  ai-digest verify --local\n  ai-digest verify --url-base https://digest.example.com\n  ai-digest verify --url https://digest.example.com/latest --plain site/plain/latest.html")]
    Verify(verify::VerifyArgs),
}

#[derive(Args)]
struct RenderArgs {
    #[arg(value_name = "INPUT")]
    input: PathBuf,
    #[arg(value_name = "OUTPUT")]
    output: PathBuf,
    #[arg(long, help = "Output language (en or zh-CN)")]
    lang: Option<String>,
    #[arg(
        long,
        value_name = "MARKDOWN",
        help = "Bundle an English companion in the same page"
    )]
    md_en: Option<PathBuf>,
    #[arg(long, help = "Specify the year for headings that omit it")]
    year: Option<i32>,
    #[arg(long, help = "Print the render summary as JSON")]
    json: bool,
}

#[derive(Args)]
struct LedgerArgs {
    #[arg(long, global = true, help = "Output language (en or zh-CN)")]
    lang: Option<String>,
    #[command(subcommand)]
    command: LedgerCmd,
}

#[derive(Subcommand)]
enum LedgerCmd {
    #[command(version = version::VERSION, after_help = "Examples:\n  ai-digest ledger find 2601.01234 --json")]
    Find {
        #[arg(required = true, value_name = "KEY")]
        keys: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    #[command(version = version::VERSION, after_help = "Examples:\n  ai-digest ledger recent --days 21 --status seen")]
    Recent {
        #[arg(long, default_value_t = 21)]
        days: u32,
        #[arg(long, value_parser = ["all", "pushed", "seen"], default_value = "all")]
        status: String,
        #[arg(long)]
        cat: Option<String>,
        #[arg(long)]
        json: bool,
    },
    #[command(version = version::VERSION, after_help = "Examples:\n  ai-digest ledger add --day 2026-01-15 < batch.txt")]
    Add {
        #[arg(long, value_name = "YYYY-MM-DD")]
        day: NaiveDate,
        #[arg(long, value_name = "FILE")]
        log: Option<PathBuf>,
        #[arg(long)]
        log_heading: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

fn has_repo_markers(p: &Path) -> bool {
    p.join("fetch/sources.toml").is_file() || p.join("examples/sample-digest.en.md").is_file()
}

fn code_root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe
            .ancestors()
            .skip(1)
            .take(4)
            .find(|p| has_repo_markers(p))
        {
            return root.to_owned();
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = cwd.ancestors().take(4).find(|p| has_repo_markers(p)) {
            return root.to_owned();
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return if dir.file_name().is_some_and(|s| s == "bin") {
                dir.parent().unwrap_or(dir).to_owned()
            } else {
                dir.to_owned()
            };
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn output_language(cli: Option<&str>, paths: &DigestPaths) -> Result<Lang> {
    let config = fs::read_to_string(&paths.config).ok().and_then(|s| {
        s.lines()
            .filter_map(|line| line.split_once('='))
            .filter(|(key, _)| key.trim() == "LANG")
            .map(|(_, val)| val.trim().to_owned())
            .next_back()
    });
    Ok(i18n::resolve_lang(cli, config.as_deref())?)
}

fn render_cmd(args: &RenderArgs, paths: &DigestPaths) -> Result<()> {
    static YEAR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([0-9]{4})-[0-9]{2}-[0-9]{2}").unwrap());
    let lang = output_language(args.lang.as_deref(), paths)?;
    let year = args
        .year
        .or_else(|| {
            args.input
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(|s| YEAR.captures(s))
                .and_then(|m| m[1].parse().ok())
        })
        .unwrap_or_else(|| Utc::now().with_timezone(&paths.timezone).year());
    let lang = if args.md_en.is_some() {
        Lang::ZhCn
    } else {
        lang
    };
    let input = fs::read_to_string(&args.input)
        .with_context(|| format!("read {}", args.input.display()))?;
    let digest = parse_digest_md(&input, lang, Some(year))
        .with_context(|| format!("parse {}", args.input.display()))?;
    let opts = render::RenderOptions::default();
    let items = digest.items().count();
    let html = if let Some(en) = &args.md_en {
        let input = fs::read_to_string(en)
            .with_context(|| format!("read English Markdown {}", en.display()))?;
        let english = parse_digest_md(&input, Lang::En, Some(year))
            .with_context(|| format!("parse {}", en.display()))?;
        let en_items = english.items().count();
        if items != en_items {
            eprintln!("WARNING: English has {en_items} items vs Chinese {items}");
        }
        render::render_bilingual_html(&digest, &english, &opts)
    } else {
        render::render_digest_html(&digest, lang, &opts)
    };
    atomic_write_text(&args.output, &html)?;
    if args.json {
        let mut summary = json!({"output": args.output, "lang": lang, "title": digest.title, "items": items, "sections": digest.sections.len()});
        if args.md_en.is_some() {
            summary["bilingual"] = json!(true);
        }
        println!("{summary}");
    } else {
        let suffix = if args.md_en.is_some() {
            ", bilingual"
        } else {
            ""
        };
        println!("wrote {} ({items} items{suffix})", args.output.display());
    }
    Ok(())
}

fn ledger_field<'a>(item: &'a Map<String, Value>, field: &str) -> &'a str {
    item.get(field).and_then(Value::as_str).unwrap_or_default()
}

fn print_ledger_line(item: &Map<String, Value>, prefix: &str) {
    let day = item
        .get("first_seen")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let pushed = ["pushed", "last_pushed"]
        .into_iter()
        .map(|key| ledger_field(item, key))
        .find(|s| !s.is_empty())
        .map_or_else(String::new, |day| format!(" | pushed {day}"));
    let title: String = ledger_field(item, "name").chars().take(70).collect();
    println!(
        "{prefix}{day} | {:6} | {} | {title}{pushed}",
        ledger::ledger_status(item),
        ledger_field(item, "id")
    );
}

fn ledger_cmd(args: &LedgerArgs, paths: &DigestPaths) -> Result<()> {
    let lang = output_language(args.lang.as_deref(), paths)?;
    match &args.command {
        LedgerCmd::Find {
            keys,
            json: output_json,
        } => {
            let ledger = ledger::load_ledger(&paths.ledger)?;
            let mut queries = Map::new();
            for key in keys {
                let hits = ledger::find_items(&ledger, key);
                if *output_json {
                    queries.insert(key.to_owned(), json!(hits));
                } else {
                    println!(
                        "{}",
                        ft(lang, "lg_hits")
                            .replace("{k}", key)
                            .replace("{n}", &hits.len().to_string())
                    );
                    for hit in hits.into_iter().take(8) {
                        print_ledger_line(hit, "  ");
                    }
                }
            }
            if *output_json {
                println!("{}", json!({"query": queries}));
            }
        }
        LedgerCmd::Recent {
            days,
            status,
            cat,
            json: output_json,
        } => {
            let ledger = ledger::load_ledger(&paths.ledger)?;
            let today = Utc::now().with_timezone(&paths.timezone).date_naive();
            let items = ledger::recent_items(&ledger, today, *days, status, cat.as_deref());
            if *output_json {
                println!(
                    "{}",
                    json!({"days": days, "status": status, "count": items.len(), "items": items})
                );
            } else {
                println!(
                    "{}",
                    ft(lang, "lg_recent")
                        .replace("{days}", &days.to_string())
                        .replace("{n}", &items.len().to_string())
                        .replace("{status}", status)
                );
                for item in items {
                    print_ledger_line(item, "");
                }
            }
        }
        LedgerCmd::Add {
            day,
            log,
            log_heading,
            json: output_json,
        } => {
            let mut batch = String::new();
            io::stdin().read_to_string(&mut batch)?;
            let result = ledger::add_items(
                paths,
                *day,
                &batch,
                AddOptions {
                    lang,
                    log: log.as_deref(),
                    log_heading: log_heading.as_deref(),
                },
            )?;
            if *output_json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!(
                    "ledger: +{} new, {} updated, {} total",
                    result.added, result.updated, result.total
                );
            }
        }
    }
    Ok(())
}

async fn run(command: Command) -> Result<u8> {
    let root = code_root();
    let paths = DigestPaths::from_env(&root).map_err(InvalidEnvironment)?;
    match command {
        Command::Fetch(args) => {
            let lang = output_language(args.lang.as_deref(), &paths)?;
            pipeline::run(&args, &paths, lang, Utc::now()).await?;
        }
        Command::Render(args) => render_cmd(&args, &paths)?,
        Command::Publish(args) => publish::run(&args, &root, paths).await?,
        Command::Ledger(args) => ledger_cmd(&args, &paths)?,
        Command::Cand(args) => {
            let lang = output_language(args.lang.as_deref(), &paths)?;
            print!("{}", tools::candidate_lookup(&args, &paths, lang)?);
        }
        Command::CheckCands(args) => {
            let lang = output_language(args.lang.as_deref(), &paths)?;
            let result = tools::check_candidates(&args, &paths, lang)?;
            if args.json {
                println!("{}", serde_json::to_string(&result)?);
            } else {
                println!("{}", result.display(lang));
            }
            return Ok(u8::from(!result.ok));
        }
        Command::Page(args) => {
            let lang = output_language(args.lang.as_deref(), &paths)?;
            let (ok, out) = tools::pages(&args, lang).await;
            println!("{out}");
            return Ok(u8::from(!ok));
        }
        Command::WeeklyLate(args) => {
            let lang = output_language(args.lang.as_deref(), &paths)?;
            if args
                .start
                .zip(args.end)
                .is_some_and(|(start, end)| start > end)
            {
                return Err(
                    InvalidEnvironment(anyhow::anyhow!("invalid weekly date range")).into(),
                );
            }
            print!("{}", tools::weekly_late(&args, &paths, lang, Utc::now())?);
        }
        Command::Xparse(args) => {
            let lang = output_language(args.lang.as_deref(), &paths)?;
            print!("{}", tools::xparse(&args, lang, paths.timezone)?);
        }
        Command::Verify(args) => {
            let config = publish::load_config(&paths.config, None)?;
            verify::run(&args, &paths, config.encrypt).await?;
        }
    }
    Ok(0)
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct InvalidEnvironment(#[source] anyhow::Error);

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if e.kind() == ErrorKind::DisplayVersion => {
            println!("{}", version::version_string());
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            let code = e.exit_code();
            let _ = e.print();
            return ExitCode::from(code as u8);
        }
    };
    match run(cli.command).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            let usage = error.is::<i18n::LangError>()
                || error.is::<LedgerReadError>()
                || error.is::<pipeline::FetchArguments>()
                || error.is::<tools::WeeklyArguments>()
                || error.is::<publish::MissingMode>()
                || error.is::<publish::MissingToken>()
                || error.is::<verify::NoTargets>()
                || error.is::<verify::UnpairedUrls>()
                || error.is::<InvalidEnvironment>();
            ExitCode::from(if usage { 2 } else { 1 })
        }
    }
}
