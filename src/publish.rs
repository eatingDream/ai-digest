use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    process::Stdio,
    sync::LazyLock,
    time::Duration,
};

use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use clap::Args;
use rand::{rngs::OsRng, seq::SliceRandom, RngCore};
use regex::{Captures, Regex};
use reqwest::{
    multipart::{Form, Part},
    Client, Method, StatusCode,
};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::assets;
use crate::crypto::{self, ScriptAllowlist, UnlockOptions};
use crate::geo::{self, GeoPage, PageType};
use crate::i18n::{self, Lang};
use crate::model::{
    atomic_write_bytes, atomic_write_text, canon_url, files_under, full_width_len,
    is_wrapper_or_shortener_url, safe_external_url, strip_tracking_params, Digest, DigestPaths,
    FileLock, LangAlt, LinkItem, ParsedIssue, SectionEntry, SiteConfig, SiteContext,
};
use crate::net::SafeHttpClient;
use crate::parser::{parse_digest_html_with_lang, parse_digest_md};
use crate::render::{self, ArchiveIssue, NavOptions, RenderOptions};
use crate::verify;

const CF_API: &str = "https://api.cloudflare.com/client/v4/";
const WRANGLER_VERSION: &str = "4.166.0";
const CONTROL_FILES: &[(&str, &str)] = &[
    ("_headers", "text/plain"),
    ("_redirects", "text/plain"),
    ("_worker.js", "application/javascript"),
    ("_routes.json", "application/json"),
];
const DEMO_DAILY: &str = "2026-01-15";
const DEMO_WEEKLY: &str = "2026-01-12_01-18";

#[derive(Args, Debug)]
pub struct PublishArgs {
    #[arg(value_name = "HTML", requires_all = ["kind", "date_label"])]
    pub html: Option<PathBuf>,
    #[arg(value_parser = ["daily", "weekly"], requires = "html")]
    pub kind: Option<String>,
    #[arg(requires = "html")]
    pub date_label: Option<String>,
    #[arg(long, help = "Create site/CONFIG and print the initial settings")]
    pub init: bool,
    #[arg(
        long,
        requires = "init",
        help = "Initialize an encrypted site with a random slug"
    )]
    pub encrypt: bool,
    #[arg(long, conflicts_with_all = ["init", "remove", "deploy_only", "demo", "html", "rerender_all", "rotate_password"], value_name = "KIND/LABEL")]
    pub issue: Option<String>,
    #[arg(long, requires = "issue")]
    pub md: Option<PathBuf>,
    #[arg(
        long,
        requires = "md",
        help = "English companion Markdown for the issue"
    )]
    pub md_en: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["init", "deploy_only", "demo", "html", "rerender_all", "rotate_password"], value_name = "KIND/LABEL")]
    pub remove: Option<String>,
    #[arg(long)]
    pub rebuild: bool,
    #[arg(long)]
    pub rerender_all: bool,
    #[arg(long, conflicts_with_all = ["html", "rebuild", "rerender_all", "rotate_password", "init", "demo"])]
    pub deploy_only: bool,
    #[arg(
        long,
        alias = "dry-run",
        help = "Prepare the site without deploying it"
    )]
    pub prepare_only: bool,
    #[arg(long, help = "Replace the password and salt and re-encrypt the site")]
    pub rotate_password: bool,
    #[arg(long, help = "Skip local and deployed byte-for-byte verification")]
    pub no_verify: bool,
    #[arg(
        long,
        help = "Verify external URLs in --md via live HTTP checks before publishing"
    )]
    pub verify_urls: bool,
    #[arg(long, help = "Also write incremental progress to stderr")]
    pub verbose: bool,
    #[arg(long, conflicts_with_all = ["init", "html", "rebuild", "rerender_all", "rotate_password"], help = "Build an offline example site")]
    pub demo: bool,
    #[arg(long, requires = "demo", default_value = "dist")]
    pub out: PathBuf,
    #[arg(long, requires = "demo")]
    pub examples: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Output language (en, zh-CN, or all for a demo)"
    )]
    pub lang: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("no Cloudflare API token; the site was prepared without deploying")]
pub struct MissingToken;

#[derive(Debug, thiserror::Error)]
#[error("select a publish mode or supply HTML, kind, and date label")]
pub struct MissingMode;

pub fn load_config(path: &Path, lang: Option<&str>) -> Result<SiteConfig> {
    SiteConfig::load(path, lang)
}

fn valid_config_value(s: &str) -> bool {
    !s.contains(['\n', '\r'])
}

/// The caller holds the site lock while initializing its configuration.
pub fn init_config(paths: &DigestPaths, lang: Option<&str>, encrypt: bool) -> Result<SiteConfig> {
    if paths.config.try_exists()? {
        let cfg = load_config(&paths.config, lang)?;
        if encrypt {
            ensure!(
                cfg.encrypt,
                "existing site/CONFIG or DIGEST_ENCRYPT resolves to ENCRYPT=0, conflicting with --init --encrypt"
            );
        }
        return Ok(cfg);
    }
    if !encrypt && env::var("SITE_SLUG").is_ok_and(|s| !s.trim().is_empty()) {
        return load_config(&paths.config, lang);
    }
    let mut cfg = load_config(&paths.config, lang)?;
    if encrypt {
        if env::var("DIGEST_ENCRYPT").is_ok_and(|v| !v.trim().is_empty()) {
            ensure!(
                cfg.encrypt,
                "DIGEST_ENCRYPT=0 conflicts with --init --encrypt"
            );
        }
        cfg.encrypt = true;
        if cfg.slug.is_empty() {
            const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyz23456789";
            cfg.slug = (0..22)
                .map(|_| char::from(*ALPHABET.choose(&mut OsRng).expect("nonempty slug alphabet")))
                .collect();
        }
    }
    let data = [
        ("PLATFORM", cfg.platform.as_str()),
        ("PROJECT", &cfg.project),
        ("ACCOUNT_ID", &cfg.account_id),
        ("ZONE_ID", &cfg.zone_id),
        ("DOMAIN", &cfg.domain),
        ("SLUG", &cfg.slug),
        ("ENCRYPT", if cfg.encrypt { "1" } else { "0" }),
        ("LANG", cfg.lang.as_str()),
    ];
    let mut text = String::new();
    for (key, val) in data {
        ensure!(valid_config_value(val), "invalid config value for {key}");
        writeln!(text, "{key}={val}")?;
    }
    atomic_write_text(&paths.config, &text)?;
    let effective = load_config(&paths.config, lang)?;
    if encrypt {
        ensure!(
            effective.encrypt,
            "initialized configuration did not resolve to ENCRYPT=1"
        );
    }
    Ok(effective)
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IssueRef {
    kind: String,
    label: String,
    start: NaiveDate,
    end: NaiveDate,
}

impl IssueRef {
    fn parse(input: &str) -> Result<Self> {
        let input = input.trim().trim_matches('/');
        let input = input.strip_suffix(".html").unwrap_or(input);
        let (kind, label) = input
            .split_once('/')
            .context("expected daily/YYYY-MM-DD or weekly/YYYY-MM-DD_MM-DD")?;
        let (start, end) = match kind {
            "daily" => {
                let d = parse_daily_label(label)?;
                (d, d)
            }
            "weekly" => parse_weekly_label(label)?,
            _ => bail!("expected daily or weekly issue"),
        };
        Ok(Self {
            kind: kind.into(),
            label: label.into(),
            start,
            end,
        })
    }

    fn rel(&self) -> String {
        format!("{}/{}", self.kind, self.label)
    }
    fn file(&self) -> String {
        format!("{}.html", self.rel())
    }
    fn name(&self) -> String {
        format!("{}-{}", self.kind, self.label)
    }
    fn md_name(&self) -> String {
        let prefix = if self.kind == "daily" {
            "digest"
        } else {
            "weekly"
        };
        format!("{prefix}-{}.md", self.label)
    }
}

fn parse_daily_label(label: &str) -> Result<NaiveDate> {
    static DATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$").unwrap());
    ensure!(DATE.is_match(label), "daily date label must be YYYY-MM-DD");
    Ok(NaiveDate::parse_from_str(label, "%Y-%m-%d")?)
}

pub fn parse_weekly_label(label: &str) -> Result<(NaiveDate, NaiveDate)> {
    static SHORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{2}-[0-9]{2}$").unwrap());
    let (start, end) = label
        .split_once('_')
        .context("weekly date label must be YYYY-MM-DD_MM-DD")?;
    let start = parse_daily_label(start)?;
    let end = if SHORT.is_match(end) {
        let year = start.year() + i32::from(start.format("%m-%d").to_string().as_str() > end);
        parse_daily_label(&format!("{year:04}-{end}"))?
    } else {
        parse_daily_label(end)?
    };
    ensure!(start <= end, "weekly end precedes start");
    Ok((start, end))
}

pub fn discover_english_md(
    md: &Path,
    explicit: Option<&Path>,
    lang: Lang,
) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        ensure!(
            path.is_file(),
            "English markdown not found: {}",
            path.display()
        );
        return Ok(Some(path.to_owned()));
    }
    if lang == Lang::ZhCn {
        if let Some(stem) = md
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.ends_with(".en"))
        {
            let companion = md.with_file_name(format!("{stem}.en.md"));
            if companion.is_file() {
                return Ok(Some(companion));
            }
        }
    }
    Ok(None)
}

fn extract_markdown_external_urls(text: &str) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let mut urls = Vec::new();
    for event in Parser::new_ext(text, Options::empty()) {
        if let Event::Start(Tag::Link { dest_url, .. }) = event {
            let u = dest_url.trim();
            if u.contains(':') || u.starts_with("//") {
                urls.push(u.to_owned());
            }
        }
    }
    urls
}

fn load_grounded_urls(runs_dir: &Path, issue_label: &str) -> BTreeSet<String> {
    let mut grounded = BTreeSet::new();
    let run_dir = runs_dir.join(issue_label);
    let cand_file = run_dir.join("fetch/candidates.jsonl");
    if let Ok(text) = fs::read_to_string(&cand_file) {
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(u) = v.get("url").and_then(Value::as_str) {
                grounded.insert(canon_url(u));
            }
            if let Some(extra) = v.get("extra").and_then(Value::as_object) {
                for key in ["github", "project", "url"] {
                    if let Some(u) = extra.get(key).and_then(Value::as_str) {
                        grounded.insert(canon_url(u));
                    }
                }
            }
        }
    }
    let x_raw = run_dir.join("x-raw.md");
    if let Ok(text) = fs::read_to_string(&x_raw) {
        static RAW_URL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"https?://[^\s<>)"'`]+"#).unwrap());
        for m in RAW_URL.find_iter(&text) {
            grounded.insert(canon_url(m.as_str()));
        }
    }
    grounded
}

fn sanitize_and_validate_digest(
    digest: &mut Digest,
    runs_dir: Option<&Path>,
    issue_label: Option<&str>,
    strict_urls: bool,
) -> Result<Vec<String>> {
    let mut external_urls = Vec::new();
    let mut seen_urls = BTreeSet::new();
    for section in &mut digest.sections {
        for entry in &mut section.entries {
            let SectionEntry::Item(item) = entry else {
                continue;
            };
            let fw = full_width_len(&item.title);
            if fw > 30.0 {
                eprintln!(
                    "warn: item #{} title exceeds 30 full-width chars ({fw:.1} FW): {}",
                    item.number, item.title
                );
            }
            for link in &mut item.links {
                let Some(safe) = safe_external_url(&link.url) else {
                    if strict_urls {
                        bail!(
                            "unsafe or non-HTTP(S) URL in item #{}: {}",
                            item.number,
                            link.url
                        );
                    }
                    continue;
                };
                if strict_urls && is_wrapper_or_shortener_url(&safe) {
                    bail!(
                        "redirect-wrapper/shortener URL rejected in item #{}: {safe}",
                        item.number
                    );
                }
                let cleaned = strip_tracking_params(&safe);
                link.url = cleaned.clone();
                if seen_urls.insert(cleaned.clone()) {
                    external_urls.push(cleaned);
                }
            }
            for field in [
                &item.dek,
                &item.look,
                &item.detail,
                &item.followup,
                &item.extra,
            ] {
                for raw in extract_markdown_external_urls(field) {
                    let Some(safe) = safe_external_url(&raw) else {
                        if strict_urls {
                            bail!(
                                "unsafe or non-HTTP(S) prose URL in item #{}: {raw}",
                                item.number
                            );
                        }
                        continue;
                    };
                    if strict_urls && is_wrapper_or_shortener_url(&safe) {
                        bail!(
                            "redirect-wrapper/shortener prose URL rejected in item #{}: {safe}",
                            item.number
                        );
                    }
                    let cleaned = strip_tracking_params(&safe);
                    if seen_urls.insert(cleaned.clone()) {
                        external_urls.push(cleaned);
                    }
                }
            }
        }
    }
    if let (Some(runs), Some(label)) = (runs_dir, issue_label) {
        let grounded = load_grounded_urls(runs, label);
        if !grounded.is_empty() {
            for u in &external_urls {
                if !grounded.contains(&canon_url(u)) {
                    eprintln!("warn: external URL not found in runs/{label} candidates: {u}");
                }
            }
        }
    }
    Ok(external_urls)
}

async fn verify_external_urls_live(urls: &[String]) -> Result<()> {
    let client = SafeHttpClient::new(
        Duration::from_secs(20),
        1,
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/124 Safari/537.36",
    );
    let headers = reqwest::header::HeaderMap::new();
    for raw in urls {
        let parsed = Url::parse(raw).with_context(|| format!("invalid external URL: {raw}"))?;
        let resp = client
            .get_with_headers(&parsed, None, &headers)
            .await
            .with_context(|| format!("live URL check failed for {raw}"))?;
        ensure!(
            resp.status().is_success(),
            "live URL check returned HTTP {} for {raw}",
            resp.status()
        );
    }
    Ok(())
}

fn read_md(path: &Path, lang: Lang, year: i32) -> Result<Digest> {
    let input =
        fs::read_to_string(path).with_context(|| format!("read markdown {}", path.display()))?;
    let mut digest = parse_digest_md(&input, lang, Some(year))
        .with_context(|| format!("parse markdown {}", path.display()))?;
    let _ = sanitize_and_validate_digest(&mut digest, None, None, false)?;
    Ok(digest)
}

async fn render_issue_inputs(
    issue: &IssueRef,
    md: &Path,
    english: Option<&Path>,
    lang: Lang,
    runs_dir: &Path,
    verify_urls: bool,
) -> Result<(String, Option<String>, usize)> {
    let input =
        fs::read_to_string(md).with_context(|| format!("read markdown {}", md.display()))?;
    let mut primary = parse_digest_md(&input, lang, Some(issue.start.year()))
        .with_context(|| format!("parse markdown {}", md.display()))?;
    let mut urls =
        sanitize_and_validate_digest(&mut primary, Some(runs_dir), Some(&issue.label), true)?;
    let mut en = english
        .map(|path| -> Result<Digest> {
            let text = fs::read_to_string(path)
                .with_context(|| format!("read markdown {}", path.display()))?;
            let mut doc = parse_digest_md(&text, Lang::En, Some(issue.start.year()))
                .with_context(|| format!("parse markdown {}", path.display()))?;
            urls.extend(sanitize_and_validate_digest(
                &mut doc,
                Some(runs_dir),
                Some(&issue.label),
                true,
            )?);
            Ok(doc)
        })
        .transpose()?;
    if verify_urls {
        let unique: Vec<String> = urls
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        verify_external_urls_live(&unique).await?;
    }
    let count = primary.items().count();
    let opts = RenderOptions::default();
    if lang == Lang::ZhCn {
        Ok((
            render::render_digest_html(&primary, lang, &opts),
            en.take()
                .map(|doc| render::render_digest_html(&doc, Lang::En, &opts)),
            count,
        ))
    } else if let Some(en) = en {
        Ok((
            render::render_bilingual_html(&primary, &en, &opts),
            None,
            count,
        ))
    } else {
        Ok((
            render::render_digest_html(&primary, lang, &opts),
            None,
            count,
        ))
    }
}

fn resolve_input(root: &Path, input: &Path) -> PathBuf {
    if input.is_absolute() || input.is_file() {
        input.to_owned()
    } else {
        root.join(input)
    }
}

fn legacy_html_path(paths: &DigestPaths, issue: &IssueRef, english: bool) -> PathBuf {
    paths.html.join(format!(
        "{}{}.html",
        issue.name(),
        if english { ".en" } else { "" }
    ))
}

fn source_path(paths: &DigestPaths, issue: &IssueRef, english: bool) -> PathBuf {
    if english {
        paths.sources.join("en").join(issue.file())
    } else {
        paths.sources.join(issue.file())
    }
}

fn save_source(paths: &DigestPaths, issue: &IssueRef, english: bool, html: &str) -> Result<()> {
    atomic_write_text(
        &source_path(paths, issue, english),
        &render::strip_injections(html),
    )?;
    Ok(())
}

#[derive(Clone)]
struct SourcePage {
    issue: IssueRef,
    html: String,
    parsed: ParsedIssue,
}

fn source_refs(base: &Path) -> Result<Vec<(IssueRef, PathBuf)>> {
    let mut out = Vec::new();
    for kind in ["daily", "weekly"] {
        for path in files_under(&base.join(kind))? {
            let Some(label) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("html")
                || path.parent() != Some(base.join(kind).as_path())
            {
                continue;
            }
            if let Ok(issue) = IssueRef::parse(&format!("{kind}/{label}")) {
                out.push((issue, path));
            }
        }
    }
    out.sort_by_key(|(issue, _)| (issue.end, issue.kind.clone(), issue.start));
    Ok(out)
}

fn source_pages(base: &Path, lang: Lang) -> Result<Vec<SourcePage>> {
    source_refs(base)?
        .into_iter()
        .map(|(issue, path)| {
            let html = clean_legacy_boot(&render::strip_injections(&fs::read_to_string(path)?));
            let parsed = parse_digest_html_with_lang(&html, Some(lang))
                .with_context(|| format!("parse source {}", issue.rel()))?;
            Ok(SourcePage {
                issue,
                html,
                parsed,
            })
        })
        .collect()
}

const LEGACY_BOOT_HASH: &str = "'sha256-84CpxjSoM06t9mCBHfj7MlnH/uxdaToEDTPGW4QpEi8='";
const LEGACY_READER_HASH: &str = "'sha256-hBeISf4iRY21t9nbA4l4YULbvvnn36MD3nxoyYRoDr4='";
const LEGACY_LANG_HASH: &str = "'sha256-rJiHfTrNehhlqh8VOmATeR9fV0uSC01/Qy1M4jESr+s='";

fn clean_legacy_boot(html: &str) -> String {
    static SCRIPT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<script>(.*?)</script\s*>").unwrap());
    SCRIPT
        .replace_all(html, |m: &Captures<'_>| {
            if crypto::script_hash(&m[1]) == LEGACY_BOOT_HASH {
                format!("<script>{}</script>", assets::BOOT_JS)
            } else {
                m[0].to_owned()
            }
        })
        .into_owned()
}

fn trusted_scripts(
    legacy: Option<&str>,
    demo_password: Option<&str>,
    plans: &Plans,
) -> Result<ScriptAllowlist> {
    let mut allowed = ScriptAllowlist::default();
    // These exact hashes come from the v0.9.0 reader, never from an input page.
    for hash in [LEGACY_BOOT_HASH, LEGACY_READER_HASH, LEGACY_LANG_HASH] {
        allowed.allow_legacy_hash(hash);
    }
    if let Some(template) = legacy {
        for lang in plans
            .pages
            .values()
            .map(|p| p.lang)
            .collect::<BTreeSet<_>>()
        {
            allowed.allow_legacy_template(
                template,
                UnlockOptions {
                    lang,
                    demo_password,
                    ..UnlockOptions::default()
                },
            )?;
        }
    }
    Ok(allowed)
}

fn rerender_sources(ctx: &SiteContext, log: &mut Trace, changes: &mut FileChanges) -> Result<()> {
    for lang in [ctx.lang, Lang::En] {
        let english = lang == Lang::En && ctx.lang == Lang::ZhCn;
        let base = if english {
            ctx.paths.sources.join("en")
        } else {
            ctx.paths.sources.clone()
        };
        let opts = RenderOptions::default();
        for (issue, _) in source_refs(&base)? {
            let mut md = ctx.root.join(issue.md_name());
            if english {
                let stem = md.file_stem().unwrap().to_string_lossy();
                md.set_file_name(format!("{stem}.en.md"));
            }
            if !md.is_file() {
                log.line(format!("skipped {}: no markdown", issue.rel()))?;
                continue;
            }
            let digest = read_md(&md, lang, issue.start.year())?;
            let html = render::render_digest_html(&digest, lang, &opts);
            changes.remember(&source_path(&ctx.paths, &issue, english))?;
            changes.remember(&legacy_html_path(&ctx.paths, &issue, english))?;
            save_source(&ctx.paths, &issue, english, &html)?;
            atomic_write_text(&legacy_html_path(&ctx.paths, &issue, english), &html)?;
            log.line(format!("re-rendered {} ({lang})", issue.rel()))?;
        }
        if ctx.lang == Lang::En {
            break;
        }
    }
    Ok(())
}

struct Page {
    html: String,
    geo: GeoPage,
    lang: Lang,
    shell_alt: Option<LangAlt>,
    alt_rel: String,
}

#[derive(Default)]
struct Plans {
    pages: BTreeMap<PathBuf, Page>,
    prefixes: Vec<String>,
    public_root_aliases: Vec<String>,
    redirect_root: Option<String>,
    symmetric: bool,
}

fn url_prefix(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        String::new()
    } else {
        format!("/{joined}")
    }
}

fn page_path(prefix: &str, rel: &str) -> String {
    format!("{prefix}/{rel}")
}

fn with_alt(alt: Option<&LangAlt>, rel: &str) -> Option<LangAlt> {
    alt.map(|alt| LangAlt {
        lang: alt.lang,
        base: format!("{}{rel}", alt.base),
    })
}

fn issue_link(prefix: &str, issue: &IssueRef) -> LinkItem {
    LinkItem {
        label: issue.end.format("%m.%d").to_string(),
        url: page_path(prefix, &issue.rel()),
    }
}

struct Edition<'a> {
    lang: Lang,
    default_lang: Lang,
    prefix: &'a str,
    canonical: &'a str,
    pages: &'a [SourcePage],
    other_prefix: Option<&'a str>,
    other_pages: &'a [SourcePage],
    external: Option<&'a LangAlt>,
    feed: &'a str,
    encrypt: bool,
}

impl Edition<'_> {
    fn alternate(&self, issue: Option<&IssueRef>, rel: &str) -> (Option<LangAlt>, String) {
        if let Some(prefix) = self.other_prefix {
            let rel = if issue.is_some_and(|i| !self.other_pages.iter().any(|p| p.issue == *i)) {
                ""
            } else {
                rel
            };
            (
                Some(LangAlt {
                    lang: self.lang.other(),
                    base: page_path(prefix, ""),
                }),
                rel.to_owned(),
            )
        } else {
            (self.external.cloned(), rel.to_owned())
        }
    }

    fn alternates(
        &self,
        issue: Option<&IssueRef>,
        canonical_path: &str,
    ) -> BTreeMap<String, String> {
        let mut alts =
            BTreeMap::from([(self.lang.hreflang().to_owned(), canonical_path.to_owned())]);
        let rel = issue.map_or_else(String::new, IssueRef::rel);
        let mut other_url: Option<String> = None;
        if let Some(prefix) = self.other_prefix {
            if issue.is_none_or(|i| self.other_pages.iter().any(|p| p.issue == *i)) {
                let target = page_path(prefix, &rel);
                alts.insert(self.lang.other().hreflang().into(), target.clone());
                other_url = Some(target);
            }
        } else if let Some(alt) = with_alt(self.external, &rel) {
            alts.insert(alt.lang.hreflang().into(), alt.base.clone());
            other_url = Some(alt.base);
        }
        if alts.len() > 1 {
            let x_default = if self.lang == self.default_lang {
                canonical_path.to_owned()
            } else {
                other_url.unwrap_or_else(|| canonical_path.to_owned())
            };
            alts.insert("x-default".into(), x_default);
        }
        alts
    }

    fn make_issue(
        &self,
        source: &SourcePage,
        route: &str,
        alias_only: bool,
        root_alias: bool,
    ) -> Page {
        let issue = &source.issue;
        let (html_url, canonical_path) = (
            page_path(self.prefix, route),
            page_path(self.canonical, &issue.rel()),
        );
        let canonical = !alias_only && self.prefix == self.canonical;
        let (shell_alt, alt_rel) = if root_alias {
            (None, String::new())
        } else {
            self.alternate(Some(issue), route)
        };
        let relevant: Vec<_> = self
            .pages
            .iter()
            .filter(|p| p.issue.kind == issue.kind)
            .collect();
        let current = relevant.iter().position(|p| p.issue == *issue);
        let prev = current
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| relevant.get(i));
        let next = current.and_then(|i| relevant.get(i + 1));
        let weekly = if issue.kind == "daily" {
            self.pages.iter().rev().find(|p| {
                p.issue.kind == "weekly"
                    && p.issue.start <= issue.start
                    && issue.start <= p.issue.end
            })
        } else {
            None
        };
        let nav_prefix = if root_alias {
            self.canonical
        } else {
            self.prefix
        };
        let link = |p: &&SourcePage| issue_link(nav_prefix, &p.issue);
        let opts = NavOptions {
            lang: self.lang,
            home: Some(LinkItem {
                label: String::new(),
                url: page_path(nav_prefix, ""),
            }),
            previous: prev.map(link),
            next: next.map(link),
            weekly: weekly.map(|p| issue_link(nav_prefix, &p.issue)),
            lang_alt: with_alt(shell_alt.as_ref(), &alt_rel),
            noindex: self.encrypt,
        };
        let html = render::inject_nav(&source.html, &opts);
        let geo = GeoPage {
            rel_html: format!("{}.html", html_url.trim_start_matches('/')),
            url_path: html_url.clone(),
            canonical_path: canonical_path.clone(),
            rel_md: format!("{}.md", html_url.trim_start_matches('/')),
            lang: self.lang,
            kind: issue.kind.clone(),
            label: issue.label.clone(),
            date_iso: issue.end.to_string(),
            is_canonical: canonical,
            alternates: self.alternates(Some(issue), &canonical_path),
            feed_path: self.feed.to_owned(),
            parsed: Some(source.parsed.clone()),
            ..GeoPage::default()
        };
        Page {
            html,
            geo,
            lang: self.lang,
            shell_alt,
            alt_rel: if alt_rel.is_empty() {
                "index.html".into()
            } else {
                alt_rel
            },
        }
    }

    fn make_index(&self) -> Page {
        let (shell_alt, _) = self.alternate(None, "");
        let archive: Vec<_> = self
            .pages
            .iter()
            .map(|p| {
                let mut seen = BTreeSet::new();
                let tags = p
                    .parsed
                    .top_tags
                    .iter()
                    .chain(p.parsed.items.iter().flat_map(|item| &item.tags))
                    .filter(|s| !s.is_empty() && seen.insert(s.as_str()))
                    .cloned()
                    .collect();
                ArchiveIssue {
                    date: p.issue.end,
                    weekly: p.issue.kind == "weekly",
                    href: page_path(self.prefix, &p.issue.rel()),
                    title: p.parsed.title.clone(),
                    headline: p
                        .parsed
                        .must
                        .first()
                        .map(|m| m.title.clone())
                        .or_else(|| p.parsed.items.first().map(|i| i.title.clone()))
                        .unwrap_or_default(),
                    tags,
                    item_count: p.parsed.items_count,
                }
            })
            .collect();
        let mut html = render::render_archive_html(
            &archive,
            self.lang,
            &RenderOptions {
                lang_alt: shell_alt.clone(),
                ..RenderOptions::default()
            },
        );
        if self.encrypt {
            html = html.replacen("</head>", "<!-- digest-meta:start --><meta name=\"robots\" content=\"noindex,nofollow\"><!-- digest-meta:end --></head>", 1);
        }
        let index_dir = page_path(self.prefix, "");
        let canonical = page_path(self.canonical, "");
        let latest_date = self
            .pages
            .iter()
            .map(|p| p.issue.end)
            .max()
            .map(|d| d.to_string())
            .unwrap_or_default();
        let geo = GeoPage {
            rel_html: page_path(self.prefix, "index.html")
                .trim_start_matches('/')
                .into(),
            rel_md: page_path(self.prefix, "index.md")
                .trim_start_matches('/')
                .into(),
            url_path: index_dir,
            canonical_path: canonical.clone(),
            lang: self.lang,
            page_type: PageType::Index,
            date_iso: latest_date,
            alternates: self.alternates(None, &canonical),
            feed_path: self.feed.to_owned(),
            is_canonical: self.prefix == self.canonical,
            ..GeoPage::default()
        };
        Page {
            html,
            geo,
            lang: self.lang,
            shell_alt,
            alt_rel: "index.html".into(),
        }
    }

    fn aliases(&self, plans: &mut Plans, root: bool) {
        let daily = self.pages.iter().rev().find(|p| p.issue.kind == "daily");
        let weekly = self.pages.iter().rev().find(|p| p.issue.kind == "weekly");
        for (route, page) in [
            ("daily/latest", daily),
            ("weekly/latest", weekly),
            ("latest", daily.or(weekly)),
        ] {
            if let Some(page) = page {
                let page = self.make_issue(page, route, true, root);
                plans.pages.insert(PathBuf::from(&page.geo.rel_html), page);
            }
        }
    }

    fn add(&self, plans: &mut Plans) {
        plans.prefixes.push(self.prefix.into());
        let index = self.make_index();
        plans
            .pages
            .insert(PathBuf::from(&index.geo.rel_html), index);
        for source in self.pages {
            let page = self.make_issue(source, &source.issue.rel(), false, false);
            plans.pages.insert(PathBuf::from(&page.geo.rel_html), page);
        }
        self.aliases(plans, false);
    }
}

fn site_plans(ctx: &SiteContext) -> Result<Plans> {
    let cfg = &ctx.config;
    let primary = source_pages(&ctx.paths.sources, ctx.lang)?;
    let en = if ctx.lang == Lang::ZhCn {
        source_pages(&ctx.paths.sources.join("en"), Lang::En)?
    } else {
        Vec::new()
    };
    let symmetric = !en.is_empty();
    let prefix = url_prefix(&[&cfg.slug]);
    let zh_prefix = url_prefix(&[&cfg.slug, "zh"]);
    let en_prefix = url_prefix(&[&cfg.slug, "en"]);
    let common = Edition {
        lang: ctx.lang,
        default_lang: ctx.lang,
        prefix: &prefix,
        canonical: if symmetric { &zh_prefix } else { &prefix },
        pages: &primary,
        other_prefix: symmetric.then_some(en_prefix.as_str()),
        other_pages: &en,
        external: cfg.lang_alt.as_ref(),
        feed: if symmetric {
            "/zh/feed.xml"
        } else {
            "/feed.xml"
        },
        encrypt: cfg.encrypt,
    };
    let mut plans = Plans {
        symmetric,
        ..Plans::default()
    };
    common.add(&mut plans);
    if symmetric {
        Edition {
            prefix: &zh_prefix,
            ..common
        }
        .add(&mut plans);
        Edition {
            lang: Lang::En,
            default_lang: ctx.lang,
            prefix: &en_prefix,
            canonical: &en_prefix,
            pages: &en,
            other_prefix: Some(&zh_prefix),
            other_pages: &primary,
            external: None,
            feed: "/en/feed.xml",
            encrypt: cfg.encrypt,
        }
        .add(&mut plans);
    }
    if cfg.root_latest && !prefix.is_empty() {
        plans.public_root_aliases.push(String::new());
        Edition {
            prefix: "",
            ..common
        }
        .aliases(&mut plans, true);
        if symmetric {
            plans
                .public_root_aliases
                .extend(["/zh".into(), "/en".into()]);
            Edition {
                prefix: "/zh",
                ..common
            }
            .aliases(&mut plans, true);
            Edition {
                lang: Lang::En,
                default_lang: ctx.lang,
                prefix: "/en",
                canonical: &en_prefix,
                pages: &en,
                other_prefix: Some("/zh"),
                other_pages: &primary,
                external: None,
                feed: "/en/feed.xml",
                encrypt: cfg.encrypt,
            }
            .aliases(&mut plans, true);
        }
    }
    Ok(plans)
}

fn set_redirects(plans: &Plans, file: &Path, removed: &[String]) -> Result<()> {
    let mut rules = String::new();
    if let Some(root) = &plans.redirect_root {
        writeln!(rules, "/ {root} 302")?;
    }
    let mut seen = BTreeSet::new();
    for line in removed {
        if let Ok(issue) = IssueRef::parse(line) {
            for prefix in &plans.prefixes {
                let from = page_path(prefix, &issue.rel());
                let relative = PathBuf::from(format!("{}.html", from.trim_start_matches('/')));
                if plans.pages.contains_key(&relative) || !seen.insert(from.clone()) {
                    continue;
                }
                writeln!(rules, "{from} {prefix}/ 302\n{from}.html {prefix}/ 302")?;
            }
        }
    }
    if !rules.is_empty() {
        atomic_write_text(file, &rules)?;
    }
    Ok(())
}

fn cache_headers(plans: &Plans) -> Result<String> {
    let mut out = String::from("/favicon.svg\n  Cache-Control: public, max-age=86400\n");
    let full: BTreeSet<_> = plans.prefixes.iter().collect();
    for prefix in plans.prefixes.iter().chain(&plans.public_root_aliases) {
        if full.contains(prefix) {
            writeln!(
                out,
                "{prefix}/\n  Cache-Control: public, max-age=60, stale-while-revalidate=604800"
            )?;
            for kind in ["daily", "weekly"] {
                writeln!(out, "{prefix}/{kind}/*\n  Cache-Control: public, max-age=300, stale-while-revalidate=604800")?;
            }
        }
        for alias in ["latest", "daily/latest", "weekly/latest"] {
            for tail in ["", "/"] {
                writeln!(
                    out,
                    "{prefix}/{alias}{tail}\n  ! Cache-Control\n  Cache-Control: no-store"
                )?;
            }
        }
    }
    Ok(out)
}

fn read_removed(path: &Path) -> Result<Vec<String>> {
    Ok(read_if_present(path)?
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

fn read_if_present(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn update_removed(path: &Path, issue: &IssueRef, remove: bool) -> Result<()> {
    let mut entries = read_removed(path)?;
    entries.retain(|s| IssueRef::parse(s).is_ok_and(|x| x != *issue));
    if remove {
        entries.push(issue.rel());
    }
    atomic_write_text(
        path,
        &entries.iter().map(|s| format!("{s}\n")).collect::<String>(),
    )?;
    Ok(())
}

fn clean_template(template: &str) -> String {
    // The legacy shell placed its theme initializer in the first bare script.
    static LEGACY_THEME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<script>(.*?)</script\s*>").unwrap());
    LEGACY_THEME
        .replacen(template, 1, |m: &Captures<'_>| {
            if m[1].contains("localStorage.getItem(\"digest-theme\")")
                && m[1].contains("theme-color")
            {
                format!("<script>{}</script>", assets::BOOT_JS)
            } else {
                m[0].to_owned()
            }
        })
        .into_owned()
}

struct Credentials {
    password: String,
    salt: Vec<u8>,
}

impl Credentials {
    fn generate() -> Self {
        let mut random = [0; 24];
        let mut salt = vec![0; crypto::SALT_LEN];
        OsRng.fill_bytes(&mut random);
        OsRng.fill_bytes(&mut salt);
        Self {
            password: URL_SAFE_NO_PAD.encode(random),
            salt,
        }
    }

    fn read() -> Result<Self> {
        Ok(Self {
            password: verify::read_password(None)?,
            salt: crypto::load_or_create_salt(&crypto::salt_file_path()?)?,
        })
    }
}

struct ArtifactStage<'a> {
    tmp: tempfile::TempDir,
    paths: DigestPaths,
    destination: &'a DigestPaths,
}

#[derive(Default)]
struct FileChanges(BTreeMap<PathBuf, Option<Vec<u8>>>);

impl FileChanges {
    fn remember(&mut self, path: &Path) -> Result<()> {
        if !self.0.contains_key(path) {
            let bytes = match fs::read(path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            self.0.insert(path.into(), bytes);
        }
        Ok(())
    }

    fn commit(&mut self) {
        self.0.clear();
    }
}

impl Drop for FileChanges {
    fn drop(&mut self) {
        for (path, old) in &self.0 {
            let restore = if let Some(bytes) = old {
                atomic_write_bytes(path, bytes)
            } else {
                fs::remove_file(path).or_else(|err| {
                    if err.kind() == io::ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(err)
                    }
                })
            };
            if let Err(error) = restore {
                eprintln!("error: restore {}: {error}", path.display());
            }
        }
    }
}

struct InstalledOutputs {
    _backup_dir: tempfile::TempDir,
    applied: Vec<(PathBuf, PathBuf)>,
}

impl InstalledOutputs {
    fn rollback(self) -> Result<()> {
        for (path, backup) in self.applied.into_iter().rev() {
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else if path.is_file() {
                fs::remove_file(&path)?;
            }
            if backup.try_exists()? {
                fs::rename(&backup, &path)?;
            }
        }
        Ok(())
    }
}

impl<'a> ArtifactStage<'a> {
    fn new(destination: &'a DigestPaths) -> Result<Self> {
        fs::create_dir_all(&destination.site)?;
        let tmp = tempfile::Builder::new()
            .prefix(".prepare-")
            .tempdir_in(&destination.site)?;
        let paths = DigestPaths::at(tmp.path(), destination.timezone);
        fs::create_dir_all(&paths.site)?;
        Ok(Self {
            tmp,
            paths,
            destination,
        })
    }

    fn install_reversible(self, encrypted: bool) -> Result<InstalledOutputs> {
        let mut batch = vec![
            (&self.paths.plain, &self.destination.plain),
            (&self.paths.public, &self.destination.public),
        ];
        if encrypted {
            batch.push((&self.paths.enc_cache, &self.destination.enc_cache));
        }
        let applied = install_outputs_tracked(&batch, self.tmp.path())?;
        Ok(InstalledOutputs {
            _backup_dir: self.tmp,
            applied,
        })
    }

    fn install(self, encrypted: bool) -> Result<()> {
        let _ = self.install_reversible(encrypted)?;
        Ok(())
    }
}

fn install_outputs(batch: &[(&PathBuf, &PathBuf)], backup_dir: &Path) -> Result<()> {
    let _ = install_outputs_tracked(batch, backup_dir)?;
    Ok(())
}

fn install_outputs_tracked(
    batch: &[(&PathBuf, &PathBuf)],
    backup_dir: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut applied = Vec::new();
    for (i, (source, dest)) in batch.iter().enumerate() {
        let backup = backup_dir.join(format!("previous-{i}"));
        let step = (|| -> Result<()> {
            if dest.try_exists()? {
                fs::rename(dest, &backup)?;
            }
            applied.push(((*dest).clone(), backup));
            fs::rename(source, dest)?;
            Ok(())
        })();
        if let Err(err) = step {
            for (path, backup) in applied.into_iter().rev() {
                if path.is_dir() {
                    fs::remove_dir_all(&path)?;
                } else if path.is_file() {
                    fs::remove_file(&path)?;
                }
                if backup.try_exists()? {
                    fs::rename(&backup, &path)?;
                }
            }
            return Err(err);
        }
    }
    Ok(applied)
}

fn mirror_html(from: &Path, into: &Path) -> Result<()> {
    for source in files_under(from)? {
        let dest = into.join(source.strip_prefix(from)?);
        atomic_write_bytes(&dest, &fs::read(source)?)?;
    }
    Ok(())
}

fn copy_cached_shells(stage: &ArtifactStage<'_>, pages: &BTreeMap<PathBuf, Page>) -> Result<()> {
    let dest = stage.destination;
    if !dest.enc_cache.is_file() {
        return Ok(());
    }
    let cache = crypto::load_enc_cache(&dest.enc_cache)?;
    crypto::save_enc_cache(&stage.paths.enc_cache, &cache)?;
    for relative in pages.keys() {
        if !relative.to_str().is_some_and(|s| cache.contains_key(s)) {
            continue;
        }
        let old = dest.public.join(relative);
        if let Some(shell) = read_if_present(&old)? {
            if crypto::extract_envelope(&shell).is_ok() {
                atomic_write_text(&stage.paths.public.join(relative), &shell)?;
            }
        }
    }
    Ok(())
}

struct BuildOptions<'a> {
    config: &'a SiteConfig,
    credentials: Option<&'a Credentials>,
    demo_password: Option<&'a str>,
    verify: bool,
    now: DateTime<Utc>,
    removed: &'a [String],
    legacy_template: Option<&'a str>,
}

fn build_artifacts(
    stage: &ArtifactStage<'_>,
    plans: &mut Plans,
    build: &BuildOptions<'_>,
    log: &mut Trace,
) -> Result<Vec<PathBuf>> {
    let paths = &stage.paths;
    let cfg = build.config;
    let is_private = cfg.encrypt;
    let template = build.legacy_template.map(clean_template);
    let search = cfg.geo_search && geo::worker_enabled(cfg);
    let origin = geo::site_origin(&cfg.domain);
    fs::create_dir_all(&paths.public)?;
    let mut canonicals = BTreeMap::<Lang, Vec<GeoPage>>::new();
    for p in plans
        .pages
        .values()
        .filter(|p| p.geo.page_type == PageType::Issue && p.geo.is_canonical)
    {
        canonicals.entry(p.lang).or_default().push(p.geo.clone());
    }
    let blank = render::render_404_html();
    // The root 404 is a static Cloudflare control page, including on private sites.
    atomic_write_text(&paths.public.join("404.html"), &blank)?;
    atomic_write_text(&paths.plain.join("404.html"), &blank)?;
    atomic_write_text(&paths.public.join("favicon.svg"), assets::FAVICON_SVG)?;
    for (relative, p) in &mut plans.pages {
        let html = if is_private {
            crypto::inject_lock_ui_html(&p.html, p.lang)
        } else {
            let pages = canonicals
                .get(&p.lang)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            geo::decorate_public_html(&p.html, &mut p.geo, &pages, &origin, search)?
        };
        atomic_write_text(&paths.plain.join(relative), &html)?;
        log.detail(format!("rendered {}", relative.display()))?;
    }
    atomic_write_text(&paths.public.join("_headers"), &cache_headers(plans)?)?;
    if is_private {
        copy_cached_shells(stage, &plans.pages)?;
        geo::clean_stale_geo_files(&paths.public)?;
        geo::write_private_robots(&paths.public)?;
    }
    crypto::privacy_scan(&stage.destination.site, &paths.plain, &paths.public)?;

    let changed = if let Some(credentials) = build.credentials {
        let key = crypto::derive_key(&credentials.password, &credentials.salt);
        let changed = crypto::encrypt_all_pages_with(paths, &key, &credentials.salt, |rel| {
            let p = &plans.pages[rel];
            UnlockOptions {
                lang: p.lang,
                lang_alt: p.shell_alt.as_ref(),
                relative_page: if p.alt_rel == "index.html" {
                    "index.html"
                } else {
                    &p.alt_rel
                },
                demo_password: build.demo_password,
                legacy_template: template.as_deref(),
            }
        })?;
        let slug = if build.demo_password.is_some() {
            ""
        } else {
            &cfg.slug
        };
        crypto::assert_public_is_encrypted_with(&paths.plain, &paths.public, slug, |rel| {
            let p = plans
                .pages
                .get(rel)
                .with_context(|| format!("unexpected encrypted page: {}", rel.display()))?;
            Ok(UnlockOptions {
                lang: p.lang,
                lang_alt: p.shell_alt.as_ref(),
                relative_page: if p.alt_rel == "index.html" {
                    "index.html"
                } else {
                    &p.alt_rel
                },
                demo_password: build.demo_password,
                legacy_template: template.as_deref(),
            })
        })?;
        changed
    } else {
        ensure!(!is_private, "missing encryption credentials");
        mirror_html(&paths.plain, &paths.public)?;
        let pages: Vec<_> = plans.pages.values().map(|p| p.geo.clone()).collect();
        geo::write_public_geo_bundle(
            &paths.public,
            &pages,
            cfg,
            plans.symmetric,
            build.now,
            &BTreeMap::new(),
        )?;
        crypto::assert_public_matches_plain(&paths.plain, &paths.public)?;
        plans
            .pages
            .keys()
            .filter(|rel| {
                fs::read(paths.public.join(rel)).ok()
                    != fs::read(stage.destination.public.join(rel)).ok()
            })
            .cloned()
            .collect()
    };
    set_redirects(plans, &paths.public.join("_redirects"), build.removed)?;
    let allowlist = trusted_scripts(template.as_deref(), build.demo_password, plans)?;
    crypto::finalize_csp_headers(&paths.public, &paths.plain, !is_private, &allowlist)?;
    geo::finalize_public_worker(&paths.public, cfg)?;
    // The finalized Worker carries the response headers and redirect table too.
    crypto::privacy_scan(&stage.destination.site, &paths.plain, &paths.public)?;
    if build.verify {
        let checked = verify::verify_local(
            paths,
            is_private,
            build.credentials.map(|c| c.password.as_str()),
        )?;
        log.line(format!("local verification: {checked} pages, exact match"))?;
    }
    Ok(changed)
}

fn strict_existing_artifacts(ctx: &SiteContext) -> Result<()> {
    let paths = &ctx.paths;
    let cfg = &ctx.config;
    crypto::privacy_scan(&paths.site, &paths.plain, &paths.public)?;
    let control = paths.public.join("404.html");
    if control.is_file() {
        let expected = paths.plain.join("404.html");
        ensure!(
            expected.is_file() && fs::read(expected)? == fs::read(control)?,
            "published 404 differs from plaintext"
        );
    }
    let template =
        read_if_present(&paths.site.join("password_template.html"))?.map(|s| clean_template(&s));
    let plans = site_plans(ctx)?;
    if cfg.encrypt {
        for p in files_under(&paths.public)? {
            let rel = p.strip_prefix(&paths.public)?;
            let static_file = matches!(
                rel.to_str(),
                Some("favicon.svg" | "404.html" | "robots.txt" | "_headers" | "_redirects")
            );
            ensure!(
                static_file
                    || p.extension().is_some_and(|e| e == "html")
                        && !rel.starts_with(".well-known"),
                "unexpected public file in encrypted site"
            );
        }
        ensure!(
            fs::read_to_string(paths.public.join("robots.txt"))? == "User-agent: *\nDisallow: /\n",
            "encrypted site robots policy is missing"
        );
        crypto::assert_public_is_encrypted_with(&paths.plain, &paths.public, &cfg.slug, |rel| {
            let p = plans.pages.get(rel).with_context(|| {
                format!("unexpected or stale encrypted page: {}", rel.display())
            })?;
            Ok(UnlockOptions {
                lang: p.lang,
                lang_alt: p.shell_alt.as_ref(),
                relative_page: if p.alt_rel == "index.html" {
                    "index.html"
                } else {
                    &p.alt_rel
                },
                demo_password: None,
                legacy_template: template.as_deref(),
            })
        })?;
    } else {
        crypto::assert_public_matches_plain(&paths.plain, &paths.public)?;
    }
    let allowed = trusted_scripts(template.as_deref(), None, &plans)?;
    let expected =
        crypto::expected_security_headers(&paths.public, &paths.plain, !cfg.encrypt, &allowed)?;
    let expected_csp = &expected["Content-Security-Policy"];
    let raw_headers = fs::read_to_string(paths.public.join("_headers"))?;
    let mut route = "";
    let mut seen_global_security = BTreeSet::new();
    for line in raw_headers.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line == line.trim_start()
            && (line.starts_with('/')
                || line.starts_with("http://")
                || line.starts_with("https://"))
        {
            route = trimmed;
            continue;
        }
        if trimmed.starts_with('!') {
            if route == "/*" {
                bail!("global header removals cannot override finalized security headers");
            }
            let target = crate::model::parse_header_removal_target(trimmed)
                .context("malformed header removal directive in _headers")?;
            ensure!(
                !crate::model::is_protected_security_header(target, cfg.encrypt),
                "route-level removal of security header is forbidden: {target}"
            );
            continue;
        }
        if let Some((key, _)) = trimmed.split_once(':') {
            let key = key.trim();
            if route != "/*" && crate::model::is_protected_security_header(key, cfg.encrypt) {
                bail!("route-level security header override is forbidden: {key}");
            }
            if route == "/*" {
                let low = key.to_ascii_lowercase();
                if expected.keys().any(|k| k.eq_ignore_ascii_case(key)) || low == "x-robots-tag" {
                    ensure!(
                        seen_global_security.insert(low),
                        "duplicate global security header: {key}"
                    );
                }
            }
        }
    }
    let (headers, _) = geo::parse_headers(&raw_headers);
    let header_value = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let policy = header_value("Content-Security-Policy")
        .context("missing finalized Content-Security-Policy")?;
    crypto::assert_csp_policy_matches(policy, expected_csp)?;
    for (key, want) in &expected {
        if key == "Content-Security-Policy" {
            continue;
        }
        ensure!(
            header_value(key) == Some(want.as_str()),
            "missing or modified finalized header: {key}"
        );
    }
    if !cfg.encrypt {
        ensure!(
            header_value("X-Robots-Tag").is_none(),
            "public site unexpectedly sets global X-Robots-Tag"
        );
    }
    let worker = paths.public.join("_worker.js");
    let routes = paths.public.join("_routes.json");
    if geo::worker_enabled(cfg) {
        let worker_cfg = geo::public_worker_config(&paths.public, cfg)?;
        let worker_csp = worker_cfg
            .global_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("Content-Security-Policy"))
            .map(|(_, v)| v.as_str())
            .context("Worker globalHeaders is missing Content-Security-Policy")?;
        crypto::assert_csp_policy_matches(worker_csp, expected_csp)?;
        ensure!(
            fs::read_to_string(&worker)? == geo::render_pages_worker(&worker_cfg)?
                && fs::read_to_string(&routes)? == geo::render_pages_routes(),
            "prepared Worker does not match the finalized headers and redirects"
        );
    } else {
        ensure!(
            !worker.try_exists()? && !routes.try_exists()?,
            "unexpected Worker artifacts"
        );
    }
    Ok(())
}

fn remove_issue(
    ctx: &SiteContext,
    issue: &IssueRef,
    stamp: &str,
    changes: &mut FileChanges,
) -> Result<PathBuf> {
    let paths = &ctx.paths;
    let primary = source_path(paths, issue, false);
    let english = source_path(paths, issue, true);
    let mut files = vec![
        (primary, format!("source-{}.html", issue.label), true),
        (english, format!("source-en-{}.html", issue.label), true),
        (
            legacy_html_path(paths, issue, false),
            format!("{}.html", issue.name()),
            true,
        ),
        (
            legacy_html_path(paths, issue, true),
            format!("{}.en.html", issue.name()),
            true,
        ),
    ];
    ensure!(
        files.iter().any(|(p, _, _)| p.is_file()),
        "{} not found in site/sources or html",
        issue.rel()
    );
    let base = paths.public.join(&ctx.config.slug);
    for lang in ["", "zh", "en"] {
        let rel = Path::new(lang).join(issue.file());
        files.push((
            base.join(&rel),
            format!("published-{}", rel.to_string_lossy().replace('/', "-")),
            false,
        ));
    }
    fs::create_dir_all(&paths.removed_backup)?;
    let mut dir = paths
        .removed_backup
        .join(format!("{}-{stamp}", issue.name()));
    for i in 2.. {
        match fs::create_dir(&dir) {
            Ok(()) => break,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                dir = paths
                    .removed_backup
                    .join(format!("{}-{stamp}-{i}", issue.name()))
            }
            Err(e) => return Err(e.into()),
        }
    }
    let mut moved = Vec::new();
    for (path, name, remove) in files {
        if path.is_file() {
            atomic_write_bytes(&dir.join(name), &fs::read(&path)?)?;
            if remove {
                moved.push(path);
            }
        }
    }
    for path in moved {
        changes.remember(&path)?;
        fs::remove_file(path)?;
    }
    changes.remember(&paths.removed_paths)?;
    update_removed(&paths.removed_paths, issue, true)?;
    Ok(dir)
}

fn secret_backup_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".prev");
    PathBuf::from(os)
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

struct RotationRollback {
    password_path: PathBuf,
    salt_path: PathBuf,
    password_backup: PathBuf,
    salt_backup: PathBuf,
    old_password: Option<String>,
    old_salt: Option<String>,
    installed: InstalledOutputs,
}

impl RotationRollback {
    fn begin(stage: ArtifactStage<'_>, credentials: &Credentials) -> Result<Self> {
        let password_path = crypto::password_file_path()?;
        let salt_path = crypto::salt_file_path()?;
        ensure!(
            password_path != salt_path,
            "password and salt files must be different"
        );
        let password_backup = secret_backup_path(&password_path);
        let salt_backup = secret_backup_path(&salt_path);
        let old_password = read_if_present(&password_path)?;
        let old_salt = read_if_present(&salt_path)?;
        if let Some(prev) = &old_password {
            crypto::write_secret_file(&password_backup, prev)?;
        }
        if let Some(prev) = &old_salt {
            crypto::write_secret_file(&salt_backup, prev)?;
        }
        let apply = (|| -> Result<InstalledOutputs> {
            crypto::write_secret_file(&password_path, &format!("{}\n", credentials.password))?;
            crypto::write_secret_file(
                &salt_path,
                &format!("{}\n", hex::encode(&credentials.salt)),
            )?;
            stage.install_reversible(true)
        })();
        match apply {
            Ok(installed) => Ok(Self {
                password_path,
                salt_path,
                password_backup,
                salt_backup,
                old_password,
                old_salt,
                installed,
            }),
            Err(error) => {
                let _ = restore_secret(&password_path, old_password.as_deref());
                let _ = restore_secret(&salt_path, old_salt.as_deref());
                let _ = remove_if_exists(&password_backup);
                let _ = remove_if_exists(&salt_backup);
                Err(error)
            }
        }
    }

    fn commit(self) -> Result<()> {
        remove_if_exists(&self.password_backup)?;
        remove_if_exists(&self.salt_backup)?;
        Ok(())
    }

    fn preserve_after_deploy(self) -> (PathBuf, PathBuf) {
        (self.password_backup, self.salt_backup)
    }

    fn rollback(self) -> Result<()> {
        restore_secret(&self.password_path, self.old_password.as_deref())?;
        restore_secret(&self.salt_path, self.old_salt.as_deref())?;
        self.installed.rollback()?;
        remove_if_exists(&self.password_backup)?;
        remove_if_exists(&self.salt_backup)?;
        Ok(())
    }
}

fn restore_secret(path: &Path, previous: Option<&str>) -> Result<()> {
    if let Some(value) = previous {
        crypto::write_secret_file(path, value)?;
    } else {
        remove_if_exists(path)?;
    }
    Ok(())
}

fn token() -> Result<Option<String>> {
    if let Some(token) = env::var("CLOUDFLARE_API_TOKEN")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
    {
        return Ok(Some(token));
    }
    let path = env::var_os("CLOUDFLARE_API_TOKEN_FILE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let default = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".config/ai-digest/cloudflare_token"));
    for path in path.into_iter().chain(default) {
        if let Some(contents) = read_if_present(&path)? {
            let token = contents.trim();
            if !token.is_empty() {
                return Ok(Some(token.into()));
            }
        }
    }
    Ok(None)
}

struct PagesApi<'a> {
    client: Client,
    base: Url,
    token: &'a str,
}

#[derive(Debug, thiserror::Error)]
#[error("Cloudflare Pages {method} {route} returned HTTP {status}")]
struct PagesStatus {
    method: Method,
    route: String,
    status: StatusCode,
}

impl<'a> PagesApi<'a> {
    fn new(mut base: Url, token: &'a str) -> Result<Self> {
        let loopback = loopback_base(&base);
        ensure!(
            (base.scheme() == "https" || (base.scheme() == "http" && loopback))
                && base.has_host()
                && base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "invalid Cloudflare Pages API base"
        );
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .user_agent(format!("ai-digest/{}", crate::version::VERSION))
            .build()?;
        Ok(Self {
            client,
            base,
            token,
        })
    }

    fn request(&self, method: Method, route: &str, token: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self
            .client
            .request(method, self.base.join(route)?)
            .bearer_auth(token))
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
        method: Method,
        route: &str,
    ) -> Result<Value> {
        let res = request.send().await?;
        if !res.status().is_success() {
            return Err(PagesStatus {
                method,
                route: route.into(),
                status: res.status(),
            }
            .into());
        }
        let text = res.text().await?;
        let body = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&text).context("invalid Cloudflare Pages API response")?
        };
        ensure!(
            body.get("success").and_then(Value::as_bool) != Some(false),
            "Cloudflare Pages API rejected {route}"
        );
        Ok(body)
    }

    async fn json(
        &self,
        method: Method,
        route: &str,
        token: &str,
        value: Option<Value>,
    ) -> Result<Value> {
        let mut request = self.request(method.clone(), route, token)?;
        if let Some(value) = value {
            request = request.json(&value);
        }
        self.send(request, method, route).await
    }
}

fn loopback_base(base: &Url) -> bool {
    matches!(base.host(), Some(url::Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(base.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
}

fn project_route(cfg: &SiteConfig) -> Result<String> {
    ensure!(
        !cfg.account_id.is_empty()
            && cfg
                .account_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')),
        "invalid or missing Cloudflare account ID"
    );
    ensure!(
        !cfg.project.is_empty()
            && cfg
                .project
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
        "invalid or missing Cloudflare Pages project"
    );
    Ok(format!(
        "accounts/{}/pages/projects/{}",
        cfg.account_id, cfg.project
    ))
}

fn asset_mime(path: &Path) -> &'static str {
    match path.file_name().and_then(|s| s.to_str()) {
        Some("feed.xml") => return "application/atom+xml; charset=utf-8",
        Some("rss.xml") => return "application/rss+xml; charset=utf-8",
        Some("feed.json") => return "application/feed+json; charset=utf-8",
        Some("api-catalog") => {
            return r#"application/linkset+json; profile="https://www.rfc-editor.org/info/rfc9727""#
        }
        _ => {}
    }
    match path.extension().and_then(|s| s.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("js") => "application/javascript",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

const MAX_UPLOAD_BATCH_FILES: usize = 20;
const MAX_UPLOAD_BATCH_BYTES: usize = 40 * 1024 * 1024;
const MAX_PAGES_ASSET_BYTES: usize = 25 * 1024 * 1024;

pub fn pages_asset_key(bytes: &[u8], content_type: &str) -> String {
    let digest = Sha256::digest(bytes);
    pages_asset_key_from_digest(&digest, content_type)
}

fn pages_asset_key_from_digest(content_digest: &[u8], content_type: &str) -> String {
    let mut keyed = Sha256::new();
    keyed.update(content_digest);
    keyed.update([0u8]);
    keyed.update(content_type.as_bytes());
    hex::encode(&keyed.finalize()[..16])
}

struct AssetEntry {
    key: String,
    path: PathBuf,
    content_type: &'static str,
}

fn encoded_upload_item_bytes(encoded_len: usize, key_len: usize, content_type_len: usize) -> usize {
    encoded_len
        .saturating_add(key_len)
        .saturating_add(content_type_len)
        .saturating_add(128)
}

fn validate_upload_item_fit(raw_len: usize, item_bytes: usize, batch_limit: usize) -> Result<()> {
    ensure!(
        raw_len <= MAX_PAGES_ASSET_BYTES,
        "Pages asset exceeds 25 MiB"
    );
    ensure!(
        item_bytes.saturating_add(2) <= batch_limit,
        "Pages upload item ({item_bytes} encoded bytes) exceeds batch byte limit ({batch_limit})"
    );
    Ok(())
}

fn hash_file_bounded(path: &Path, rel: &str) -> Result<([u8; 32], usize)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0usize;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n);
        ensure!(
            total <= MAX_PAGES_ASSET_BYTES,
            "Pages asset exceeds 25 MiB: {rel}"
        );
        hasher.update(&buf[..n]);
    }
    Ok((hasher.finalize().into(), total))
}

fn build_manifest(
    public: &Path,
) -> Result<(BTreeMap<String, String>, BTreeMap<String, AssetEntry>)> {
    let mut manifest = BTreeMap::new();
    let mut dedup = BTreeMap::<([u8; 32], &'static str), String>::new();
    let mut assets = BTreeMap::new();
    for path in files_under(public)? {
        let relative = path.strip_prefix(public)?;
        let rel = relative.to_str().context("invalid Pages asset path")?;
        if CONTROL_FILES.iter().any(|(name, _)| rel == *name) {
            continue;
        }
        let (digest, byte_len) = hash_file_bounded(&path, rel)?;
        let content_type = asset_mime(relative);
        let key = match dedup.get(&(digest, content_type)) {
            Some(existing) => existing.clone(),
            None => {
                let key = pages_asset_key_from_digest(&digest, content_type);
                let estimated_encoded = byte_len.div_ceil(3).saturating_mul(4);
                let estimated_item =
                    encoded_upload_item_bytes(estimated_encoded, key.len(), content_type.len());
                validate_upload_item_fit(byte_len, estimated_item, MAX_UPLOAD_BATCH_BYTES)
                    .with_context(|| format!("Pages asset {rel}"))?;
                assets.insert(
                    key.clone(),
                    AssetEntry {
                        key: key.clone(),
                        path: path.clone(),
                        content_type,
                    },
                );
                dedup.insert((digest, content_type), key.clone());
                key
            }
        };
        manifest.insert(format!("/{rel}"), key);
    }
    ensure!(!manifest.is_empty(), "no Pages assets to upload");
    ensure!(
        manifest.len() <= 20_000,
        "Pages asset manifest exceeds 20000 entries"
    );
    Ok((manifest, assets))
}

pub async fn deploy_via_pages_api(
    base: Url,
    cfg: &SiteConfig,
    public: &Path,
    token: &str,
    message: &str,
) -> Result<Value> {
    let api = PagesApi::new(base, token)?;
    direct_deploy(&api, cfg, public, message).await
}

async fn direct_deploy(
    api: &PagesApi<'_>,
    cfg: &SiteConfig,
    public: &Path,
    message: &str,
) -> Result<Value> {
    let project = project_route(cfg)?;
    let (manifest, assets) = build_manifest(public)?;
    let token_route = format!("{project}/upload-token");
    let result = match api.json(Method::GET, &token_route, api.token, None).await {
        Ok(result) => result,
        Err(error)
            if error
                .downcast_ref::<PagesStatus>()
                .is_some_and(|e| e.status == StatusCode::NOT_FOUND) =>
        {
            let create = format!("accounts/{}/pages/projects", cfg.account_id);
            api.json(
                Method::POST,
                &create,
                api.token,
                Some(json!({"name": cfg.project, "production_branch": "main"})),
            )
            .await?;
            api.json(Method::GET, &token_route, api.token, None).await?
        }
        Err(error) => return Err(error),
    };
    let result = &result["result"];
    let upload_token = result["jwt"]
        .as_str()
        .or_else(|| result["token"].as_str())
        .or_else(|| result.as_str())
        .filter(|s| !s.is_empty())
        .context("Pages API did not return an upload credential")?;
    let hashes: Vec<_> = assets.keys().cloned().collect();
    let result = api
        .json(
            Method::POST,
            "pages/assets/check-missing",
            upload_token,
            Some(json!({"hashes": hashes})),
        )
        .await;
    let missing: Vec<_> = result
        .ok()
        .and_then(|body| {
            let result = body.get("result").unwrap_or(&body);
            let list = result.as_array().or_else(|| result["hashes"].as_array())?;
            Some(
                list.iter()
                    .filter_map(|v| v.as_str().or_else(|| v["hash"].as_str()).map(str::to_owned))
                    .collect(),
            )
        })
        .unwrap_or_else(|| hashes.clone());
    let mut uploaded = BTreeSet::new();
    let mut batch = Vec::new();
    let mut batch_bytes = 2usize;
    for key in &missing {
        if !uploaded.insert(key.as_str()) {
            continue;
        }
        let Some(spec) = assets.get(key) else {
            continue;
        };
        let bytes = fs::read(&spec.path)?;
        let encoded = STANDARD.encode(&bytes);
        let item_bytes =
            encoded_upload_item_bytes(encoded.len(), spec.key.len(), spec.content_type.len());
        validate_upload_item_fit(bytes.len(), item_bytes, MAX_UPLOAD_BATCH_BYTES)?;
        if !batch.is_empty()
            && (batch.len() >= MAX_UPLOAD_BATCH_FILES
                || batch_bytes.saturating_add(item_bytes) > MAX_UPLOAD_BATCH_BYTES)
        {
            api.json(
                Method::POST,
                "pages/assets/upload",
                upload_token,
                Some(json!(batch)),
            )
            .await?;
            batch.clear();
            batch_bytes = 2;
        }
        batch_bytes = batch_bytes.saturating_add(item_bytes);
        batch.push(json!({
            "key": spec.key,
            "value": encoded,
            "base64": true,
            "metadata": {"contentType": spec.content_type}
        }));
    }
    if !batch.is_empty() {
        api.json(
            Method::POST,
            "pages/assets/upload",
            upload_token,
            Some(json!(batch)),
        )
        .await?;
    }
    let _ = api
        .json(
            Method::POST,
            "pages/assets/upsert-hashes",
            upload_token,
            Some(json!({"hashes": hashes})),
        )
        .await;
    let manifest = Part::text(serde_json::to_string(&manifest)?).mime_str("application/json")?;
    let mut form = Form::new()
        .part("manifest", manifest)
        .text("branch", "main")
        .text("commit_message", message.to_owned());
    for (name, mime) in CONTROL_FILES {
        let path = public.join(name);
        if path.is_file() {
            let file = Part::bytes(fs::read(path)?)
                .file_name(*name)
                .mime_str(mime)?;
            form = form.part(*name, file);
        } else {
            ensure!(*name != "_headers", "missing finalized _headers");
        }
    }
    let route = format!("{project}/deployments");
    let request = api
        .request(Method::POST, &route, api.token)?
        .multipart(form);
    let mut result = api.send(request, Method::POST, &route).await?;
    if let Some(data) = result.as_object_mut() {
        data.insert("via".into(), json!("pages-api"));
    }
    Ok(result)
}

async fn wrangler(ctx: &SiteContext, token: &str, message: &str, log: &mut Trace) -> Result<Value> {
    let version = env::var("WRANGLER_VERSION").unwrap_or_else(|_| WRANGLER_VERSION.into());
    ensure!(
        !version.is_empty()
            && version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.')),
        "invalid WRANGLER_VERSION"
    );
    let out = ctx.paths.public.canonicalize()?;
    let mut command = tokio::process::Command::new("npx");
    command
        .args(["--yes", &format!("wrangler@{version}"), "pages", "deploy"])
        .arg(out)
        .args([
            "--project-name",
            &ctx.config.project,
            "--branch",
            "main",
            "--commit-dirty=true",
            "--commit-message",
            message,
        ])
        .current_dir(&ctx.root)
        .env("CLOUDFLARE_API_TOKEN", token)
        .env("CLOUDFLARE_ACCOUNT_ID", &ctx.config.account_id)
        .env("WRANGLER_SEND_METRICS", "false")
        .env("CI", "true")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(420), command.output())
        .await
        .context("Wrangler deploy timed out")??;
    let trace = format!(
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    )
    .replace(token, "***");
    log.detail(&trace)?;
    let low = trace.to_ascii_lowercase();
    let uploaded = (low.contains("success! uploaded") || low.contains("deployment complete"))
        && (low.contains("cache/wrangler") || low.contains("permission error occurred"));
    ensure!(
        result.status.success() || uploaded,
        "Wrangler Pages deploy exited {}",
        result.status
    );
    let url = trace.split_whitespace().rfind(|s| {
        s.starts_with("https://") && (s.contains("pages.dev") || s.contains(&ctx.config.domain))
    });
    Ok(json!({"via": "wrangler", "result": {"url": url}}))
}

fn deploy_preconditions(cfg: &SiteConfig) -> Result<Url> {
    ensure!(
        cfg.platform.eq_ignore_ascii_case("pages"),
        "unsupported publish platform: {}",
        cfg.platform
    );
    project_route(cfg)?;
    let base: Url = env::var("DIGEST_CF_API_BASE")
        .unwrap_or_else(|_| CF_API.into())
        .parse()?;
    ensure!(
        !assets::USING_WEB_STUBS || loopback_base(&base),
        "live deployments require built web assets"
    );
    let _ = PagesApi::new(base.clone(), "check")?;
    Ok(base)
}

async fn deploy(
    ctx: &SiteContext,
    token: &str,
    message: &str,
    bind_domain: bool,
    log: &mut Trace,
) -> Result<Value> {
    let cfg = &ctx.config;
    let base = deploy_preconditions(cfg)?;
    let api = PagesApi::new(base, token)?;
    let result = match direct_deploy(&api, cfg, &ctx.paths.public, message).await {
        Ok(response) => Ok(response),
        Err(error) => {
            log.line(format!("Pages API failed: {error:#}. Trying Wrangler."))?;
            wrangler(ctx, token, message, log)
                .await
                .with_context(|| format!("direct upload also failed: {error:#}"))
        }
    }?;
    if bind_domain && cfg.domain != SiteConfig::default().domain {
        if let Err(error) = attach_domain(&api, cfg).await {
            log.line(format!("custom domain attachment: {error:#}"))?;
        }
    }
    Ok(result)
}

async fn attach_domain(api: &PagesApi<'_>, cfg: &SiteConfig) -> Result<()> {
    let origin = Url::parse(&geo::site_origin(&cfg.domain))?;
    let domain = origin
        .domain()
        .context("Pages custom domain requires a DNS name")?;
    let route = format!("{}/domains", project_route(cfg)?);
    let existing = api.json(Method::GET, &route, api.token, None).await?;
    let attached = existing["result"]
        .as_array()
        .context("invalid Pages custom domain listing")?
        .iter()
        .any(|row| row["name"].as_str().or_else(|| row["domain"].as_str()) == Some(domain));
    if !attached {
        api.json(
            Method::POST,
            &route,
            api.token,
            Some(json!({"name": domain})),
        )
        .await?;
    }
    Ok(())
}

async fn verify_live(
    ctx: &SiteContext,
    deployment: &Value,
    password: Option<&str>,
    log: &mut Trace,
) -> Result<usize> {
    let result = &deployment["result"];
    let host = if ctx.config.domain == SiteConfig::default().domain {
        result["url"]
            .as_str()
            .or_else(|| result["aliases"].get(0)?.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| geo::site_origin(&ctx.config.domain))
    } else {
        geo::site_origin(&ctx.config.domain)
    };
    let base = Url::parse(&host)?;
    let mut last = anyhow!("live verification did not start");
    for attempt in 1..=5 {
        match verify::verify_url_base(&ctx.paths, &base, ctx.config.encrypt, password).await {
            Ok(count) => return Ok(count),
            Err(e) => {
                log.detail(format!("live verify attempt {attempt}: {e:#}"))?;
                last = e;
            }
        }
        if attempt < 5 {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
    Err(last).context("live verification failed")
}

struct Trace {
    file: Option<fs::File>,
    verbose: bool,
}

impl Trace {
    fn line(&mut self, message: impl AsRef<str>) -> Result<()> {
        let message = message.as_ref();
        if let Some(file) = &mut self.file {
            writeln!(file, "{message}")?;
        }
        if self.file.is_none() || self.verbose {
            eprintln!("{message}");
        }
        Ok(())
    }

    fn detail(&mut self, message: impl AsRef<str>) -> Result<()> {
        if self.file.is_some() || self.verbose {
            self.line(message)?;
        }
        Ok(())
    }
}

pub fn build_demo(
    out: &Path,
    examples: &Path,
    languages: &[Lang],
    password: Option<&str>,
    config: &SiteConfig,
    verify_pages: bool,
) -> Result<usize> {
    let password = password.filter(|s| !s.is_empty());
    ensure!(
        password.is_none_or(|pw| pw.chars().count() >= 4),
        "DIGEST_DEMO_PASSWORD must have at least four characters"
    );
    ensure!(
        !languages.is_empty() && languages.iter().collect::<BTreeSet<_>>().len() == languages.len(),
        "invalid demo languages"
    );
    let multi = languages.len() > 1;
    let slug = if multi {
        ""
    } else if config.slug.is_empty() {
        "demo"
    } else {
        &config.slug
    };
    if slug != "demo" {
        crate::model::validate_slug(slug)?;
    }
    let encrypted = password.is_some();
    let cfg = SiteConfig {
        slug: slug.into(),
        encrypt: encrypted,
        lang: languages[0],
        lang_alt: None,
        root_latest: false,
        ..config.clone()
    };
    let mut editions = BTreeMap::new();
    for &lang in languages {
        let mut pages = Vec::new();
        for (kind, label, stem) in [
            ("daily", DEMO_DAILY, "sample-digest"),
            ("weekly", DEMO_WEEKLY, "sample-weekly"),
        ] {
            let issue = IssueRef::parse(&format!("{kind}/{label}"))?;
            let path = examples.join(format!("{stem}.{lang}.md"));
            let markdown = read_md(&path, lang, issue.start.year())?;
            let html = render::render_digest_html(&markdown, lang, &RenderOptions::default());
            let parsed = parse_digest_html_with_lang(&html, Some(lang))?;
            pages.push(SourcePage {
                issue,
                html,
                parsed,
            });
        }
        pages.sort_by_key(|p| p.issue.end);
        editions.insert(lang, pages);
    }
    let mut plans = Plans::default();
    for &lang in languages {
        let prefix = url_prefix(&[if multi { lang.as_str() } else { slug }]);
        let other = url_prefix(&[lang.other().as_str()]);
        Edition {
            lang,
            default_lang: languages[0],
            prefix: &prefix,
            canonical: &prefix,
            pages: &editions[&lang],
            other_prefix: multi.then_some(other.as_str()),
            other_pages: editions.get(&lang.other()).map_or(&[], Vec::as_slice),
            external: None,
            feed: "/feed.xml",
            encrypt: encrypted,
        }
        .add(&mut plans);
    }
    plans.redirect_root = plans.prefixes.first().map(|p| page_path(p, ""));
    let out = std::path::absolute(out)?;
    let canonical_out = if out.exists() {
        out.canonicalize()?
    } else {
        out.clone()
    };
    let example_path = examples.canonicalize()?;
    ensure!(
        !example_path.starts_with(&canonical_out),
        "demo output would replace the examples directory"
    );
    let out_parent = out
        .parent()
        .context("demo output requires a parent directory")?;
    if out.exists() {
        ensure!(
            fs::symlink_metadata(&out)?.file_type().is_dir(),
            "demo output must be a directory"
        );
    }
    fs::create_dir_all(out_parent)?;
    let work = tempfile::Builder::new()
        .prefix(".digest-demo-")
        .tempdir_in(out_parent)?;
    let dest = DigestPaths::at(work.path(), chrono_tz::UTC);
    let stage = ArtifactStage::new(&dest)?;
    let credentials = password.map(|pw| {
        let mut salt = vec![0; crypto::SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        Credentials {
            password: pw.into(),
            salt,
        }
    });
    let mut log = Trace {
        file: None,
        verbose: false,
    };
    build_artifacts(
        &stage,
        &mut plans,
        &BuildOptions {
            config: &cfg,
            credentials: credentials.as_ref(),
            demo_password: password,
            verify: verify_pages,
            now: Utc::now(),
            removed: &[],
            legacy_template: None,
        },
        &mut log,
    )?;
    install_outputs(&[(&stage.paths.public, &out)], work.path())?;
    Ok(plans.pages.len())
}

pub async fn run(args: &PublishArgs, code_root: &Path, paths: DigestPaths) -> Result<()> {
    if args.demo {
        let languages = match args.lang.as_deref() {
            None | Some("all") => vec![Lang::En, Lang::ZhCn],
            Some(lang) => vec![i18n::normalize_lang(lang)?],
        };
        // A demo is isolated from the production config and encryption settings.
        let cfg = SiteConfig::parse("", Some(languages[0].as_str()), |name| match name {
            "SITE_DOMAIN"
            | "DIGEST_GEO_AI_TRAIN"
            | "DIGEST_CONTENT_SIGNAL"
            | "DIGEST_GEO_WORKER"
            | "DIGEST_GEO_SEARCH" => env::var(name).ok(),
            "SITE_SLUG" if languages.len() == 1 => env::var(name).ok(),
            _ => None,
        })?;
        let examples = args
            .examples
            .clone()
            .unwrap_or_else(|| code_root.join("examples"));
        let pass = env::var("DIGEST_DEMO_PASSWORD").ok();
        let count = build_demo(
            &args.out,
            &examples,
            &languages,
            pass.as_deref(),
            &cfg,
            !args.no_verify,
        )?;
        println!(
            "demo site: {} issue(s), {count} pages → {}",
            languages.len() * 2,
            args.out.display()
        );
        return Ok(());
    }
    if !(args.init
        || args.issue.is_some()
        || args.remove.is_some()
        || args.html.is_some()
        || args.rebuild
        || args.deploy_only
        || args.rerender_all
        || args.rotate_password)
    {
        return Err(MissingMode.into());
    }
    let _lock = FileLock::acquire(&paths.publish_lock)?;
    if let Some(lang) = args.lang.as_deref() {
        i18n::normalize_lang(lang)?;
    }
    if args.init {
        let cfg = init_config(&paths, args.lang.as_deref(), args.encrypt)?;
        println!("SLUG={}\nENCRYPT={}", cfg.slug, u8::from(cfg.encrypt));
        return Ok(());
    }
    let now = Utc::now();
    let stamp = now
        .with_timezone(&paths.timezone)
        .format("%Y%m%d-%H%M%S")
        .to_string();
    let action = if args.remove.is_some() {
        Some("remove")
    } else if args.issue.is_some() {
        Some("issue")
    } else {
        None
    };
    let mut log_path =
        action.map(|action| paths.logs.join(format!("publish-{stamp}-{action}.log")));
    let log_file = if let Some(path) = &mut log_path {
        fs::create_dir_all(&paths.logs)?;
        let mut suffix = 2;
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&*path)
            {
                Ok(file) => break Some(file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    *path = paths
                        .logs
                        .join(format!("publish-{stamp}-{}-{suffix}.log", action.unwrap()));
                    suffix += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }
    } else {
        None
    };
    let mut trace = Trace {
        file: log_file,
        verbose: args.verbose,
    };
    let result = run_site(args, paths, now, &stamp, &mut trace).await;
    if let Some(action) = action {
        let log = log_path.as_ref().unwrap().display();
        match result {
            Ok(lines) => {
                println!("OK {}\nlog: {log}", lines.join("\n"));
                Ok(())
            }
            Err(error) => {
                trace.line(format!("ERROR {action}: {error:#}"))?;
                println!("ERROR {action}: {error:#}\nlog: {log}");
                Err(error)
            }
        }
    } else {
        for line in result? {
            println!("{line}");
        }
        Ok(())
    }
}

async fn run_site(
    args: &PublishArgs,
    paths: DigestPaths,
    now: DateTime<Utc>,
    stamp: &str,
    log: &mut Trace,
) -> Result<Vec<String>> {
    let config = if args.deploy_only {
        load_config(&paths.config, args.lang.as_deref())?
    } else {
        init_config(&paths, args.lang.as_deref(), false)?
    };
    let ctx = SiteContext::new(paths, config);
    let cfg = &ctx.config;
    ensure!(
        !args.rotate_password || cfg.encrypt,
        "--rotate-password requires ENCRYPT=1"
    );
    let rotating = args.rotate_password;
    let prechecked_token = if rotating && !args.prepare_only {
        match token()? {
            Some(token) => {
                deploy_preconditions(cfg)?;
                Some(token)
            }
            None => return Err(MissingToken.into()),
        }
    } else {
        None
    };
    let mut changes = FileChanges::default();
    let mut details = Vec::new();
    let mut requested_issue = None;
    if let Some(reference) = &args.remove {
        let issue = IssueRef::parse(reference)?;
        let backup = remove_issue(&ctx, &issue, stamp, &mut changes)?;
        log.line(format!("removed {} and backed it up", issue.rel()))?;
        details.push(format!(
            "removed: {} (backup: {})",
            issue.rel(),
            backup.display()
        ));
        requested_issue = Some(issue);
    }
    if let Some(reference) = &args.issue {
        let issue = IssueRef::parse(reference)?;
        let primary_html = legacy_html_path(&ctx.paths, &issue, false);
        let english_html = legacy_html_path(&ctx.paths, &issue, true);
        let (primary, english, count) = if let Some(md) = &args.md {
            let md = resolve_input(&ctx.root, md);
            let explicit = args
                .md_en
                .as_deref()
                .map(|path| resolve_input(&ctx.root, path));
            let en = discover_english_md(&md, explicit.as_deref(), ctx.lang)?;
            let (primary, english, count) = render_issue_inputs(
                &issue,
                &md,
                en.as_deref(),
                ctx.lang,
                &ctx.paths.runs,
                args.verify_urls,
            )
            .await?;
            changes.remember(&primary_html)?;
            atomic_write_text(&primary_html, &primary)?;
            if let Some(html) = &english {
                changes.remember(&english_html)?;
                atomic_write_text(&english_html, html)?;
            }
            (primary, english, Some(count))
        } else {
            let html = read_if_present(&primary_html)?
                .context("no --md given and rendered issue HTML does not exist")?;
            let en = if ctx.lang == Lang::ZhCn {
                read_if_present(&english_html)?
            } else {
                None
            };
            (html, en, None)
        };
        changes.remember(&source_path(&ctx.paths, &issue, false))?;
        save_source(&ctx.paths, &issue, false, &primary)?;
        if let Some(en) = english {
            changes.remember(&source_path(&ctx.paths, &issue, true))?;
            save_source(&ctx.paths, &issue, true, &en)?;
        }
        changes.remember(&ctx.paths.removed_paths)?;
        update_removed(&ctx.paths.removed_paths, &issue, false)?;
        let note = count.map_or_else(String::new, |n| format!(" ({n} items)"));
        details.push(format!("published: {}{note}", issue.rel()));
        log.line(format!("saved source {}", issue.rel()))?;
        requested_issue = Some(issue);
    }
    if let Some(input) = &args.html {
        let issue = IssueRef::parse(&format!(
            "{}/{}",
            args.kind.as_deref().unwrap_or_default(),
            args.date_label.as_deref().unwrap_or_default()
        ))?;
        let html = fs::read_to_string(resolve_input(&ctx.root, input))?;
        changes.remember(&source_path(&ctx.paths, &issue, false))?;
        save_source(&ctx.paths, &issue, false, &html)?;
        changes.remember(&ctx.paths.removed_paths)?;
        update_removed(&ctx.paths.removed_paths, &issue, false)?;
        requested_issue = Some(issue);
    }
    if args.rerender_all {
        rerender_sources(&ctx, log, &mut changes)?;
    }
    if args.deploy_only {
        strict_existing_artifacts(&ctx)?;
    }
    let credentials = if cfg.encrypt && !args.deploy_only {
        Some(if rotating {
            Credentials::generate()
        } else {
            Credentials::read()?
        })
    } else {
        None
    };
    let deploy_only_password = if cfg.encrypt && args.deploy_only && !args.no_verify {
        Some(verify::read_password(None)?)
    } else {
        None
    };
    let password = credentials
        .as_ref()
        .map(|c| c.password.as_str())
        .or(deploy_only_password.as_deref());
    let symmetric;
    let mut rotation_rollback = None;
    if args.deploy_only {
        if !args.no_verify {
            let count = verify::verify_local(&ctx.paths, cfg.encrypt, password)?;
            log.line(format!("local verification: {count} pages, exact match"))?;
        }
        symmetric =
            ctx.lang == Lang::ZhCn && !source_refs(&ctx.paths.sources.join("en"))?.is_empty();
    } else {
        let mut plans = site_plans(&ctx)?;
        symmetric = plans.symmetric;
        let stage = ArtifactStage::new(&ctx.paths)?;
        let removed = read_removed(&ctx.paths.removed_paths)?;
        let template = if cfg.encrypt {
            read_if_present(&ctx.paths.site.join("password_template.html"))?
        } else {
            None
        };
        let changed = build_artifacts(
            &stage,
            &mut plans,
            &BuildOptions {
                config: cfg,
                credentials: credentials.as_ref(),
                demo_password: None,
                verify: !args.no_verify,
                now,
                removed: &removed,
                legacy_template: template.as_deref(),
            },
            log,
        )?;
        if rotating {
            let rollback = RotationRollback::begin(
                stage,
                credentials
                    .as_ref()
                    .expect("credentials for password rotation"),
            )?;
            if args.prepare_only {
                rollback.commit()?;
            } else {
                rotation_rollback = Some(rollback);
            }
            log.line("rotated password and salt")?;
        } else {
            stage.install(cfg.encrypt)?;
        }
        changes.commit();
        let changed = changed
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        details.push(format!(
            "changed pages: {}",
            if changed.is_empty() { "none" } else { &changed }
        ));
    }
    let prefix = if symmetric {
        url_prefix(&[&cfg.slug, "zh"])
    } else {
        url_prefix(&[&cfg.slug])
    };
    let origin = geo::site_origin(&cfg.domain);
    let mut verified = None;
    if args.prepare_only {
        details.push("deploy: skipped (--prepare-only)".into());
    } else {
        let token = match prechecked_token {
            Some(token) => token,
            None => match token()? {
                Some(token) => token,
                None if args.issue.is_some() || args.remove.is_some() => {
                    bail!("no Cloudflare API token; the site was prepared without deploying")
                }
                None => return Err(MissingToken.into()),
            },
        };
        let message = if let Some(issue) = &requested_issue {
            format!(
                "{} {}",
                if args.remove.is_some() {
                    "remove"
                } else {
                    "publish"
                },
                issue.rel()
            )
        } else {
            "publish rebuild".into()
        };
        let bind_domain = args.issue.is_none() && args.remove.is_none();
        let deployment = match deploy(&ctx, &token, &message, bind_domain, log).await {
            Ok(deployment) => deployment,
            Err(error) => {
                if let Some(rollback) = rotation_rollback.take() {
                    if let Err(restore_err) = rollback.rollback() {
                        log.line(format!(
                            "ERROR restoring previous password/salt after rotation failure: {restore_err:#}"
                        ))?;
                    } else {
                        log.line(
                            "restored previous password, salt, and site after rotation failure",
                        )?;
                    }
                }
                return Err(error);
            }
        };
        let via = deployment["via"].as_str().unwrap_or("Pages");
        log.line(format!("deployed via {via}"))?;
        let live_count = if !args.no_verify {
            match verify_live(&ctx, &deployment, password, log).await {
                Ok(count) => Some(count),
                Err(error) => {
                    if let Some(rollback) = rotation_rollback.take() {
                        let (pw_bak, salt_bak) = rollback.preserve_after_deploy();
                        let msg = format!(
                            "WARNING: remote deployment succeeded with the rotated password/salt, \
but post-deploy live verification failed. Keeping the new local credentials and site; \
previous backups preserved at {} and {}. Re-run `ai-digest verify` after propagation or \
roll back the remote deployment manually before restoring backups.",
                            pw_bak.display(),
                            salt_bak.display()
                        );
                        log.line(&msg)?;
                        eprintln!("{msg}");
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        if let Some(rollback) = rotation_rollback.take() {
            rollback.commit()?;
        }
        details.push(format!("deploy: ok ({via})"));
        verified = live_count;
    }
    if let Some(issue) = &requested_issue {
        details.push(format!("url: {origin}{prefix}/{}", issue.rel()));
    }
    details.push(format!("index: {origin}{prefix}/"));
    if let Some(count) = verified {
        details.push(format!("verify check: OK ({count} pages, exact match)"));
    } else if !args.no_verify {
        details.push("verify check: OK (local pages, exact match)".into());
    }
    Ok(details)
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    #[test]
    fn upload_item_fit_enforces_single_asset_and_batch_caps() {
        // Max 25 MiB raw asset base64-encoded fits inside 40 MiB batch cap.
        let max_encoded = MAX_PAGES_ASSET_BYTES.div_ceil(3) * 4;
        let max_item = encoded_upload_item_bytes(max_encoded, 32, 32);
        assert!(
            validate_upload_item_fit(MAX_PAGES_ASSET_BYTES, max_item, MAX_UPLOAD_BATCH_BYTES)
                .is_ok()
        );

        // Single asset > 25 MiB is rejected.
        let over_raw = MAX_PAGES_ASSET_BYTES + 1;
        assert!(validate_upload_item_fit(over_raw, max_item, MAX_UPLOAD_BATCH_BYTES).is_err());

        // Single item whose encoded size exceeds batch cap is rejected (even on an empty batch).
        assert!(validate_upload_item_fit(1024, 2048, 2048).is_err());
        assert!(validate_upload_item_fit(1024, 2046, 2048).is_ok());
    }
}
