use std::{
    collections::{BTreeMap, VecDeque},
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::{Duration, Instant},
};

use anyhow::{bail, ensure, Context, Result};
use chrono::NaiveDate;
use chrono_tz::Tz;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::i18n::{normalize_lang, resolve_lang_with_env, Lang};

#[derive(Clone, Debug)]
pub struct DigestPaths {
    pub root: PathBuf,
    pub ledger: PathBuf,
    pub ledger_backup: PathBuf,
    pub timezone: Tz,
    pub state: PathBuf,
    pub runs: PathBuf,
    pub html: PathBuf,
    pub logs: PathBuf,
    pub site: PathBuf,
    pub config: PathBuf,
    pub sources: PathBuf,
    pub plain: PathBuf,
    pub public: PathBuf,
    pub enc_cache: PathBuf,
    pub removed_paths: PathBuf,
    pub removed_backup: PathBuf,
    pub publish_lock: PathBuf,
    pub preferences: PathBuf,
    pub registry: PathBuf,
}

impl DigestPaths {
    pub fn from_env(code_root: impl AsRef<Path>) -> Result<Self> {
        Self::with_env(code_root.as_ref(), |key| env::var(key).ok())
    }

    pub fn with_env(code_root: &Path, get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let var = |key| {
            get(key)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let root_override = var("DIGEST_ROOT");
        let root = root_override
            .as_ref()
            .map_or_else(|| code_root.to_owned(), PathBuf::from);
        let timezone = var("DIGEST_TZ")
            .unwrap_or_else(|| "UTC".into())
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid DIGEST_TZ"))?;
        let mut paths = Self::at(&root, timezone);
        if root_override.is_none() {
            paths.state = code_root.join("fetch/state");
        }
        paths.registry = code_root.join("fetch/sources.toml");
        if let Some(p) = var("DIGEST_PREFERENCES_FILE") {
            paths.preferences = PathBuf::from(p);
        }
        if let Some(p) = var("LEDGER_PATH") {
            paths.ledger = PathBuf::from(p);
            paths.ledger_backup = parent(&paths.ledger).join("logs/ledger-backup");
        }
        Ok(paths)
    }

    pub fn at(root: impl AsRef<Path>, timezone: Tz) -> Self {
        let root = root.as_ref();
        let site = root.join("site");
        Self {
            root: root.into(),
            timezone,
            ledger: root.join("ledger.json"),
            ledger_backup: root.join("logs/ledger-backup"),
            state: root.join("state"),
            runs: root.join("runs"),
            html: root.join("html"),
            logs: root.join("logs"),
            config: site.join("CONFIG"),
            sources: site.join("sources"),
            plain: site.join("plain"),
            public: site.join("public"),
            enc_cache: site.join("enc-cache.json"),
            removed_paths: site.join("removed-paths.txt"),
            removed_backup: root.join("backfill/removed"),
            publish_lock: root.join(".publish.lock"),
            preferences: root.join("fetch/preferences.json"),
            registry: root.join("fetch/sources.toml"),
            site,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LangAlt {
    pub lang: Lang,
    pub base: String,
}

pub const RESERVED_SLUGS: &[&str] = &[
    ".",
    "..",
    "_headers",
    "_redirects",
    "_worker.js",
    "_routes.json",
    "robots.txt",
    "404.html",
    "favicon.svg",
    "sitemap.xml",
    "llms.txt",
    "llms-full.txt",
    "feed.xml",
    "search-index.json",
    "zh",
    "en",
    "daily",
    "weekly",
    "latest",
];

#[derive(Clone, Debug)]
pub struct SiteConfig {
    pub platform: String,
    pub project: String,
    pub account_id: String,
    pub zone_id: String,
    pub domain: String,
    pub slug: String,
    pub encrypt: bool,
    pub lang: Lang,
    pub lang_alt: Option<LangAlt>,
    pub root_latest: bool,
    pub geo_ai_train: bool,
    pub content_signal: String,
    pub geo_worker: bool,
    pub geo_search: bool,
}

impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            platform: "pages".into(),
            project: "ai-digest".into(),
            account_id: String::new(),
            zone_id: String::new(),
            domain: "digest.example.com".into(),
            slug: String::new(),
            encrypt: false,
            lang: Lang::En,
            lang_alt: None,
            root_latest: false,
            geo_ai_train: true,
            content_signal: "ai-train=yes, search=yes, ai-input=yes".into(),
            geo_worker: true,
            geo_search: true,
        }
    }
}

impl SiteConfig {
    pub fn load(path: &Path, cli_lang: Option<&str>) -> Result<Self> {
        let input = match fs::read_to_string(path) {
            Ok(input) => input,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err.into()),
        };
        if let Some(raw) = env::var_os("DIGEST_ENCRYPT") {
            ensure!(
                raw.to_str().is_some(),
                "invalid DIGEST_ENCRYPT value: must be valid UTF-8"
            );
        }
        Self::parse(&input, cli_lang, |key| env::var(key).ok())
    }

    pub fn parse(
        input: &str,
        cli_lang: Option<&str>,
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        let mut data: BTreeMap<String, String> = input
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                (!line.starts_with('#'))
                    .then(|| line.split_once('='))
                    .flatten()
            })
            .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
            .collect();
        for (alias, key) in [
            ("CF_ACCOUNT_ID", "ACCOUNT_ID"),
            ("CF_PROJECT", "PROJECT"),
            ("CF_PAGES_PROJECT", "PROJECT"),
        ] {
            if let Some(value) = data.get(alias).cloned() {
                data.entry(key.into()).or_insert(value);
            }
        }
        for (var, key) in [
            ("SITE_DOMAIN", "DOMAIN"),
            ("SITE_SLUG", "SLUG"),
            ("CF_ACCOUNT_ID", "ACCOUNT_ID"),
            ("CF_ZONE_ID", "ZONE_ID"),
            ("CF_PROJECT", "PROJECT"),
            ("CF_PAGES_PROJECT", "PROJECT"),
            ("DIGEST_LANG_ALT", "LANG_ALT"),
            ("DIGEST_ROOT_LATEST", "ROOT_LATEST"),
            ("DIGEST_ENCRYPT", "ENCRYPT"),
            ("DIGEST_GEO_AI_TRAIN", "GEO_AI_TRAIN"),
            ("DIGEST_CONTENT_SIGNAL", "CONTENT_SIGNAL"),
            ("DIGEST_GEO_WORKER", "GEO_WORKER"),
            ("DIGEST_GEO_SEARCH", "GEO_SEARCH"),
        ] {
            if let Some(value) = get(var) {
                let value = value.trim();
                if !value.is_empty() || key == "SLUG" {
                    data.insert(key.into(), value.into());
                }
            }
        }
        static PLACEHOLDER: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"[<>]|\{\{[^{}]*\}\}").unwrap());
        for (key, value) in &data {
            ensure!(
                !PLACEHOLDER.is_match(value),
                "{key} contains an unreplaced placeholder"
            );
        }
        let val = |key: &str| data.get(key).map(String::as_str).unwrap_or("");
        let lang =
            resolve_lang_with_env(cli_lang, get("DIGEST_LANG").as_deref(), Some(val("LANG")))?;
        validate_slug(val("SLUG"))?;
        let (content_signal, geo_ai_train) =
            content_signal(val("CONTENT_SIGNAL"), val("GEO_AI_TRAIN"));
        let defaults = Self::default();
        Ok(Self {
            lang,
            slug: val("SLUG").into(),
            lang_alt: parse_lang_alt(val("LANG_ALT"), lang)?,
            platform: data.get("PLATFORM").cloned().unwrap_or(defaults.platform),
            project: data.get("PROJECT").cloned().unwrap_or(defaults.project),
            domain: data.get("DOMAIN").cloned().unwrap_or(defaults.domain),
            account_id: val("ACCOUNT_ID").into(),
            zone_id: val("ZONE_ID").into(),
            encrypt: parse_bool_strict(val("ENCRYPT"), "ENCRYPT", false)?,
            root_latest: truthy(val("ROOT_LATEST")),
            geo_worker: !falsy(val("GEO_WORKER")),
            geo_search: !falsy(val("GEO_SEARCH")),
            content_signal,
            geo_ai_train,
        })
    }
}

pub fn validate_slug(slug: &str) -> Result<()> {
    if !slug.is_empty() {
        ensure!(
            slug.len() >= 8
                && !slug.starts_with('.')
                && slug
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
                && !RESERVED_SLUGS.contains(&slug.to_ascii_lowercase().as_str()),
            "invalid SLUG: use at least 8 URL-safe characters and avoid reserved names"
        );
    }
    Ok(())
}

pub fn is_protected_security_header(name: &str, encrypted: bool) -> bool {
    let low = name.trim().to_ascii_lowercase();
    matches!(
        low.as_str(),
        "content-security-policy"
            | "x-frame-options"
            | "x-content-type-options"
            | "referrer-policy"
            | "strict-transport-security"
            | "permissions-policy"
    ) || low.starts_with("cross-origin-")
        || (encrypted && low == "x-robots-tag")
}

pub fn parse_header_removal_target(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix('!')?.trim();
    let name = rest.split_once(':').map_or(rest, |(k, _)| k).trim();
    let name = name.split_whitespace().next().unwrap_or("");
    (!name.is_empty()).then_some(name)
}

fn parse_bool_strict(s: &str, name: &str, default: bool) -> Result<bool> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("invalid {name} value: expected 0/1/true/false/yes/no/on/off"),
    }
}

fn truthy(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
fn falsy(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

pub fn content_signal(raw: &str, ai_train: &str) -> (String, bool) {
    let mut values = [true; 3];
    let labels = ["ai-train", "search", "ai-input"];
    for (key, value) in raw.split(',').filter_map(|part| part.split_once('=')) {
        if let Some(i) = labels
            .iter()
            .position(|label| key.trim().eq_ignore_ascii_case(label))
        {
            match value.trim().to_ascii_lowercase().as_str() {
                "yes" => values[i] = true,
                "no" => values[i] = false,
                _ => {}
            }
        }
    }
    if truthy(ai_train) || falsy(ai_train) {
        values[0] = truthy(ai_train);
    }
    let signal = labels
        .into_iter()
        .zip(values)
        .map(|(k, v)| format!("{k}={}", if v { "yes" } else { "no" }))
        .collect::<Vec<_>>()
        .join(", ");
    (signal, values[0])
}

fn parse_lang_alt(raw: &str, current: Lang) -> Result<Option<LangAlt>> {
    if raw.is_empty() {
        return Ok(None);
    }
    ensure!(
        !raw.chars().any(|c| c.is_control() || c == '\\'),
        "invalid LANG_ALT"
    );
    static PREFIX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z_-]{1,9}):(.+)$").unwrap());
    let (lang, base) = if let Some(m) = PREFIX.captures(raw).filter(|m| !m[2].starts_with("//")) {
        (normalize_lang(&m[1])?, m.get(2).unwrap().as_str().trim())
    } else {
        (current.other(), raw)
    };
    ensure!(
        !base.is_empty() && !base.chars().any(char::is_whitespace),
        "invalid LANG_ALT"
    );
    let base = if base.to_ascii_lowercase().starts_with("http://")
        || base.to_ascii_lowercase().starts_with("https://")
    {
        let url = url::Url::parse(base)?;
        ensure!(
            url.has_host() && url.query().is_none() && url.fragment().is_none(),
            "invalid LANG_ALT URL"
        );
        format!("{}/", base.trim_end_matches('/'))
    } else {
        ensure!(
            !base.starts_with("//")
                && !base.split('/').any(|s| s == "..")
                && base
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~/-".contains(&b)),
            "invalid LANG_ALT path"
        );
        let path = base.trim_matches('/');
        if path.is_empty() {
            "/".into()
        } else {
            format!("/{path}/")
        }
    };
    Ok(Some(LangAlt { lang, base }))
}

#[derive(Clone, Debug)]
pub struct SiteContext {
    pub root: PathBuf,
    pub config: SiteConfig,
    pub paths: DigestPaths,
    pub lang: Lang,
}

impl SiteContext {
    pub fn new(paths: DigestPaths, config: SiteConfig) -> Self {
        Self {
            root: paths.root.clone(),
            lang: config.lang,
            config,
            paths,
        }
    }
    pub fn load(code_root: &Path, cli_lang: Option<&str>) -> Result<Self> {
        let paths = DigestPaths::from_env(code_root)?;
        let config = SiteConfig::load(&paths.config, cli_lang)?;
        Ok(Self::new(paths, config))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    pub lang: Lang,
    pub title: String,
    pub period: String,
    pub weekly: bool,
    pub dates: Vec<NaiveDate>,
    pub must: Vec<MustItem>,
    pub themes: Vec<ThemeItem>,
    pub toc_groups: Vec<TocGroup>,
    pub toc_note: String,
    pub sections: Vec<Section>,
}

impl Digest {
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.sections.iter().flat_map(Section::items)
    }
    pub fn full_title(&self) -> String {
        if self.period.is_empty() {
            self.title.clone()
        } else {
            format!("{} · {}", self.title, self.period)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub note: String,
    pub entries: Vec<SectionEntry>,
}
impl Section {
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.entries.iter().filter_map(|entry| match entry {
            SectionEntry::Item(item) => Some(item.as_ref()),
            SectionEntry::Subheading(_) => None,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectionEntry {
    Subheading(String),
    Item(Box<Item>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub number: String,
    pub title: String,
    pub is_new: bool,
    pub meta: String,
    pub source: String,
    pub when: String,
    pub tags: Vec<String>,
    pub dek: String,
    pub look: String,
    pub detail: String,
    pub followup: String,
    pub extra: String,
    pub links: Vec<LinkItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MustItem {
    pub title: String,
    pub when: String,
    pub dek: String,
    pub anchor: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeItem {
    pub title: String,
    pub body: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TocEntry {
    pub number: String,
    pub title: String,
    pub is_new: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TocGroup {
    pub label: String,
    pub entries: Vec<TocEntry>,
    pub note: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkItem {
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedIssue {
    #[serde(skip)]
    pub lang: Lang,
    pub title: String,
    pub edition: String,
    pub meta_spans: Vec<String>,
    pub period_line: String,
    pub stats_line: String,
    pub motto: String,
    pub description: String,
    pub top_tags: Vec<String>,
    pub must: Vec<ParsedMust>,
    pub themes: Vec<ThemeItem>,
    pub sections: Vec<ParsedSection>,
    pub items: Vec<ParsedItem>,
    pub items_count: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedMust {
    pub title: String,
    pub dek: String,
    pub anchor: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedSection {
    pub id: String,
    pub title: String,
    pub items: Vec<ParsedItem>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedItem {
    pub id: String,
    pub number: String,
    pub title: String,
    pub source: String,
    pub when: String,
    pub tags: Vec<String>,
    pub dek: String,
    pub look: String,
    pub detail: String,
    pub extra: String,
    pub links: Vec<(String, String)>,
    pub section: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InLedger {
    Status(String),
    Flag(bool),
}
impl Default for InLedger {
    fn default() -> Self {
        Self::Flag(false)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cand {
    pub title: String,
    pub url: String,
    pub source: String,
    pub channel: String,
    pub kind: String,
    pub published: Option<String>,
    #[serde(deserialize_with = "nullable_default")]
    pub summary: String,
    #[serde(deserialize_with = "nullable_default")]
    pub authors: Vec<String>,
    #[serde(deserialize_with = "nullable_default")]
    pub extra: Map<String, Value>,
    pub id: String,
    #[serde(deserialize_with = "nullable_default")]
    pub topics: Vec<String>,
    pub pref: i32,
    #[serde(deserialize_with = "nullable_default")]
    pub also_in: Vec<String>,
    #[serde(deserialize_with = "nullable_default")]
    pub in_ledger: InLedger,
}

fn nullable_default<'de, T: Deserialize<'de> + Default, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceSpec {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub channel: String,
    pub kind: String,
    pub url: Option<String>,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub orgs: Vec<String>,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub queries: Vec<String>,
    pub filter: Option<String>,
    pub pattern: Option<String>,
    pub exclude: Option<String>,
    pub max_age_days: Option<u32>,
    pub min_points: Option<u32>,
    pub user_agent: Option<String>,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default = "enabled")]
    pub resolve: bool,
    pub resolve_max: Option<usize>,
    pub first_run_keep: Option<usize>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}
const fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub settings: BTreeMap<String, toml::Value>,
    pub source: Vec<SourceSpec>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LedgerFile {
    pub items: Vec<Map<String, Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FetchReport {
    pub window_start: String,
    pub window_end: String,
    pub generated_at: String,
    pub elapsed_sec: f64,
    pub total_raw: usize,
    pub total_dedup: usize,
    pub notes: Vec<String>,
    pub sources: Vec<FetchSourceReport>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FetchSourceReport {
    pub id: String,
    pub name: String,
    pub channel: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub n: usize,
    pub sec: f64,
    pub status: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FetchState(pub Map<String, Value>);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDoc {
    #[serde(rename = "u")]
    pub url: String,
    #[serde(rename = "m")]
    pub markdown: String,
    #[serde(rename = "l")]
    pub lang: Lang,
    #[serde(rename = "k")]
    pub kind: String,
    #[serde(rename = "d")]
    pub date: String,
    #[serde(rename = "it")]
    pub issue_title: String,
    #[serde(rename = "t")]
    pub title: String,
    #[serde(rename = "s")]
    pub source: String,
    #[serde(rename = "g")]
    pub tags: Vec<String>,
    #[serde(rename = "p")]
    pub snippet: String,
    #[serde(rename = "lk")]
    pub links: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerConfig {
    pub origin: String,
    pub content_signal: String,
    pub link_header: String,
    pub search_enabled: bool,
    pub global_headers: BTreeMap<String, String>,
    pub cache_rules: Vec<(String, String)>,
    pub redirects: Vec<(String, String, u16)>,
}

pub fn canon_url(input: &str) -> String {
    let input = input.trim();
    if input.is_empty() {
        return String::new();
    }
    const TRACKING: &[&str] = &[
        "ref", "ref_src", "source", "rss", "src", "fbclid", "gclid", "mc_cid", "mc_eid", "smid",
        "cmpid", "sr_share",
    ];
    static ARXIV: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:arxiv\.org/(?:abs|pdf|html)/|huggingface\.co/papers/)(\d{4}\.\d{4,5})")
            .unwrap()
    });
    if let Some(m) = ARXIV.captures(input) {
        return format!("arxiv:{}", &m[1]);
    }
    let input = input.replace(['\t', '\n', '\r'], "");
    let (scheme, rest) = input
        .split_once(':')
        .filter(|(prefix, _)| has_scheme(&format!("{prefix}:")))
        .unwrap_or(("", &input));
    let (host, rest) = if let Some(authority) = rest.strip_prefix("//") {
        let end = authority.find(['/', '?', '#']).unwrap_or(authority.len());
        (&authority[..end], &authority[end..])
    } else {
        ("", rest)
    };
    let host = host.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let host = host.strip_prefix("m.").unwrap_or(host);
    let rest = rest.split('#').next().unwrap_or("");
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = if matches!(
        scheme.to_ascii_lowercase().as_str(),
        "http" | "https" | "ftp" | ""
    ) {
        let last_segment = path.rfind('/').map_or(0, |i| i + 1);
        path[last_segment..]
            .find(';')
            .map_or(path, |i| &path[..i + last_segment])
    } else {
        path
    }
    .trim_end_matches('/');
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            url::form_urlencoded::parse(query.as_bytes()).filter(|(key, val)| {
                let key = key.to_ascii_lowercase();
                !val.is_empty() && !key.starts_with("utm_") && !TRACKING.contains(&key.as_str())
            }),
        )
        .finish()
        .replace('*', "%2A")
        .replace("%7E", "~");
    let path = if path.starts_with('/') {
        path.into()
    } else {
        format!("/{path}")
    };
    if query.is_empty() {
        format!("https://{host}{path}")
    } else {
        format!("https://{host}{path}?{query}")
    }
}

fn has_scheme(input: &str) -> bool {
    input.split_once(':').is_some_and(|(before, _)| {
        before
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && before
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
    })
}

pub fn norm_title(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .take(80)
        .collect()
}

pub fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn is_safe_url(input: &str) -> bool {
    normalized_safe_url(input).is_some()
}

/// Return the same normalized value that was checked so the browser receives that value.
pub fn normalized_safe_url(input: &str) -> Option<String> {
    let raw = html_unescape(input);
    if raw.bytes().any(|b| b.is_ascii_control()) || input.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }
    let cleaned = raw.trim().replace(' ', "");
    if cleaned.is_empty()
        || cleaned.starts_with("//")
        || cleaned.starts_with('\\')
        || cleaned
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .contains('\\')
    {
        return None;
    }
    if has_scheme(&cleaned) {
        let (scheme, rest) = cleaned.split_once(':')?;
        if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
            return None;
        }
        let authority = rest.strip_prefix("//")?.split(['/', '?', '#']).next()?;
        if authority.is_empty() {
            return None;
        }
    }
    Some(cleaned)
}

pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

pub fn html_unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let input = text.replace('<', "&lt;");
    html_tokens(&input)
        .filter_map(|token| match token {
            html5gum::Token::String(s) => Some(String::from_utf8_lossy(&s.0).into_owned()),
            _ => None,
        })
        .collect()
}

pub fn escape_json_for_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub fn html_tokens(html: &str) -> impl Iterator<Item = html5gum::Token> + '_ {
    let mut emitter = html5gum::DefaultEmitter::default();
    emitter.naively_switch_states(true);
    html5gum::Tokenizer::new_with_emitter(html, emitter)
        .map(|token| token.expect("string input is infallible"))
}

pub fn strip_marked(html: &str, marker: &str) -> String {
    let start = format!("<!-- {marker}:start -->");
    let end = format!("<!-- {marker}:end -->");
    let mut result = String::with_capacity(html.len());
    let mut rest = html;
    while let Some((before, block)) = rest.split_once(&start) {
        let Some((_, after)) = block.split_once(&end) else {
            break;
        };
        result.push_str(before);
        rest = after;
    }
    result.push_str(rest);
    result
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

pub fn atomic_write_bytes(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = parent(path);
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(data)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

pub fn atomic_write_text(path: &Path, text: &str) -> io::Result<()> {
    atomic_write_bytes(path, text.as_bytes())
}

pub fn files_under(root: &Path) -> io::Result<Vec<PathBuf>> {
    if !root.try_exists()? {
        return Ok(Vec::new());
    }
    let mut queue = VecDeque::from([root.to_path_buf()]);
    let mut files = Vec::new();
    while let Some(dir) = queue.pop_front() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                queue.push_back(entry.path());
            } else if ty.is_file() && entry.file_name() != ".DS_Store" {
                files.push(entry.path());
            } else if ty.is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected symlink in output tree",
                ));
            }
        }
    }
    files.sort_unstable();
    Ok(files)
}

pub struct FileLock(fs::File);

impl FileLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        Self::with_timeout(path, Duration::from_secs(900), Duration::from_secs(5))
    }

    pub fn with_timeout(path: &Path, timeout: Duration, poll: Duration) -> Result<Self> {
        fs::create_dir_all(parent(path))?;
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let start = Instant::now();
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self(file)),
                Err(err) if err.kind() == fs2::lock_contended_error().kind() => {
                    let remaining = timeout.saturating_sub(start.elapsed());
                    if remaining.is_zero() {
                        bail!("timed out acquiring file lock");
                    }
                    std::thread::sleep(remaining.min(poll));
                }
                Err(err) => return Err(err).context("acquire file lock"),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
