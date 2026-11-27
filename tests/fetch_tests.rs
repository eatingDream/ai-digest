use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

use ai_digest::{
    fetch::{
        pipeline::dedupe,
        sources::{
            clean_text, compute_window, date_from_text, fetch_source, paper_signals, parse_dt,
            parse_registry, score_topics, SourceContext, CHANNELS, KINDS, TYPES,
        },
        tools::{page_text, pages_with_client, x_json, PageArgs},
    },
    i18n::Lang,
    model::{atomic_write_text, canon_url, norm_title, Cand, FetchState, InLedger},
    net::SafeHttpClient,
};
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde_json::{json, Value};
use tempfile::tempdir;
use wiremock::{
    matchers::{header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const BIN: &str = env!("CARGO_BIN_EXE_ai-digest");

fn at(s: &str) -> DateTime<Utc> {
    parse_dt(s).unwrap().with_timezone(&Utc)
}

fn paper(id: &str, title: &str) -> Cand {
    Cand {
        id: "abc123def456".into(),
        kind: "paper".into(),
        channel: "paper".into(),
        source: "arxiv".into(),
        url: format!("https://arxiv.org/abs/{id}"),
        title: title.into(),
        extra: json!({"arxiv_id": id}).as_object().unwrap().clone(),
        ..Cand::default()
    }
}

fn rows(dir: &Path, items: &[Cand]) {
    let lines: String = items
        .iter()
        .map(|item| format!("{}\n", serde_json::to_string(item).unwrap()))
        .collect();
    atomic_write_text(&dir.join("candidates.jsonl"), &lines).unwrap();
}

fn fetch(root: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new(BIN)
        .args(["fetch"])
        .args(args)
        .current_dir(root)
        .env("DIGEST_ROOT", root)
        .env("DIGEST_TZ", "UTC")
        .env_remove("LEDGER_PATH")
        .env_remove("DIGEST_LANG")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

#[test]
fn registry_schema_includes_all_sources_types_channels_and_kinds() {
    let (settings, entries) = parse_registry(include_str!("../fetch/sources.toml")).unwrap();
    assert_eq!(entries.len(), 155);
    assert_eq!(entries.iter().filter(|s| s.enabled).count(), 153);
    assert_eq!(
        (settings.timeout, settings.workers, settings.summary_chars),
        (20, 24, 700)
    );
    assert_eq!(
        entries
            .iter()
            .map(|s| s.id.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        entries.len()
    );
    for (known, actual) in [
        (
            TYPES,
            entries
                .iter()
                .map(|s| s.source_type.as_str())
                .collect::<BTreeSet<_>>(),
        ),
        (
            CHANNELS,
            entries.iter().map(|s| s.channel.as_str()).collect(),
        ),
        (KINDS, entries.iter().map(|s| s.kind.as_str()).collect()),
    ] {
        assert_eq!(known.iter().copied().collect::<BTreeSet<_>>(), actual);
    }
    for entry in &entries {
        for list in [&entry.orgs, &entry.repos, &entry.categories, &entry.urls] {
            assert_eq!(
                list.iter().collect::<BTreeSet<_>>().len(),
                list.len(),
                "{}",
                entry.id
            );
        }
    }
    let invalid = r#"
[[source]]
id = "sample"
name = "Test source"
type = "github_releases"
channel = "tooling"
kind = "release"
repos = ["example/a", "example/a"]
"#;
    assert!(parse_registry(invalid)
        .unwrap_err()
        .to_string()
        .contains("repos"));
}

#[test]
fn canonical_urls_normalization_and_dedupe_preserve_source_metadata() {
    assert_eq!(
        canon_url("https://www.example.com/post/?utm_source=x&id=3&ref=y"),
        "https://example.com/post?id=3"
    );
    for url in [
        "https://arxiv.org/abs/2601.01234",
        "https://arxiv.org/pdf/2601.01234v2.pdf",
        "https://huggingface.co/papers/2601.01234",
    ] {
        assert_eq!(canon_url(url), "arxiv:2601.01234");
    }
    assert_eq!(norm_title("Hello, World! 你好"), "helloworld你好");
    assert_eq!(norm_title(&"x".repeat(200)).len(), 80);
    assert_eq!(clean_text("<p>a  <b>b</b>&amp;c</p>", 0), "a b&c");
    assert_eq!(clean_text(&"字".repeat(20), 4), "字字字…");
    let a = paper("2601.01234", "Same long title for dedupe");
    let mut b = a.clone();
    b.url = "https://huggingface.co/papers/2601.01234".into();
    b.source = "hf_daily".into();
    b.summary = "Synthetic abstract".into();
    b.extra
        .insert("github".into(), json!("https://github.com/example/repo"));
    let rows = dedupe(vec![a, b]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].also_in, ["hf_daily"]);
    assert_eq!(rows[0].extra["hf_daily"], true);
    assert_eq!(rows[0].extra["github"], "https://github.com/example/repo");
    assert_eq!(rows[0].summary, "Synthetic abstract");
    assert_eq!(rows[0].id.len(), 12);

    // SEC-2: arXiv ID extraction only on exact arxiv.org / huggingface.co hosts
    assert_ne!(
        canon_url("https://evil.example/?q=arxiv.org/abs/2601.01234"),
        "arxiv:2601.01234"
    );
    // SEC-2: never merge distinct URLs purely on matching titles
    let mut c1 = paper("2601.09999", "Identical long title across distinct urls");
    c1.url = "https://lab-a.example/post".into();
    c1.extra.clear();
    let mut c2 = c1.clone();
    c2.url = "https://evil-b.example/post".into();
    assert_eq!(dedupe(vec![c1, c2]).len(), 2);
}

#[test]
fn dates_and_windows_cover_timezones_boundaries_and_dst() {
    let dt = parse_dt("Thu, 15 Jan 2026 08:00:00 +0800").unwrap();
    assert_eq!(dt.with_timezone(&Utc).hour(), 0);
    assert_eq!(
        parse_dt("2026-01-15T00:00+08:00")
            .unwrap()
            .offset()
            .local_minus_utc(),
        28_800
    );
    assert_eq!(
        parse_dt("2026-11-15 13:19:51  +0800")
            .unwrap()
            .offset()
            .local_minus_utc(),
        28800
    );
    assert_eq!(parse_dt("2026-01-15").unwrap().day(), 15);
    assert!(parse_dt("2026-02-30").is_none());
    for (text, date) in [
        ("Published Oct 05, 2026", "2026-10-05"),
        ("13 Nov 2026", "2026-11-13"),
        ("2026年11月13日 公告", "2026-11-13"),
        ("https://example.com/news261113", "2026-11-13"),
        ("2026-11/13 release", "2026-11-13"),
        ("2026.11.13 release", "2026-11-13"),
    ] {
        assert_eq!(date_from_text(text).unwrap().date_naive().to_string(), date);
    }
    assert!(date_from_text("nothing here").is_none());
    let now = at("2026-01-16T01:00:00Z");
    let tz = chrono_tz::Asia::Taipei;
    let daily = compute_window(tz, now, false, true, None, None, None).unwrap();
    assert_eq!(daily.label, "2026-01-15");
    assert_eq!(daily.iso(daily.start), "2026-01-15T00:00+08:00");
    assert!(daily.contains(daily.start));
    assert!(!daily.contains(daily.end));
    let week = compute_window(
        tz,
        now,
        true,
        false,
        Some("2026-01-15".parse().unwrap()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(week.label, "2026-01-18-weekly");
    assert_eq!((week.end - week.start).num_days(), 7);
    let custom = compute_window(
        tz,
        now,
        false,
        false,
        None,
        Some("2026-01-14T09:00"),
        Some("2026-01-15T09:00"),
    )
    .unwrap();
    assert_eq!(custom.label, "2026-01-15");
    let dst = compute_window(
        chrono_tz::America::New_York,
        now,
        false,
        true,
        Some("2026-03-08".parse().unwrap()),
        None,
        None,
    )
    .unwrap();
    assert_eq!((dst.end - dst.start).num_hours(), 23);
}

#[test]
fn paper_signals_localize_and_topic_scoring_only_counts_matching_preference() {
    let front = paper(
        "2601.00001",
        "Orion-3 Technical Report: Open-Weights Release",
    );
    assert_eq!(
        paper_signals(&front, &BTreeMap::new(), Lang::En),
        ["keyword:technical report/open weights / model family"]
    );
    assert_eq!(
        paper_signals(&front, &BTreeMap::new(), Lang::ZhCn),
        ["词:技术报告/开源权重/模型族"]
    );
    let mut routine = paper("2601.00002", "A routine synthetic study");
    assert!(paper_signals(&routine, &BTreeMap::new(), Lang::En).is_empty());
    routine.summary = "An inference latency evaluation for agents, with release metadata.".into();
    score_topics(
        &mut routine,
        &BTreeMap::from([("agents".into(), 2), ("inference".into(), 3)]),
    );
    assert_eq!(routine.pref, 5);
    assert!(!routine.topics.contains(&"product".into()));
}

#[test]
fn rebuild_and_reuse_preserve_legacy_metadata_and_fold_artifacts() {
    let work = tempdir().unwrap();
    let root = work.path();
    let dir = root.join("runs/2026-01-15/fetch");
    rows(&dir, &[paper("2601.01234", "A routine synthetic study")]);
    atomic_write_text(&dir.join("report.json"), &json!({
        "window_start":"2026-01-15T00:00+00:00","window_end":"2026-01-16T00:00+00:00","generated_at":"2026-01-16T00:15+00:00",
        "total_raw":1,"total_dedup":1,"sources":[{"id":"arxiv","status":"ok"}],"notes":["sample note"]
    }).to_string()).unwrap();
    let d = dir.to_str().unwrap();
    fetch(root, &["--rebuild", d, "--lang", "en"]);
    assert!(dir.join("triage-papers-rest.md").is_file());
    assert!(!fs::read_to_string(dir.join("triage-papers.md"))
        .unwrap()
        .contains("routine synthetic"));
    fetch(root, &["--rebuild", d, "--no-fold"]);
    assert!(!dir.join("triage-papers-rest.md").is_file());
    assert!(fs::read_to_string(dir.join("triage-papers.md"))
        .unwrap()
        .contains("routine synthetic"));
    let out = fetch(
        root,
        &[
            "--daily",
            "--day",
            "2026-01-15",
            "--reuse",
            "--json",
            "--registry",
            "missing.toml",
        ],
    );
    let summary: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(summary["reused"], true);
    assert_eq!(summary["sources"], 1);
    assert_eq!(summary["raw"], 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("already fetched this window"));
}

#[test]
fn recall_state_is_isolated_archived_and_nonfatal_source_errors_are_committed_last() {
    let work = tempdir().unwrap();
    let root = work.path();
    let registry = r#"
[settings]
timeout = 1
workers = 2
[[source]]
id = "blocked"
name = "Blocked fixture"
type = "rss"
channel = "lab"
kind = "lab_post"
url = "http://127.0.0.1/feed"
"#;
    fs::write(root.join("fixture.toml"), registry).unwrap();
    atomic_write_text(
        &root.join("state/triage-seen.json"),
        r#"{"stale":"2025-01-15","fresh":"2026-01-14"}"#,
    )
    .unwrap();
    fs::write(root.join("ledger.json"), json!({"items":[]}).to_string()).unwrap();
    let args = [
        "--daily",
        "--day",
        "2026-01-15",
        "--registry",
        "fixture.toml",
        "--json",
    ];
    let first = fetch(root, &args);
    let summary: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(summary["errors"][0]["id"], "blocked");
    assert!(summary["errors"][0]["status"]
        .as_str()
        .unwrap()
        .contains("non-public"));
    assert_eq!(summary["dedup"], 0);
    assert!(root.join("state/state.json").is_file());
    assert_eq!(
        fs::read_to_string(root.join("state/triage-seen.json")).unwrap(),
        r#"{"fresh":"2026-01-14"}"#
    );
    let day = root.join("runs/2026-01-15");
    assert!(!day.join("fetch/report.json.partial").exists());
    fetch(root, &args);
    assert!(fs::read_dir(&day)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().starts_with("fetch-prev-")));
    fetch(
        root,
        &[
            "--daily",
            "--day",
            "2026-01-15",
            "--registry",
            "fixture.toml",
            "--only",
            "blocked",
            "--no-state",
        ],
    );
    assert!(day.join("fetch-extra/report.json").is_file());
    let another = tempdir().unwrap();
    fs::write(another.path().join("fixture.toml"), registry).unwrap();
    fetch(
        another.path(),
        &[
            "--daily",
            "--day",
            "2026-01-15",
            "--registry",
            "fixture.toml",
            "--skip",
            "blocked",
            "--no-state",
        ],
    );
    assert!(!another.path().join("state").exists());
    assert!(!another.path().join("ledger.json").exists());
}

#[tokio::test]
async fn github_api_uses_scoped_bearer_and_reports_rate_limit_once() {
    let server = MockServer::start().await;
    let credential = ["fixture", "credential"].join("-");
    let auth = format!("Bearer {credential}");
    Mock::given(method("GET")).and(path("/orgs/example/repos"))
        .and(header("authorization", auth.as_str()))
        .and(header("accept", "application/vnd.github+json"))
        .and(header("x-github-api-version", "2022-11-28"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "created_at":"2026-01-15T08:00:00Z","full_name":"example/test","html_url":"https://github.com/example/test"
        }]))).expect(1).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/orgs/delayed/repos"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .set_body_string("API rate limit exceeded"),
        )
        .expect(2)
        .mount(&server)
        .await;
    let (settings, mut srcs) = parse_registry(
        r#"
[[source]]
id = "github"
name = "Fixture GitHub org"
type = "github_org_repos"
channel = "repo"
kind = "repo"
orgs = ["example"]
"#,
    )
    .unwrap();
    let now = at("2026-01-16T00:30:00Z");
    let window = compute_window(chrono_tz::UTC, now, false, true, None, None, None).unwrap();
    let mut context = SourceContext::new(window, &settings, Lang::En, FetchState::default())
        .with_github_token(Some(credential));
    context.http = context.http.with_fixture_origin(*server.address()).unwrap();
    let source = &mut srcs[0];
    assert_eq!(
        fetch_source(&context, source).await.unwrap()[0].title,
        "example/test"
    );
    source.orgs = vec!["delayed".into()];
    for _ in 0..2 {
        assert!(fetch_source(&context, source)
            .await
            .unwrap_err()
            .to_string()
            .contains("403 (rate limit"));
    }
    let notes = context.notes.lock().unwrap();
    assert_eq!(
        notes
            .iter()
            .filter(|s| s.contains("even with token"))
            .count(),
        1
    );
}

#[tokio::test]
async fn page_extraction_readme_fallback_batch_order_and_ssrf() {
    let html = format!("<html><head><title>Example Article</title><meta property=\"article:published_time\" content=\"2026-01-15\"></head><body><nav><p>private navigation fixture</p></nav><article><h1>Benchmarks</h1><p>{}</p></article></body></html>", "Synthetic benchmark gain of 61.2 on ExampleBench. ".repeat(10));
    let extracted = page_text(&html, 300, Some("ExampleBench"), 40, false, Lang::En);
    assert!(
        extracted.contains("Example Article")
            && extracted.contains("61.2")
            && !extracted.contains("private navigation")
    );
    assert!(page_text("", 20, None, 10, false, Lang::En).contains("JS-rendered"));
    let server = MockServer::start().await;
    Mock::given(path("/post"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(30))
                .set_body_string(html),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/example/repo/HEAD/README.md"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/example/repo/main/README.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string("# Project\nFast serving engine"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/abs/2601.01234")).respond_with(ResponseTemplate::new(200).set_body_string(r#"<title>Research</title><div class="dateline">15 January</div><div class="authors"><a>A. Author</a></div><blockquote class="abstract">Abstract: Compact result.</blockquote>"#)).expect(1).mount(&server).await;
    Mock::given(path("/start"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "http://169.254.169.254/must-not-fetch"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let http = SafeHttpClient::default()
        .with_fixture_origin(*server.address())
        .unwrap();
    let args = PageArgs {
        urls: [
            "https://example.com/post@@ExampleBench",
            "https://github.com/example/repo@@serving",
            "http://arxiv.org/pdf/2601.01234v2",
            "https://example.com/start",
        ]
        .map(String::from)
        .to_vec(),
        chars: 3000,
        find: None,
        ctx: 15,
        meta: false,
        strict: false,
        lang: None,
    };
    let (ok, out) = pages_with_client(&args, Lang::En, &http).await;
    assert!(ok);
    assert!(
        out.find("Example Article") < out.find("example/repo README")
            && out.contains("A. Author")
            && out.contains("Compact result.")
    );
    assert!(out.contains("--- serving: 1 hit(s)") && out.contains("use WebFetch instead"));
    for input in [
        "http://127.0.0.1/a",
        "http://10.0.0.1/a",
        "http://169.254.169.254/a",
        "http://[::ffff:127.0.0.1]/a",
        "file:///etc/services",
    ] {
        let args = PageArgs {
            urls: vec![input.into()],
            find: None,
            lang: None,
            ..args
        };
        let (ok, text) = pages_with_client(&args, Lang::En, &http).await;
        assert!(!ok && text.contains("ERROR"), "{input}: {text}");
    }
    assert!(!server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|request| request.url.path() == "/must-not-fetch"));
}

#[tokio::test]
async fn feed_html_arxiv_and_hf_daily_keep_window_and_state() {
    let server = MockServer::start().await;
    let atom = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><entry><title>AI Research</title><link rel="self" href="/feed"/><link href="/story"/><author><name>Example Author</name></author><published>2026-01-15T04:00:00Z</published><summary type="html">&lt;b&gt;Synthetic&lt;/b&gt; &amp; test.</summary></entry><entry><title>Outside</title><link href="/excluded"/><published>2026-01-16T00:00:00Z</published></entry></feed>"#;
    Mock::given(path("/feed"))
        .respond_with(ResponseTemplate::new(200).set_body_string(atom))
        .expect(1)
        .mount(&server)
        .await;
    let rss = r#"<rss version="2.0"><channel><item><title>AI tooling</title><link>https://example.com/rss-post</link><description><![CDATA[<p>Good &amp; clear.</p>]]></description><pubDate>Thu, 15 Jan 2026 11:00:00 GMT</pubDate></item></channel></rss>"#;
    Mock::given(path("/rss"))
        .respond_with(ResponseTemplate::new(200).set_body_string(rss))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/links")).respond_with(ResponseTemplate::new(200).set_body_string(r#"<a href="/articles/first">First</a><a href="/articles/2026-01-15/second">Second</a><a href="/articles/skip">Exclude</a>"#)).expect(1).mount(&server).await;
    Mock::given(path("/articles/first")).respond_with(ResponseTemplate::new(200).set_body_string(r#"<title>fixture</title><meta property="og:title" content="Resolved Title"><meta itemprop="datePublished" content="2026-01-15T08:00:00Z">"#)).expect(1).mount(&server).await;
    Mock::given(path("/articles/2026-01-15/second"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<main>Fixture</main>"))
        .expect(1)
        .mount(&server)
        .await;
    let listing = r#"<h3>Thu, 15 Jan 2026</h3><dl id="articles"><dt><a href="/abs/2601.01234">arXiv:2601.01234</a></dt><dd><div class="list-title">Title: Example paper</div><div class="list-authors"><a>Example Author</a></div><div class="list-subjects">Subjects: Machine Learning</div></dd></dl>"#;
    Mock::given(path("/list/cs.LG/pastweek"))
        .respond_with(ResponseTemplate::new(200).set_body_string(listing))
        .expect(1)
        .mount(&server)
        .await;
    let newest = r#"<h3>New listings for Thursday, 15 January 2026</h3><dl id="articles"><h3>New submissions</h3><dt><a href="/abs/2601.01234">id</a></dt><dd><p class="mathjax">Synthetic abstract.</p></dd></dl>"#;
    Mock::given(path("/list/cs.LG/new"))
        .respond_with(ResponseTemplate::new(200).set_body_string(newest))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/daily_papers")).and(query_param("date", "2026-01-15")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{
       "paper":{"id":"2601.01234","title":"HF paper","ai_summary":"Synthetic summary","authors":[{"name":"Example Author"}],"organization":{"fullname":"Example Lab"}}
    }]))).expect(1).mount(&server).await;
    let (settings, specs) = parse_registry(
        r#"
[[source]]
id = "atom"
name = "Atom fixture"
type = "rss"
channel = "lab"
kind = "lab_post"
url = "https://example.com/feed"
[[source]]
id = "rss"
name = "RSS fixture"
type = "rss"
channel = "tooling"
kind = "release"
url = "https://example.com/rss"
[[source]]
id = "links"
name = "Links fixture"
type = "html_links"
channel = "news"
kind = "news"
url = "https://example.com/links"
pattern = "^/articles/"
exclude = "/skip$"
[[source]]
id = "arxiv"
name = "arXiv fixture"
type = "arxiv"
channel = "paper"
kind = "paper"
categories = ["cs.LG"]
[[source]]
id = "hf_daily"
name = "HF fixture"
type = "hf_daily"
channel = "paper"
kind = "paper"
url = "https://huggingface.co/api/daily_papers"
"#,
    )
    .unwrap();
    let now = at("2026-01-16T03:00:00Z");
    let window = compute_window(chrono_tz::UTC, now, false, true, None, None, None).unwrap();
    let mut state = FetchState::default();
    state.0.insert("hf_daily:last".into(), json!("2026-01-14"));
    let mut context =
        SourceContext::new(window, &settings, Lang::En, state).with_github_token(None);
    context.now = now;
    context.http = context.http.with_fixture_origin(*server.address()).unwrap();
    let (atom, rss, links, arxiv, hf) = tokio::join!(
        fetch_source(&context, &specs[0]),
        fetch_source(&context, &specs[1]),
        fetch_source(&context, &specs[2]),
        fetch_source(&context, &specs[3]),
        fetch_source(&context, &specs[4])
    );
    let atom = atom.unwrap();
    assert_eq!(atom.len(), 1);
    assert_eq!(atom[0].url, "https://example.com/story");
    assert_eq!(atom[0].authors, ["Example Author"]);
    assert_eq!(atom[0].summary, "Synthetic & test.");
    assert_eq!(rss.unwrap()[0].summary, "Good & clear.");
    let links = links.unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].title, "Resolved Title");
    assert_eq!(links[0].extra["baseline_run"], true);
    let arxiv = arxiv.unwrap();
    assert_eq!(arxiv[0].title, "Example paper");
    assert_eq!(arxiv[0].summary, "Synthetic abstract.");
    assert_eq!(arxiv[0].extra["listing"], "2026-01-15");
    let hf = hf.unwrap();
    assert_eq!(hf[0].extra["org"], "Example Lab");
    let state = &context.state.lock().unwrap().0;
    assert_eq!(state["html:links"]["https://example.com/articles/first"], 0);
    assert!(state["html:links"]
        .get("https://example.com/articles/skip")
        .is_none());
    assert_eq!(state["arxiv:ingested"], json!(["2026-01-15"]));
    assert_eq!(state["hf_daily:last"], "2026-01-15");
}

#[tokio::test]
async fn remaining_adapters_apply_filters_and_discovery_metadata() {
    let server = MockServer::start().await;
    let when = "2026-01-15T10:00:00Z";
    let model = json!({"id":"example/core","createdAt":when,"pipeline_tag":"text-generation","tags":["region:test","agents"]});
    let quant = json!({"id":"example/core-gguf","createdAt":when});
    Mock::given(path("/api/models"))
        .and(query_param("author", "example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([model, quant])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/trending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([model, quant])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/trending")).respond_with(ResponseTemplate::new(200).set_body_string(r#"<article class="Box-row"><h2><a href="/example/trending">Project</a></h2><p>A new research project</p><i itemprop="programmingLanguage">Rust</i></article>"#)).expect(1).mount(&server).await;
    Mock::given(path("/repos/example/release/releases")).respond_with(ResponseTemplate::new(200).set_body_json(json!([
        {"name":"Preview","tag_name":"v2.0-beta","published_at":when,"prerelease":true},
        {"tag_name":"v1.3.0","published_at":when,"html_url":"https://github.com/example/release/releases/tag/v1.3.0"},
        {"tag_name":"v1.2.0","published_at":"2026-01-15T09:00:00Z"}
    ]))).expect(1).mount(&server).await;
    Mock::given(path("/search/repositories")).and(query_param("q","agents created:>=2026-01-13")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
         "items":[{"full_name":"example/repo","html_url":"https://github.com/example/repo","description":"AI agents","created_at":when,"language":"Rust","topics":["agents"]},
                  {"full_name":"example2026/spam","html_url":"https://github.com/example2026/spam","description":"AI agents"}]
    }))).expect(1).mount(&server).await;
    Mock::given(path("/api/hn")).and(query_param("page","0")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "nbPages":1, "hits":[{"created_at_i":at(when).timestamp(),"objectID":"17","title":"New AI agents","story_text":"Synthetic result"}]
    }))).expect(1).mount(&server).await;
    Mock::given(path("/r/example/.rss")).respond_with(ResponseTemplate::new(200).set_body_string(r#"<feed><entry><title>Sample thread</title><link href="https://example.com/thread"/><updated>2026-01-15T01:00:00Z</updated><content>Thread body submitted by /u/example</content></entry></feed>"#)).expect(1).mount(&server).await;
    let (settings, specs) = parse_registry(
        r#"
[[source]]
id = "org_model"
name = "Organizations"
type = "hf_org_models"
channel = "model"
kind = "model"
orgs = ["example"]
[[source]]
id = "hf_trending"
name = "HF trending"
type = "hf_trending"
channel = "model"
kind = "model"
url = "https://huggingface.co/api/trending"
[[source]]
id = "gh_trending"
name = "GitHub trending"
type = "github_trending"
channel = "repo"
kind = "repo"
urls = ["https://github.com/trending"]
[[source]]
id = "releases"
name = "Releases"
type = "github_releases"
channel = "tooling"
kind = "release"
repos = ["example/release"]
[[source]]
id = "search"
name = "Repository search"
type = "github_search"
channel = "repo"
kind = "repo"
queries = ["agents created:>={since3}"]
[[source]]
id = "hn"
name = "HN"
type = "hn"
channel = "community"
kind = "discussion"
url = "https://example.com/api/hn"
[[source]]
id = "reddit"
name = "Reddit"
type = "reddit"
channel = "community"
kind = "discussion"
url = "https://www.reddit.com/r/example/.rss"
"#,
    )
    .unwrap();
    let now = at("2026-01-16T03:00:00Z");
    let win = compute_window(chrono_tz::UTC, now, false, true, None, None, None).unwrap();
    let mut context =
        SourceContext::new(win, &settings, Lang::En, FetchState::default()).with_github_token(None);
    context.http = context.http.with_fixture_origin(*server.address()).unwrap();
    let found =
        futures_util::future::join_all(specs.iter().map(|s| fetch_source(&context, s))).await;
    let found: Vec<_> = found.into_iter().map(Result::unwrap).collect();
    assert!(found.iter().all(|items| items.len() == 1), "{found:?}");
    assert_eq!(found[0][0].extra["org"], "example");
    assert_eq!(found[1][0].extra["discovery_only"], true);
    assert_eq!(found[2][0].extra["language"], "Rust");
    assert_eq!(found[3][0].extra["releases_in_window"], 2);
    assert!(found[3][0].title.ends_with("v1.3.0"));
    assert_eq!(found[4][0].extra["topics"], json!(["agents"]));
    assert_eq!(
        found[5][0].extra["hn"],
        "https://news.ycombinator.com/item?id=17"
    );
    assert_eq!(found[6][0].summary, "Thread body");
}

#[test]
fn offline_lookup_validation_weekly_gap_and_xparse() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let first = root.join("runs/2026-01-13/fetch");
    let second = root.join("runs/2026-01-14/fetch-extra");
    let mut shortlist: Vec<_> = (0..10)
        .map(|i| paper(&format!("2601.{i:05}"), &format!("Synthetic study {i}")))
        .collect();
    shortlist[0].in_ledger = InLedger::Status("pushed".into());
    shortlist[0].id = "dist00".into();
    shortlist[0].summary = "First summary".into();
    shortlist[1].pref = 3;
    shortlist[1].summary = "Synthetic abstract detail for weekly.".into();
    rows(&first, &shortlist);
    let mut late = shortlist[1].clone();
    late.source = "hf_daily".into();
    late.extra.insert("org".into(), json!("Example Institute"));
    let mut duplicate = shortlist[0].clone();
    duplicate.summary = "Later summary".into();
    rows(&second, &[late, duplicate]);
    let gap = root.join("runs/2026-01-18-weekly/fetch");
    let lab = Cand {
        channel: "lab".into(),
        title: "Laboratory update mentions 2601.00001".into(),
        url: "https://example.com/lab".into(),
        source: "lab_fixture".into(),
        ..Cand::default()
    };
    rows(&gap, &[lab]);
    atomic_write_text(
        &gap.join("report.json"),
        r#"{"window_start":"2026-01-17T00:00+00:00","window_end":"2026-01-18T00:00+00:00"}"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(BIN)
            .args(args)
            .current_dir(root)
            .env("DIGEST_ROOT", root)
            .env("DIGEST_TZ", "Asia/Taipei")
            .env("DIGEST_LANG", "en")
            .env_remove("LEDGER_PATH")
            .output()
            .unwrap()
    };
    let out = run(&["cand", "Dist00", "--since", "2026-01-13"]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Later summary") && !text.contains("First summary"));
    let draft = root.join("runs/2026-01-13/paper-cands.md.tmp");
    let mut markdown = "## Strong\n".to_owned();
    for i in 0..10 {
        if i == 4 {
            markdown.push_str("## Maybe\n");
        }
        markdown.push_str(&format!("- 2601.{i:05} | Study {i}\n"));
    }
    fs::write(&draft, markdown).unwrap();
    let out = run(&[
        "check-cands",
        draft.to_str().unwrap(),
        "--finalize",
        "--json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let check: Value = serde_json::from_slice(&out.stdout).unwrap();
    for (key, val) in [
        ("ok", json!(true)),
        ("n", json!(10)),
        ("strong", json!(4)),
        ("weak", json!(6)),
        ("pushed", json!(1)),
        ("unknown", json!([])),
        ("duplicates", json!([])),
        ("problems", json!([])),
    ] {
        assert_eq!(check[key], val);
    }
    assert!(!draft.exists());
    assert!(Path::new(check["final"].as_str().unwrap()).is_file());
    let out = run(&["weekly-late", "--day", "2026-01-15", "--chars", "25"]);
    let out = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "gap-fill",
        "HF late 01-14",
        "Synthetic abstract detail",
        "lab_fixture",
        "mentioned:",
    ] {
        assert!(out.contains(expected), "{out}");
    }
    let raw = "log noise { \"meta\": wrong }\n{\"data\":[{\"id\":101,\"author_id\":1,\"created_at\":\"2026-01-15T00:00Z\",\"text\":\"This synthetic release announcement contains a useful signal https://example.com\"},{\"id\":102,\"text\":\"https://example.com/short\"},{\"id\":103,\"text\":\"RT @example This message has too much filler to be useful.\"},{\"id\":104,\"text\":\"This synthetic release announcement contains a useful signal\"}],\"includes\":{\"users\":[{\"id\":1,\"username\":\"examplelabs\"}]},\"meta\":{\"next_token\":\"more\"}} remainder";
    assert_eq!(x_json(raw).unwrap()["data"].as_array().unwrap().len(), 4);
    fs::write(root.join("dump.txt"), raw).unwrap();
    let out = run(&[
        "xparse",
        "A=dump.txt",
        "B=dump.txt",
        "--out",
        "runs/x-raw.md",
    ]);
    assert!(out.status.success());
    let body = fs::read_to_string(root.join("runs/x-raw.md")).unwrap();
    assert!(
        body.contains("A=4 B=4")
            && body.contains("truncated")
            && body.contains("01-15 08:00 x.com/examplelabs/status/101")
    );
    assert_eq!(body.matches("/status/").count(), 1);
}

#[test]
fn extreme_window_boundaries_return_usage_error_instead_of_panicking() {
    let now = at("2026-01-16T03:00:00Z");
    let east: chrono_tz::Tz = "Asia/Shanghai".parse().unwrap();
    let west: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();

    // Extreme RFC3339 boundaries that previously could overflow when converted to local timezone
    assert!(compute_window(
        west,
        now,
        false,
        false,
        None,
        Some("-262143-01-01T00:00:00Z"),
        Some("-262143-01-01T02:00:00Z"),
    )
    .is_err());
    assert!(compute_window(
        east,
        now,
        false,
        false,
        None,
        Some("+262142-12-31T20:00:00Z"),
        Some("+262142-12-31T23:59:59Z"),
    )
    .is_err());
    assert!(compute_window(
        east,
        now,
        false,
        true,
        Some(chrono::NaiveDate::MIN),
        None,
        None
    )
    .is_err());
    assert!(compute_window(
        west,
        now,
        false,
        true,
        Some(chrono::NaiveDate::MAX),
        None,
        None
    )
    .is_err());

    // CLI surfaces boundary overflow as exit code 2 (usage error), never a panic (101)
    let tmp = tempdir().unwrap();
    let out = Command::new(BIN)
        .args([
            "fetch",
            "--start",
            "+262142-12-31T20:00:00Z",
            "--end",
            "+262142-12-31T23:59:59Z",
        ])
        .current_dir(tmp.path())
        .env("DIGEST_ROOT", tmp.path())
        .env("DIGEST_TZ", "Asia/Shanghai")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn sec1_and_sec2_page_strict_check_cands_malformed_and_hf_summary_priority() {
    let tmp = tempdir().unwrap();

    // SEC-1: page --strict fails closed with non-zero exit when any URL is unreachable/blocked
    let strict_out = Command::new(BIN)
        .args(["page", "--strict", "http://127.0.0.1:9/blocked"])
        .current_dir(tmp.path())
        .env("DIGEST_ROOT", tmp.path())
        .output()
        .unwrap();
    assert!(
        !strict_out.status.success(),
        "page --strict must exit non-zero on failure"
    );

    // SEC-1: check-cands rejects malformed freeform lines
    let run_dir = tmp.path().join("runs/2026-01-15/fetch");
    fs::create_dir_all(&run_dir).unwrap();
    rows(&run_dir, &[paper("2601.01234", "Valid Paper")]);
    let pick_file = tmp.path().join("picks.txt");
    atomic_write_text(
        &pick_file,
        "abc123def456\nnot a valid candidate line at all\n",
    )
    .unwrap();
    let check_out = Command::new(BIN)
        .args([
            "fetch",
            "check-cands",
            "--date",
            "2026-01-15",
            pick_file.to_str().unwrap(),
        ])
        .current_dir(tmp.path())
        .env("DIGEST_ROOT", tmp.path())
        .env("DIGEST_TZ", "UTC")
        .output()
        .unwrap();
    assert!(
        !check_out.status.success(),
        "check-cands must reject malformed freeform lines"
    );

    // SEC-2: hf_daily prefers author summary over ai_summary
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mock = MockServer::start().await;
        Mock::given(path("/api/daily_papers"))
            .and(query_param("date", "2026-01-15"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "paper": {
                    "id": "2601.09999",
                    "title": "Author Abstract Priority",
                    "summary": "Official author abstract",
                    "ai_summary": "Untrusted synthetic summary",
                    "authors": [{"name": "Alice"}],
                    "organization": {"fullname": "Lab"}
                }
            }])))
            .mount(&mock)
            .await;
        let (settings, specs) = parse_registry(
            r#"
[[source]]
id = "hf_daily"
name = "HF fixture"
type = "hf_daily"
channel = "paper"
kind = "paper"
url = "https://huggingface.co/api/daily_papers"
"#,
        )
        .unwrap();
        let now = at("2026-01-16T03:00:00Z");
        let window = compute_window(chrono_tz::UTC, now, false, true, None, None, None).unwrap();
        let mut state = FetchState::default();
        state.0.insert("hf_daily:last".into(), json!("2026-01-14"));
        let mut ctx =
            SourceContext::new(window, &settings, Lang::En, state).with_github_token(None);
        ctx.now = now;
        ctx.http = ctx.http.with_fixture_origin(*mock.address()).unwrap();
        let cands = fetch_source(&ctx, &specs[0]).await.unwrap();
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].summary, "Official author abstract");
    });
}
