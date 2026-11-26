use std::{
    collections::{BTreeMap, HashSet},
    fs, io,
    path::Path,
    sync::LazyLock,
};

use anyhow::{ensure, Result};
use chrono::{Duration, NaiveDate, Utc};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::{
    i18n::{ft, Lang},
    model::{
        atomic_write_bytes, atomic_write_text, canon_url, norm_title, DigestPaths, FileLock,
        LedgerFile,
    },
};

#[derive(Debug, thiserror::Error)]
#[error("cannot read ledger: {0}")]
pub struct LedgerReadError(#[source] anyhow::Error);

impl LedgerReadError {
    pub const fn exit_code(&self) -> u8 {
        2
    }
}

struct LedgerDocument {
    data: Map<String, Value>,
    original: Option<Vec<u8>>,
}

fn load_document(path: &Path) -> Result<LedgerDocument, LedgerReadError> {
    let read = || -> Result<_> {
        match fs::read(path) {
            Ok(bytes) => {
                let doc: Map<String, Value> = serde_json::from_slice(&bytes)?;
                let items = doc.get("items").and_then(Value::as_array);
                ensure!(
                    items.is_some_and(|rows| rows.iter().all(Value::is_object)),
                    "expected JSON object with an items array of objects"
                );
                Ok(LedgerDocument {
                    data: doc,
                    original: Some(bytes),
                })
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(LedgerDocument {
                data: Map::from_iter([("items".into(), json!([]))]),
                original: None,
            }),
            Err(err) => Err(err.into()),
        }
    };
    read().map_err(LedgerReadError)
}

pub fn load_ledger(path: &Path) -> Result<LedgerFile, LedgerReadError> {
    let LedgerDocument { data, .. } = load_document(path)?;
    serde_json::from_value(Value::Object(data)).map_err(|err| LedgerReadError(err.into()))
}

fn field<'a>(item: &'a Map<String, Value>, key: &str) -> &'a str {
    item.get(key).and_then(Value::as_str).unwrap_or("")
}

fn urls(item: &Map<String, Value>) -> impl Iterator<Item = &str> {
    item.get("urls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

pub fn ledger_status(item: &Map<String, Value>) -> &'static str {
    if field(item, "status").contains("seen") || field(item, "category").contains("seen") {
        "seen"
    } else {
        "pushed"
    }
}

pub fn load_ledger_keys(path: &Path) -> Result<BTreeMap<String, &'static str>, LedgerReadError> {
    static ARXIV: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[0-9]{4}\.[0-9]{4,5}$").unwrap());
    let mut keys = BTreeMap::new();
    for item in load_ledger(path)?.items {
        let status = ledger_status(&item);
        let id = field(&item, "id");
        let name = field(&item, "name");
        let paper = ARXIV.is_match(id).then(|| format!("arxiv:{id}"));
        let title = (!name.is_empty()).then(|| format!("t:{}", norm_title(name)));
        for key in urls(&item).map(canon_url).chain(paper).chain(title) {
            if status == "pushed" {
                keys.insert(key, status);
            } else {
                keys.entry(key).or_insert(status);
            }
        }
    }
    Ok(keys)
}

pub fn find_items<'a>(ledger: &'a LedgerFile, key: &str) -> Vec<&'a Map<String, Value>> {
    let key = key.to_lowercase();
    ledger
        .items
        .iter()
        .filter(|it| {
            let fields = [it.get("name"), it.get("id"), it.get("urls")];
            json!(fields).to_string().to_lowercase().contains(&key)
        })
        .collect()
}

pub fn recent_items<'a>(
    ledger: &'a LedgerFile,
    today: NaiveDate,
    days: u32,
    status: &str,
    category: Option<&str>,
) -> Vec<&'a Map<String, Value>> {
    let cutoff = today
        .checked_sub_signed(Duration::days(i64::from(days)))
        .unwrap_or(NaiveDate::MIN)
        .to_string();
    let mut found: Vec<_> = ledger
        .items
        .iter()
        .filter(|it| {
            let mut date = field(it, "first_seen");
            if date.is_empty() {
                date = field(it, "day");
            }
            date >= cutoff.as_str()
                && (status == "all" || ledger_status(it) == status)
                && category
                    .filter(|s| !s.is_empty())
                    .is_none_or(|cat| field(it, "category") == cat)
        })
        .collect();
    found.sort_by_key(|it| field(it, "first_seen"));
    found
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AddOptions<'a> {
    pub lang: Lang,
    pub log: Option<&'a Path>,
    pub log_heading: Option<&'a str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AddResult {
    pub added: usize,
    pub updated: usize,
    pub total: usize,
}

pub fn add_items(
    paths: &DigestPaths,
    day: NaiveDate,
    stdin: &str,
    options: AddOptions<'_>,
) -> Result<AddResult> {
    let mut lock_path = paths.ledger.as_os_str().to_owned();
    lock_path.push(".lock");
    let _lock = FileLock::acquire(Path::new(&lock_path))?;
    if let Some(path) = options.log {
        ensure!(
            path != paths.ledger && path != Path::new(&lock_path),
            "filter log must be separate from ledger"
        );
    }
    let LedgerDocument {
        data: mut doc,
        original: existing,
    } = load_document(&paths.ledger)?;
    let day = day.to_string();
    let backup = paths.ledger_backup.join(format!("ledger-{day}.json"));
    if !backup.try_exists()? {
        atomic_write_bytes(
            &backup,
            existing.as_deref().unwrap_or(b"{\n \"items\": []\n}"),
        )?;
    }

    let rows = doc["items"].as_array_mut().expect("validated ledger");
    let mut result = AddResult::default();
    let mut pushed_notes = Vec::new();
    let mut seen_notes = Vec::new();
    for line in stdin
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut fields = line.split('|').map(str::trim).chain(std::iter::repeat(""));
        let fields: [&str; 6] = std::array::from_fn(|_| fields.next().unwrap());
        let [kind, name, id, category, url_list, note] = fields;
        let pushed = kind.eq_ignore_ascii_case("P");
        let notes = if pushed {
            &mut pushed_notes
        } else {
            &mut seen_notes
        };
        let colon = ft(options.lang, "lg_colon");
        notes.push(if note.is_empty() {
            format!("- {name}")
        } else {
            format!("- {name}{colon}{note}")
        });
        let id = if id.is_empty() {
            slugify(name)
        } else {
            id.into()
        };
        let input_urls: Vec<_> = url_list.split_whitespace().collect();
        let incoming: HashSet<_> = input_urls.iter().map(|u| canon_url(u)).collect();
        let hit = rows.iter_mut().filter_map(Value::as_object_mut).find(|it| {
            it.get("id").and_then(Value::as_str) == Some(id.as_str())
                || urls(it).any(|u| incoming.contains(&canon_url(u)))
        });
        let status = if pushed { "pushed" } else { "seen-not-pushed" };
        if let Some(it) = hit {
            if pushed {
                it.insert("status".into(), json!(status));
                it.insert("pushed".into(), json!(day));
                // Old ledgers used "seen" as a category in place of a status.
                if field(it, "category").contains("seen") {
                    let category = if category.is_empty() || category.contains("seen") {
                        "other"
                    } else {
                        category
                    };
                    it.insert("category".into(), json!(category));
                }
            } else if ledger_status(it) == "seen" {
                it.insert("status".into(), json!(status));
            }
            if !input_urls.is_empty() {
                let mut known: HashSet<_> = urls(it).map(canon_url).collect();
                let mut values = it
                    .get("urls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                values.extend(
                    input_urls
                        .into_iter()
                        .filter(|u| known.insert(canon_url(u)))
                        .map(|u| json!(u)),
                );
                it.insert("urls".into(), Value::Array(values));
            }
            if !note.is_empty() {
                it.insert("notes".into(), json!(note));
            }
            result.updated += 1;
        } else {
            let mut known = HashSet::new();
            let dedup: Vec<_> = input_urls
                .into_iter()
                .filter(|u| known.insert(canon_url(u)))
                .collect();
            let category = if category.is_empty() || (pushed && category.contains("seen")) {
                "other"
            } else {
                category
            };
            let mut it = json!({"day":day,"name":name,"id":id,"urls":dedup,"first_seen":day,"category":category,"status":status});
            if pushed {
                it["pushed"] = json!(day);
            }
            if !note.is_empty() {
                it["notes"] = json!(note);
            }
            rows.push(it);
            result.added += 1;
        }
    }
    result.total = rows.len();
    doc.insert("item_count".into(), json!(result.total));
    doc.insert(
        "previous_digest_at".into(),
        json!(Utc::now()
            .with_timezone(&paths.timezone)
            .format("%Y-%m-%dT%H:%M%:z")
            .to_string()),
    );
    let mut serialized = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    doc.serialize(&mut serde_json::Serializer::with_formatter(
        &mut serialized,
        formatter,
    ))?;
    atomic_write_bytes(&paths.ledger, &serialized)?;
    if let Some(path) = options.log {
        append_filter_log(path, options, &pushed_notes, &seen_notes)?;
    }
    Ok(result)
}

fn slugify(name: &str) -> String {
    static INVALID: LazyLock<Regex> = LazyLock::new(|| Regex::new("[^a-z0-9]+").unwrap());
    INVALID
        .replace_all(&name.to_lowercase(), "-")
        .trim_matches('-')
        .chars()
        .take(40)
        .collect()
}

fn append_filter_log(
    path: &Path,
    options: AddOptions<'_>,
    pushed: &[String],
    seen: &[String],
) -> Result<()> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let _lock = FileLock::acquire(Path::new(&lock_path))?;
    let mut log = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    log.push('\n');
    let level = if let Some(heading) = options.log_heading.filter(|h| !h.is_empty()) {
        log.push_str(&format!("## {heading}\n\n"));
        "###"
    } else {
        "##"
    };
    for (key, rows) in [("lg_pushed", pushed), ("lg_seen", seen)] {
        if rows.is_empty() {
            continue;
        }
        log.push_str(&format!(
            "{level} {}\n{}\n",
            ft(options.lang, key),
            rows.join("\n")
        ));
    }
    atomic_write_text(path, &log)?;
    Ok(())
}
