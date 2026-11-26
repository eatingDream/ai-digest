use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::{json, Value};
use syn::{visit::Visit, ItemUse, UseTree};
use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_ai-digest");
const EN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/sample-digest.en.md");
const ZH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/examples/sample-digest.zh-CN.md"
);
const COMMANDS: &[&str] = &[
    "fetch",
    "render",
    "publish",
    "ledger",
    "cand",
    "check-cands",
    "page",
    "weekly-late",
    "xparse",
    "verify",
];

fn cli(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(root)
        .env("DIGEST_ROOT", root)
        .env("DIGEST_TZ", "UTC")
        .env_remove("DIGEST_LANG")
        .env_remove("LEDGER_PATH")
        .env_remove("DIGEST_ENCRYPT")
        .env_remove("SITE_SLUG");
    cmd
}

fn invoke(root: &Path, args: &[&str], expected: i32) -> Output {
    checked(cli(root, args).output().unwrap(), expected)
}

fn checked(out: Output, expected: i32) -> Output {
    assert_eq!(
        out.status.code(),
        Some(expected),
        "stdout={}\nstderr={}",
        stdout(&out),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).unwrap()
}
fn decode(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap()
}
fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn help_and_version_for_root_all_commands_and_hidden_aliases() {
    let root = tempdir().unwrap();
    for command in std::iter::once("").chain(COMMANDS.iter().copied()) {
        let mut args: Vec<_> = (!command.is_empty())
            .then_some(command)
            .into_iter()
            .collect();
        args.push("--help");
        let out = invoke(root.path(), &args, 0);
        let help = stdout(&out).to_ascii_lowercase();
        assert!(
            help.contains("usage:") && help.contains("examples:") && help.contains("ai-digest "),
            "{command}: {help}"
        );
        assert!(!help.chars().any(|c| matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{20000}'..='\u{323af}')));
        *args.last_mut().unwrap() = "--version";
        assert_eq!(
            stdout(&invoke(root.path(), &args, 0)).trim(),
            format!("ai-digest {}", env!("CARGO_PKG_VERSION"))
        );
    }
    for alias in [
        "render-html",
        "check_cands",
        "weekly_late",
        "verify-decrypt",
    ] {
        assert_eq!(
            stdout(&invoke(root.path(), &[alias, "--version"], 0)).trim(),
            format!("ai-digest {}", env!("CARGO_PKG_VERSION"))
        );
    }
    let help = invoke(root.path(), &["--help"], 0);
    assert!(!stdout(&help).contains("render-html"));
}

#[test]
fn invalid_languages_and_bad_arguments_have_usage_exit() {
    let root = tempdir().unwrap();
    for args in [
        vec!["render", EN, "out.html", "--lang", "bad-lang"],
        vec!["publish", "--demo", "--lang", "bad-lang"],
        vec!["fetch", "--lang", "bad-lang"],
        vec!["ledger", "find", "key", "--lang", "bad-lang"],
        vec!["cand", "key", "--lang", "bad-lang"],
        vec!["check-cands", "paper.md", "--lang", "bad-lang"],
        vec!["page", "https://example.com", "--lang", "bad-lang"],
        vec!["weekly-late", "--lang", "bad-lang"],
        vec!["xparse", "A=missing", "--lang", "bad-lang"],
        vec!["fetch", "--start", "invalid"],
        vec!["fetch", "--daily", "--day", "+262142-12-31"],
        vec!["fetch", "--weekly", "--day", "-262143-01-01"],
        vec!["weekly-late", "--day", "+262142-12-31"],
        vec!["weekly-late", "--day", "-262143-01-01"],
        vec!["verify", "--url", "https://example.com/a"],
        vec![
            "verify",
            "--url",
            "https://example.com/a",
            "--url",
            "https://example.com/b",
            "--plain",
            "a.html",
        ],
        vec!["publish"],
    ] {
        invoke(root.path(), &args, 2);
    }
}

#[test]
fn render_json_bilingual_missing_input_and_malformed_markdown() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let single = decode(&invoke(
        root,
        &["render", EN, "nested/issue.html", "--json"],
        0,
    ));
    assert_eq!(single["output"], "nested/issue.html");
    assert_eq!(single["lang"], "en");
    assert_eq!(single["items"], 10);
    assert_eq!(single["sections"], 7);
    assert!(single["title"].as_str().unwrap().contains("AI Daily"));
    assert!(single.get("bilingual").is_none());
    assert!(!fs::read_to_string(root.join("nested/issue.html"))
        .unwrap()
        .contains("lang-pane lang-"));
    let bi = decode(&invoke(
        root,
        &["render", ZH, "bi.html", "--md-en", EN, "--json"],
        0,
    ));
    assert_eq!(bi["bilingual"], true);
    assert_eq!(bi["lang"], "zh-CN");
    let html = fs::read_to_string(root.join("bi.html")).unwrap();
    assert!(
        html.contains("lang-pane lang-en")
            && html.contains("id=\"en-item-1\"")
            && html.contains("href=\"#en-item-1\"")
    );
    let short = root.join("short-en.md");
    fs::write(
        &short,
        "**AI Daily · 1/15**\n\n## Models\n\n### 1. Example\n\n**In brief:** Example result.\n",
    )
    .unwrap();
    let warning = invoke(
        root,
        &["render", ZH, "warning.html", "--md-en", path(&short)],
        0,
    );
    assert!(String::from_utf8_lossy(&warning.stderr)
        .contains("WARNING: English has 1 items vs Chinese 10"));
    invoke(root, &["render", ZH, "bad.html", "--md-en", "absent.md"], 1);
    assert!(!root.join("bad.html").exists());
    fs::write(
        root.join("malformed.md"),
        "**AI Daily · 1/15**\nNo stories.\n",
    )
    .unwrap();
    invoke(root, &["render", "malformed.md", "bad.html"], 1);
}

#[test]
fn render_year_and_language_precedence() {
    let root = tempdir().unwrap();
    let root = root.path();
    fs::create_dir_all(root.join("site")).unwrap();
    fs::write(root.join("site/CONFIG"), "LANG=zh-CN\n").unwrap();
    fs::copy(EN, root.join("digest-2042-01-15.md")).unwrap();
    let args = ["render", "digest-2042-01-15.md", "page.html", "--json"];
    assert_eq!(decode(&invoke(root, &args, 0))["lang"], "zh-CN");
    let env = cli(root, &args).env("DIGEST_LANG", "en").output().unwrap();
    assert_eq!(decode(&checked(env, 0))["lang"], "en");
    assert!(fs::read_to_string(root.join("page.html"))
        .unwrap()
        .contains("2042"));
    let mut args = args.to_vec();
    args.extend(["--lang", "zh", "--year", "2043"]);
    let out = cli(root, &args).env("DIGEST_LANG", "en").output().unwrap();
    assert_eq!(decode(&checked(out, 0))["lang"], "zh-CN");
    assert!(fs::read_to_string(root.join("page.html"))
        .unwrap()
        .contains("2043"));
}

fn add(root: &Path, args: &[&str], input: &str) -> Value {
    let mut child = cli(root, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    decode(&checked(child.wait_with_output().unwrap(), 0))
}

#[test]
fn ledger_empty_add_find_recent_backup_log_and_isolation() {
    let a = tempdir().unwrap();
    let b = tempdir().unwrap();
    let a = a.path();
    let b = b.path();
    assert_eq!(
        decode(&invoke(a, &["ledger", "find", "Example", "--json"], 0)),
        json!({"query":{"Example":[]}})
    );
    assert!(!a.join("ledger.json").exists());
    let day = "2026-01-15";
    let args = [
        "ledger",
        "add",
        "--day",
        day,
        "--log",
        "runs/filter.md",
        "--log-heading",
        "Fixture addendum",
        "--json",
    ];
    let batch = "# ignored\nP|Example Paper|2601.01234|paper|https://example.com/a?utm_source=one https://www.example.com/a|reason\nS|Example Tool||tool|https://example.com/b|read later\n";
    assert_eq!(
        add(a, &args, batch),
        json!({"added":2,"updated":0,"total":2})
    );
    let backup = a.join("logs/ledger-backup/ledger-2026-01-15.json");
    let snapshot = fs::read(&backup).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&snapshot).unwrap(),
        json!({"items":[]})
    );
    let log = fs::read_to_string(a.join("runs/filter.md")).unwrap();
    for label in [
        "Fixture addendum",
        "Pushed",
        "reason",
        "Seen, not pushed",
        "read later",
    ] {
        assert!(log.contains(label), "missing {label}");
    }
    assert_eq!(
        add(
            a,
            &args,
            "S|Example Paper|replacement|seen|https://example.com/a|existing\n"
        ),
        json!({"added":0,"updated":1,"total":2})
    );
    assert_eq!(fs::read(&backup).unwrap(), snapshot);
    let hits = decode(&invoke(
        a,
        &["ledger", "find", "2601.01234", "EXAMPLE.COM/B", "--json"],
        0,
    ));
    assert_eq!(hits["query"]["2601.01234"][0]["status"], "pushed");
    assert_eq!(
        hits["query"]["2601.01234"][0]["urls"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(hits["query"]["EXAMPLE.COM/B"][0]["id"], "example-tool");
    let recent = decode(&invoke(
        a,
        &[
            "ledger", "recent", "--days", "9999", "--status", "seen", "--cat", "tool", "--json",
        ],
        0,
    ));
    assert_eq!(recent["count"], 1);
    assert_eq!(
        decode(&invoke(b, &["ledger", "find", "Example", "--json"], 0))["query"]["Example"],
        json!([])
    );
    fs::write(a.join("ledger.json"), "{corrupt").unwrap();
    invoke(a, &["ledger", "find", "Example"], 2);
}

#[test]
fn offline_helper_and_verifier_exit_codes() {
    let root = tempdir().unwrap();
    let root = root.path();
    let run = root.join("runs/2026-01-15");
    fs::create_dir_all(run.join("fetch")).unwrap();
    fs::write(run.join("fetch/candidates.jsonl"), r#"{"id":"abc123","extra":{"arxiv_id":"2601.01234","org":"Example Institute"},"title":"Synthetic paper","channel":"paper","source":"hf_daily","kind":"paper","pref":4,"url":"https://example.com/paper","authors":["Example Author"]}"#).unwrap();
    let cand = invoke(root, &["cand", "abc123", "missing"], 0);
    assert!(
        stdout(&cand).contains("authors: Example Author") && stdout(&cand).contains("no match")
    );
    let tmp_file = run.join("paper-cands.md.tmp");
    fs::write(
        &tmp_file,
        "## Strong\n- 2601.01234 A\n- ☆ 2601.01234 B\n- 2601.01235 C\n",
    )
    .unwrap();
    let check = decode(&invoke(
        root,
        &["check_cands", path(&tmp_file), "--finalize", "--json"],
        1,
    ));
    assert_eq!(check["duplicates"], json!(["2601.01234"]));
    assert_eq!(check["unknown"], json!(["2601.01235"]));
    assert!(tmp_file.exists());
    invoke(root, &["check-cands", "missing.md"], 1);
    assert!(
        stdout(&invoke(root, &["weekly_late", "--day", "2026-01-15"], 0))
            .contains("Synthetic paper")
    );
    assert!(stdout(&invoke(root, &["xparse", "A=missing"], 0)).contains("parse failed"));
    let blocked = invoke(root, &["page", "http://127.0.0.1"], 1);
    assert!(stdout(&blocked).contains("use WebFetch instead"));
    invoke(root, &["verify"], 2);
    invoke(root, &["verify-decrypt", "--local"], 2);
    for dir in ["site/plain", "site/public"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(
        root.join("site/plain/index.html"),
        b"<html>example</html>\r\n",
    )
    .unwrap();
    fs::write(
        root.join("site/public/index.html"),
        b"<html>example</html>\r\n",
    )
    .unwrap();
    assert!(stdout(&invoke(root, &["verify", "--local"], 0)).contains("all 1 page(s)"));
    fs::write(
        root.join("site/public/index.html"),
        b"<html>example</html>\n",
    )
    .unwrap();
    invoke(root, &["verify", "--local"], 1);
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension() == Some(OsStr::new("rs")) {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn layer(module: &str) -> Option<u8> {
    Some(match module {
        "version" | "i18n" | "assets" | "math" | "net" => 0,
        "model" => 1,
        "parser" | "ledger" | "render" | "crypto" | "fetch::sources" => 2,
        "geo" | "verify" | "fetch::tools" | "fetch::pipeline" => 3,
        "fetch" | "publish" => 4,
        "lib" | "main" => 5,
        _ => return None,
    })
}

fn imports(tree: &UseTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            imports(&p.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Group(group) => {
            for child in &group.items {
                imports(child, prefix, out);
            }
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            if name.ident != "self" {
                path.push(name.ident.to_string());
            }
            out.push(path);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            out.push(path);
        }
        UseTree::Glob(_) => out.push(prefix.clone()),
    }
}

#[derive(Default)]
struct UseEdges(Vec<Vec<String>>);

impl<'ast> Visit<'ast> for UseEdges {
    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        imports(&node.tree, &mut Vec::new(), &mut self.0);
    }
}

#[test]
fn every_source_import_edge_is_strictly_lower_triangular() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0;
    let mut modules = BTreeSet::new();
    let mut violations = Vec::new();
    for file in rust_files(&root) {
        let relative = file.strip_prefix(&root).unwrap();
        let module = relative
            .with_extension("")
            .to_string_lossy()
            .replace('/', "::");
        let module = module.strip_suffix("::mod").unwrap_or(&module);
        let importer =
            layer(module).unwrap_or_else(|| panic!("assign a DAG layer to {}", relative.display()));
        modules.insert(module.to_owned());
        let ast = syn::parse_file(&fs::read_to_string(&file).unwrap()).unwrap();
        let mut collector = UseEdges::default();
        collector.visit_file(&ast);
        for path in collector.0 {
            let Some((namespace, suffix)) = path.split_first() else {
                continue;
            };
            if namespace != "crate" && namespace != "ai_digest" {
                continue;
            }
            let target = match suffix {
                [first, sub, ..]
                    if first == "fetch"
                        && matches!(sub.as_str(), "sources" | "tools" | "pipeline") =>
                {
                    format!("fetch::{sub}")
                }
                [first, ..] => first.clone(),
                _ => panic!("crate root glob or alias is not a verifiable dependency"),
            };
            let imported = layer(&target)
                .unwrap_or_else(|| panic!("unknown DAG module {target} in {}", relative.display()));
            checked += 1;
            if importer <= imported {
                violations.push(format!("{module} (L{importer}) → {target} (L{imported})"));
            }
        }
    }
    assert_eq!(modules.len(), 19, "DAG must cover all source modules");
    assert!(checked >= 40, "only {checked} imports were audited");
    assert!(
        violations.is_empty(),
        "invalid import edges:\n{}",
        violations.join("\n")
    );
    println!(
        "audited {checked} import targets across {} Rust modules",
        modules.len()
    );
}
