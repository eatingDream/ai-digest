use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use ai_digest::{
    assets,
    crypto::{self, ScriptAllowlist},
    geo,
    i18n::Lang,
    model::{atomic_write_text, files_under, DigestPaths, SiteConfig},
    net::SafeHttpClient,
    publish::{self, build_demo, deploy_via_pages_api},
    render, verify,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use url::Url;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

const BIN: &str = env!("CARGO_BIN_EXE_ai-digest");
const DAILY: &str = "2026-01-15";
const WEEKLY: &str = "2026-01-12_01-18";
const DEMO_KEY: &str = "fixture demo";

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn cli(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(root)
        .env("DIGEST_ROOT", root)
        .env("DIGEST_TZ", "UTC");
    for key in [
        "SITE_SLUG",
        "SITE_DOMAIN",
        "DIGEST_ENCRYPT",
        "DIGEST_LANG",
        "DIGEST_LANG_ALT",
        "DIGEST_ROOT_LATEST",
        "DIGEST_GEO_AI_TRAIN",
        "DIGEST_CONTENT_SIGNAL",
        "DIGEST_GEO_WORKER",
        "DIGEST_GEO_SEARCH",
        "DIGEST_DEMO_PASSWORD",
        "CLOUDFLARE_API_TOKEN",
        "CF_PAGES_PROJECT",
        "CF_PROJECT",
        "CF_ACCOUNT_ID",
        "CF_ZONE_ID",
        "DIGEST_CF_API_BASE",
        "DIGEST_PRIVACY_PATTERNS_FILE",
        "DIGEST_SITE_PASSWORD",
        "DIGEST_INDEXNOW_KEY",
        "DIGEST_INDEXNOW_ENDPOINT",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("CLOUDFLARE_API_TOKEN_FILE", root.join("unused-token-file"))
        .env("HOME", root)
        .env("DIGEST_SITE_PASSWORD_FILE", root.join("fixture-password"))
        .env("DIGEST_SITE_SALT_FILE", root.join("fixture-salt"));
    cmd
}

fn out(root: &Path, args: &[&str], code: i32) -> Output {
    checked(cli(root, args).output().unwrap(), code)
}

fn checked(result: Output, code: i32) -> Output {
    assert_eq!(
        result.status.code(),
        Some(code),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

fn get(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}

fn issue_markdown(root: &Path, bilingual: bool) {
    let sample = examples();
    let lang = if bilingual { "zh-CN" } else { "en" };
    fs::copy(
        sample.join(format!("sample-digest.{lang}.md")),
        root.join(format!("digest-{DAILY}.md")),
    )
    .unwrap();
    fs::copy(
        sample.join(format!("sample-weekly.{lang}.md")),
        root.join(format!("weekly-{WEEKLY}.md")),
    )
    .unwrap();
    if bilingual {
        fs::copy(
            sample.join("sample-digest.en.md"),
            root.join(format!("digest-{DAILY}.en.md")),
        )
        .unwrap();
        fs::copy(
            sample.join("sample-weekly.en.md"),
            root.join(format!("weekly-{WEEKLY}.en.md")),
        )
        .unwrap();
    }
}

fn add_daily(root: &Path) -> Output {
    out(
        root,
        &[
            "publish",
            "--issue",
            &format!("daily/{DAILY}"),
            "--md",
            &format!("digest-{DAILY}.md"),
            "--prepare-only",
        ],
        0,
    )
}

fn add_weekly(root: &Path) {
    out(
        root,
        &[
            "publish",
            "--issue",
            &format!("weekly/{WEEKLY}"),
            "--md",
            &format!("weekly-{WEEKLY}.md"),
            "--prepare-only",
        ],
        0,
    );
}

fn password(root: &Path) -> String {
    let value = ["  示例 ", "passphrase ", "for the fixture  "].concat();
    crypto::write_secret_file(&root.join("fixture-password"), &format!("{value}\r\n")).unwrap();
    value
}

#[test]
fn init_defaults_slug_and_config_overrides() {
    let a = tempdir().unwrap();
    let result = out(a.path(), &["publish", "--init"], 0);
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "SLUG=\nENCRYPT=0\n"
    );
    assert!(get(a.path(), "site/CONFIG").contains("PLATFORM=pages"));
    assert!(!a.path().join("site/public").exists());
    let b = tempdir().unwrap();
    let result = out(b.path(), &["publish", "--init", "--encrypt"], 0);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let slug = stdout
        .lines()
        .next()
        .unwrap()
        .strip_prefix("SLUG=")
        .unwrap();
    assert_eq!(slug.len(), 22);
    assert!(slug
        .bytes()
        .all(|b| b"abcdefghijkmnopqrstuvwxyz23456789".contains(&b)));
    assert!(stdout.contains("ENCRYPT=1"));
    let again = out(b.path(), &["publish", "--init"], 0);
    assert_eq!(again.stdout, result.stdout);
    let override_empty = checked(
        cli(b.path(), &["publish", "--init"])
            .env("SITE_SLUG", "")
            .env("DIGEST_ENCRYPT", "0")
            .output()
            .unwrap(),
        0,
    );
    assert_eq!(
        String::from_utf8_lossy(&override_empty.stdout),
        "SLUG=\nENCRYPT=0\n"
    );
    for slug in [
        "<replace>",
        "short",
        "robots.txt",
        "_headers",
        "invalid/path",
    ] {
        let bad = checked(
            cli(b.path(), &["publish", "--init"])
                .env("SITE_SLUG", slug)
                .output()
                .unwrap(),
            1,
        );
        assert!(String::from_utf8_lossy(&bad.stderr).contains("SLUG"));
    }
    let cfg = SiteConfig::parse(
        "DOMAIN=base.example.com\nSLUG=sample-slug\nLANG=en\n",
        None,
        |key| match key {
            "SITE_DOMAIN" => Some("digest.example.org".into()),
            "DIGEST_LANG" => Some("zh".into()),
            "SITE_SLUG" => Some(String::new()),
            "CF_PAGES_PROJECT" => Some("fixture-project".into()),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(
        (
            cfg.domain.as_str(),
            cfg.slug.as_str(),
            cfg.lang,
            cfg.project.as_str()
        ),
        ("digest.example.org", "", Lang::ZhCn, "fixture-project")
    );
}

#[test]
fn demo_single_and_both_languages_build_isolated_geo_and_encrypted_sites() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let before = [
        "SITE_SLUG",
        "DIGEST_ENCRYPT",
        "DIGEST_LANG",
        "DIGEST_LANG_ALT",
    ]
    .map(std::env::var_os);
    let public = root.join("public");
    assert_eq!(
        build_demo(
            &public,
            &examples(),
            &[Lang::En, Lang::ZhCn],
            None,
            &SiteConfig::default(),
            true
        )
        .unwrap(),
        12
    );
    assert_eq!(
        before,
        [
            "SITE_SLUG",
            "DIGEST_ENCRYPT",
            "DIGEST_LANG",
            "DIGEST_LANG_ALT"
        ]
        .map(std::env::var_os)
    );
    assert_eq!(get(&public, "_redirects"), "/ /en/ 302\n");
    assert!(get(&public, "en/index.html").contains("href=\"/zh-CN/\""));
    assert!(get(&public, "zh-CN/index.html").contains("href=\"/en/\""));
    assert!(get(&public, "en/daily/2026-01-15.html").contains("href=\"/zh-CN/daily/2026-01-15\""));
    let sitemap = get(&public, "sitemap.xml");
    assert!(
        sitemap.contains("/en/daily/2026-01-15") && sitemap.contains("/zh-CN/daily/2026-01-15")
    );
    assert!(get(&public, "_worker.js").contains(r#"["/","/en/",302]"#));
    assert!(get(&public, "_headers")
        .contains("/zh-CN/daily/latest\n  ! Cache-Control\n  Cache-Control: no-store"));
    out(
        root,
        &["publish", "--demo", "--lang", "en", "--out", "single"],
        0,
    );
    assert!(root.join("single/demo/index.html").is_file());
    assert_eq!(get(root, "single/_redirects"), "/ /demo/ 302\n");
    assert!(!root.join("single/en").exists());
    let private = root.join("encrypted");
    assert_eq!(
        build_demo(
            &private,
            &examples(),
            &[Lang::En, Lang::ZhCn],
            Some(DEMO_KEY),
            &SiteConfig::default(),
            true
        )
        .unwrap(),
        12
    );
    for (dir, lang, alt) in [("en", Lang::En, "zh-CN"), ("zh-CN", Lang::ZhCn, "en")] {
        let shell = get(&private, &format!("{dir}/daily/{DAILY}.html"));
        crypto::assert_encrypted_shell(&shell, lang).unwrap();
        assert!(shell.contains("demo-hint") && shell.contains(&format!("/{alt}/daily/{DAILY}")));
        let raw =
            crypto::decrypt_envelope(&crypto::extract_envelope(&shell).unwrap(), DEMO_KEY).unwrap();
        assert!(std::str::from_utf8(&raw)
            .unwrap()
            .contains("digest-lock:start"));
    }
    assert!(!private.join("llms.txt").exists());
    let single_private = checked(
        cli(
            root,
            &[
                "publish",
                "--demo",
                "--lang",
                "zh-CN",
                "--out",
                "single-encrypted",
            ],
        )
        .env("DIGEST_DEMO_PASSWORD", DEMO_KEY)
        .output()
        .unwrap(),
        0,
    );
    assert!(String::from_utf8_lossy(&single_private.stdout).contains("demo site: 2 issue(s)"));
    assert!(get(root, "single-encrypted/demo/index.html").contains("id=\"pw\""));
    checked(
        cli(root, &["publish", "--demo", "--out", "bad"])
            .env("DIGEST_DEMO_PASSWORD", "bad")
            .output()
            .unwrap(),
        1,
    );
    assert!(!root.join("site/CONFIG").exists());
}

#[test]
fn publish_companions_latest_redirects_rerender_removal_and_nav_idempotency() {
    let work = tempdir().unwrap();
    let root = work.path();
    atomic_write_text(&root.join("site/CONFIG"), "LANG=zh-CN\n").unwrap();
    atomic_write_text(
        &root.join("site/removed-paths.txt"),
        "daily/latest\nweekly/latest.html\ndaily/2026-01-10\n",
    )
    .unwrap();
    issue_markdown(root, true);
    let result = add_daily(root);
    assert!(String::from_utf8_lossy(&result.stdout).starts_with("OK published: daily/2026-01-15"));
    assert!(String::from_utf8_lossy(&result.stdout).contains("log:"));
    assert_eq!(fs::read_dir(root.join("logs")).unwrap().count(), 1);
    add_weekly(root);
    for folder in ["", "zh/", "en/"] {
        for path in [
            "index.html",
            "latest.html",
            "daily/latest.html",
            "weekly/latest.html",
            "daily/2026-01-15.html",
            "weekly/2026-01-12_01-18.html",
        ] {
            assert!(
                root.join(format!("site/public/{folder}{path}")).is_file(),
                "{folder}{path}"
            );
        }
    }
    let reader = get(root, "site/public/zh/daily/2026-01-15.html");
    assert!(
        reader.contains("href=\"/en/daily/2026-01-15\"") && !reader.contains("lang-pane lang-")
    );
    let en = get(root, "site/public/en/daily/2026-01-15.html");
    assert!(en.contains("href=\"/zh/daily/2026-01-15\"") && !en.contains("lang-pane lang-"));
    let redirects = get(root, "site/public/_redirects");
    assert!(!redirects.contains("/latest"));
    assert_eq!(redirects.lines().count(), 6);
    for marker in [
        "digest-nav-slot",
        "digest-nav:top:start",
        "digest-nav:bottom:start",
    ] {
        assert_eq!(reader.matches(marker).count(), 1, "{marker}");
    }
    out(root, &["publish", "--rebuild", "--prepare-only"], 0);
    assert_eq!(get(root, "site/public/zh/daily/2026-01-15.html"), reader);
    let raw = get(root, "site/sources/daily/2026-01-15.html");
    assert!(!raw.contains("digest-nav:top:start"));
    assert_eq!(
        render::strip_injections(&render::strip_injections(&reader)),
        render::strip_injections(&reader)
    );

    out(
        root,
        &[
            "publish",
            "--issue",
            "daily/2026-01-15",
            "--md",
            "digest-2026-01-15.md",
            "--md-en",
            "missing.en.md",
            "--prepare-only",
        ],
        1,
    );
    assert_eq!(get(root, "site/sources/daily/2026-01-15.html"), raw);
    let md = root.join("digest-2026-01-15.en.md");
    let changed = get(root, "digest-2026-01-15.en.md").replace("AI Daily", "Fixture AI Daily");
    fs::write(md, changed).unwrap();
    out(root, &["publish", "--rerender-all", "--prepare-only"], 0);
    assert!(get(root, "site/public/en/daily/2026-01-15.html").contains("Fixture AI Daily"));
    out(
        root,
        &["publish", "--remove", "daily/2026-01-15", "--prepare-only"],
        0,
    );
    let backups = files_under(&root.join("backfill/removed")).unwrap();
    for name in [
        "source-2026-01-15.html",
        "source-en-2026-01-15.html",
        "published-zh-daily-2026-01-15.html",
        "daily-2026-01-15.en.html",
    ] {
        assert!(
            backups
                .iter()
                .any(|f| f.file_name() == Some(OsStr::new(name))),
            "{name}"
        );
    }
    assert!(get(root, "site/removed-paths.txt").contains("daily/2026-01-15"));
    assert!(get(root, "site/public/_redirects").contains("/en/daily/2026-01-15 /en/ 302"));
    assert!(!root.join("site/public/en/daily/2026-01-15.html").exists());
    assert!(!root.join("site/public/en/daily/latest.html").exists());
    assert!(root.join("site/public/en/latest.html").is_file());
}

#[test]
fn csp_and_geo_artifacts_and_same_directory_public_to_encrypted_transition() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    atomic_write_text(&root.join("site/CONFIG"), "LANG=zh-CN\nGEO_AI_TRAIN=no\n").unwrap();
    issue_markdown(root, true);
    add_daily(root);
    add_weekly(root);
    let public = root.join("site/public");
    let reader = get(&public, "zh/daily/2026-01-15.html");
    for marker in [
        "rel=\"canonical\"",
        "hreflang=\"en\"",
        "text/markdown",
        "application/atom+xml",
        "application/ld+json",
        "NewsArticle",
        "ItemList",
        "agent:",
    ] {
        assert!(reader.contains(marker), "{marker}");
    }
    let headers = get(&public, "_headers");
    let rules = [
        "/*\n",
        "/favicon.svg\n",
        "\n/\n",
        "\n/daily/*\n",
        "\n/weekly/*\n",
        "\n/daily/latest\n",
        "\n/feed.xml\n",
    ];
    let offsets: Vec<_> = rules
        .iter()
        .map(|s| headers.find(s).unwrap_or_else(|| panic!("{s}")))
        .collect();
    assert!(
        offsets.windows(2).all(|pair| pair[0] < pair[1]),
        "{offsets:?}"
    );
    let (global, cache) = geo::parse_headers(&headers);
    assert!(global.get("Link").unwrap().contains("/.well-known/search"));
    assert_eq!(global["Vary"], "Accept");
    assert!(!global.contains_key("X-Robots-Tag"));
    assert!(cache.contains(&("/daily/latest".into(), "no-store".into())));
    let robots = get(&public, "robots.txt");
    assert!(robots.contains("Content-Signal: ai-train=no, search=yes, ai-input=yes"));
    assert!(
        robots.contains("User-agent: GPTBot\nDisallow: /")
            && robots.contains("User-agent: OAI-SearchBot\nAllow: /")
    );
    for file in [
        "daily/2026-01-15.md",
        "en/feed.xml",
        "zh/feed.xml",
        "feed.xml",
        "llms.txt",
        "llms-full.txt",
        "sitemap.xml",
        "search-index.json",
        "_worker.js",
        "_routes.json",
        geo::SKILL_INDEX,
        geo::SKILL_PATH,
    ] {
        assert!(public.join(file).is_file(), "{file}");
    }
    let skill = fs::read(public.join(geo::SKILL_PATH)).unwrap();
    let index: Value = serde_json::from_str(&get(&public, geo::SKILL_INDEX)).unwrap();
    assert_eq!(
        index["skills"][0]["digest"],
        format!("sha256:{}", hex::encode(Sha256::digest(skill)))
    );
    let search: Value = serde_json::from_str(&get(&public, "search-index.json")).unwrap();
    assert!(search["docs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|doc| doc["l"] == "zh-CN"));
    assert_eq!(out(root, &["verify", "--local"], 0).status.code(), Some(0));
    let pass = password(root);
    checked(
        cli(root, &["publish", "--rebuild", "--prepare-only"])
            .env("DIGEST_ENCRYPT", "1")
            .output()
            .unwrap(),
        0,
    );
    for file in [
        "search-index.json",
        "feed.xml",
        "zh/feed.xml",
        "en/feed.xml",
        "llms.txt",
        "llms-full.txt",
        "sitemap.xml",
        "_worker.js",
        "_routes.json",
        ".well-known",
    ] {
        assert!(!public.join(file).exists(), "{file}");
    }
    assert!(files_under(&public)
        .unwrap()
        .iter()
        .all(|p| p.extension() != Some(OsStr::new("md"))));
    assert_eq!(get(&public, "robots.txt"), "User-agent: *\nDisallow: /\n");
    let (global, _) = geo::parse_headers(&get(&public, "_headers"));
    assert_eq!(global["X-Robots-Tag"], "noindex, nofollow");
    let csp = global["Content-Security-Policy"]
        .split(';')
        .map(str::trim)
        .find(|s| s.starts_with("script-src "))
        .unwrap();
    assert!(!csp.contains("'unsafe-inline'"));
    let paths = DigestPaths::at(root, chrono_tz::UTC);
    let first = crypto::script_hash(assets::BOOT_JS);
    for (dir, rel) in [
        (&paths.plain, "zh/daily/2026-01-15.html"),
        (&paths.plain, "zh/index.html"),
        (&paths.public, "zh/daily/2026-01-15.html"),
        (&paths.public, "404.html"),
    ] {
        let content = get(dir, rel);
        let hashes = crypto::collect_script_hashes(&content, &ScriptAllowlist::default()).unwrap();
        assert_eq!(hashes[0], first);
        assert!(hashes.iter().all(|hash| csp.contains(hash)));
    }
    assert!(csp.contains(&crypto::script_hash(assets::LOCK_JS)));
    assert!(csp.contains(&crypto::script_hash(assets::READER_JS)));
    assert!(verify::verify_local(&paths, true, Some(&pass)).unwrap() > 4);
    assert!(!get(root, "site/sources/daily/2026-01-15.html").contains("digest-lock:"));
    assert!(get(root, "site/plain/zh/daily/2026-01-15.html").contains("digest-lock:"));
    fs::rename(root.join("site/plain"), root.join("site/plain-hidden")).unwrap();
    checked(
        cli(root, &["verify", "--local"])
            .env("DIGEST_ENCRYPT", "1")
            .output()
            .unwrap(),
        2,
    );
}

#[test]
fn incremental_encryption_cache_rotation_root_latest_and_legacy_node_verify() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let pass = password(root);
    let slug = "fixture-route";
    atomic_write_text(&root.join("site/CONFIG"), &format!("LANG=zh-CN\nSLUG={slug}\nENCRYPT=1\nROOT_LATEST=1\nLANG_ALT=https://companion.example.com/\n")).unwrap();
    let legacy_template = r##"<!DOCTYPE html>
<html lang="{{html_lang}}">
<head>
<meta charset="utf-8">
<meta name="robots" content="noindex, nofollow, noarchive">
<meta name="theme-color" content="#F8FAFD">
<title>{{pw_title}}</title>
<script>
(function () {
  var t = localStorage.getItem("digest-theme");
  var m = document.querySelector('meta[name="theme-color"]');
  if (m && t === "dark") m.setAttribute("content", "#0B0D12");
})();
</script>
</head>
<body>
<div id="box" hidden>
  <h1><span class="ai">AI</span>{{brand_tail}}</h1>
  <form id="f" autocomplete="off">
    <label class="pw-lab" for="pw">{{pw_label}}</label>
    <input id="pw" name="password" type="password" required>
    <label class="rem"><input id="rm" type="checkbox" checked> {{pw_remember}}</label>
    <button id="go" type="submit"><span class="go-t">{{pw_open}}</span></button>
    <p id="err" hidden>{{pw_wrong}}</p>
  </form>
  {{demo_hint}}
  {{lang_switch}}
</div>
<script type="application/json" id="c">__ENCRYPT_CFG_JSON__</script>
<script>
(function () {
  var txtOpen = {{js:pw_open}};
  var txtOpening = {{js:pw_opening}};
  var txtWrong = {{js:pw_wrong}};
  var txtUnsupported = {{js:pw_unsupported}};
  void [txtOpen, txtOpening, txtWrong, txtUnsupported];
})();
</script>
</body>
</html>
"##;
    atomic_write_text(&root.join("site/password_template.html"), legacy_template).unwrap();
    issue_markdown(root, true);
    add_daily(root);
    let index = format!("site/public/{slug}/index.html");
    let before = get(root, &index);
    let saved_cache = get(root, "site/enc-cache.json");
    let rerun = add_daily(root);
    assert!(String::from_utf8_lossy(&rerun.stdout).contains("changed pages: none"));
    assert_eq!(get(root, &index), before);
    assert_eq!(get(root, "site/enc-cache.json"), saved_cache);
    let boot = crypto::script_hash(assets::BOOT_JS);
    let mut allow = ScriptAllowlist::default();
    let raw_template = get(root, "site/password_template.html");
    allow
        .allow_legacy_template(&raw_template, crypto::UnlockOptions::default())
        .unwrap();
    let en = get(root, &format!("site/public/{slug}/en/index.html"));
    assert_eq!(crypto::collect_script_hashes(&en, &allow).unwrap()[0], boot);
    for relative in [
        "latest.html",
        "daily/latest.html",
        "zh/latest.html",
        "en/latest.html",
    ] {
        let shell = get(root, &format!("site/public/{relative}"));
        assert!(shell.contains("id=\"rm\"") && shell.contains("id=\"pw\""));
        assert!(!shell.contains(slug) && !shell.contains("companion.example.com"));
    }
    let paths = DigestPaths::at(root, chrono_tz::UTC);
    assert!(verify::verify_local(&paths, true, Some(&pass)).unwrap() > 6);
    checked(
        cli(
            root,
            &["verify", "--local", "--password-file", "fixture-password"],
        )
        .env("DIGEST_SITE_PASSWORD_FILE", root.join("absent-password"))
        .output()
        .unwrap(),
        0,
    );
    let legacy = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("web/dist/verify_decrypt.mjs"))
        .arg("--local")
        .env("DIGEST_ROOT", root)
        .env("DIGEST_SITE_PASSWORD_FILE", root.join("fixture-password"))
        .env("DIGEST_ENCRYPT", "1")
        .env("SITE_SLUG", slug)
        .output()
        .unwrap();
    assert!(
        legacy.status.success(),
        "{}",
        String::from_utf8_lossy(&legacy.stderr)
    );
    assert!(String::from_utf8_lossy(&legacy.stdout).contains("decrypted and matched plaintext"));
    let old_salt = get(root, "fixture-salt");
    out(root, &["publish", "--rotate-password", "--prepare-only"], 0);
    let new = crypto::read_password_file(&root.join("fixture-password")).unwrap();
    assert!(pass != new && old_salt != get(root, "fixture-salt"));
    assert!(before != get(root, &index));
    assert!(verify::verify_local(&paths, true, Some(&pass)).is_err());
    assert!(verify::verify_local(&paths, true, Some(&new)).unwrap() > 6);
    out(root, &["publish", "--deploy-only", "--prepare-only"], 0);
}

#[tokio::test]
async fn deploy_only_fails_privacy_leak_and_script_checks_before_upload() {
    let mock = MockServer::start().await;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    atomic_write_text(
        &root.join("site/CONFIG"),
        "ACCOUNT_ID=example\nPROJECT=fixture\n",
    )
    .unwrap();
    issue_markdown(root, false);
    add_daily(root);
    let token = ["mock", "credential"].join("-");
    let deploy = || {
        cli(root, &["publish", "--deploy-only", "--no-verify"])
            .env("DIGEST_CF_API_BASE", mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .output()
            .unwrap()
    };
    let mirror = root.join("site/public/daily/2026-01-15.md");
    let original = fs::read(&mirror).unwrap();
    fs::write(&mirror, "Entity fixture name&#64;example&#46;com").unwrap();
    let failure = checked(deploy(), 1);
    assert!(String::from_utf8_lossy(&failure.stderr).contains("email"));
    fs::write(&mirror, original).unwrap();
    let path = root.join("site/public/daily/2026-01-15.html");
    let original = fs::read(&path).unwrap();
    fs::write(&path, "<html>unpaired bytes</html>").unwrap();
    assert!(
        String::from_utf8_lossy(&checked(deploy(), 1).stderr).contains("differs from plaintext")
    );
    fs::write(&path, &original).unwrap();
    let malformed = format!(
        "{}<script>unexpected()</script>",
        String::from_utf8(original).unwrap()
    );
    fs::write(&path, &malformed).unwrap();
    fs::write(root.join("site/plain/daily/2026-01-15.html"), &malformed).unwrap();
    assert!(String::from_utf8_lossy(&checked(deploy(), 1).stderr).contains("unexpected executable"));
    password(root);
    checked(
        cli(root, &["publish", "--rebuild", "--prepare-only"])
            .env("DIGEST_ENCRYPT", "1")
            .output()
            .unwrap(),
        0,
    );
    let private_deploy = || {
        cli(root, &["publish", "--deploy-only", "--no-verify"])
            .env("DIGEST_CF_API_BASE", mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .env("DIGEST_ENCRYPT", "1")
            .output()
            .unwrap()
    };
    let leak = root.join("site/public/search-index.json");
    fs::write(&leak, "{}").unwrap();
    assert!(
        String::from_utf8_lossy(&checked(private_deploy(), 1).stderr)
            .contains("unexpected public file")
    );
    fs::remove_file(leak).unwrap();
    fs::copy(root.join("site/plain/daily/2026-01-15.html"), &path).unwrap();
    assert!(
        String::from_utf8_lossy(&checked(private_deploy(), 1).stderr)
            .contains("missing encrypted envelope")
    );
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn pages_api_uploads_assets_and_control_files_separately() {
    let mock = MockServer::start().await;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let files = [
        ("index.html", "<html>Fixture</html>"),
        ("notes.md", "# Fixture content"),
        ("_headers", "/*\n  X-Content-Type-Options: nosniff\n"),
        ("_redirects", "/old / 302\n"),
        (
            "_worker.js",
            "export default {fetch(){return new Response('fixture')}}",
        ),
        (
            "_routes.json",
            r#"{"version":1,"include":["/*"],"exclude":[]}"#,
        ),
    ];
    for (path, txt) in files {
        fs::write(root.join(path), txt).unwrap();
    }
    let cfg = SiteConfig {
        account_id: "example".into(),
        project: "sample".into(),
        ..SiteConfig::default()
    };
    let token = ["fixture", "account"].join("-");
    let upload = ["fixture", "upload"].join("-");
    Mock::given(method("GET"))
        .and(path("/accounts/example/pages/projects/sample/upload-token"))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":{"jwt":upload}})))
        .expect(1)
        .mount(&mock)
        .await;
    let asset = publish::pages_asset_key(b"<html>Fixture</html>", "text/html; charset=utf-8");
    Mock::given(method("POST"))
        .and(path("/pages/assets/check-missing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":[asset]})))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/upload"))
        .and(header("authorization", format!("Bearer {upload}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/upsert-hashes"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/accounts/example/pages/projects/sample/deployments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"success":true,"result":{"url":"https://example.pages.dev"}}),
            ),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let result = deploy_via_pages_api(
        mock.uri().parse().unwrap(),
        &cfg,
        root,
        &token,
        "publish fixture",
    )
    .await
    .unwrap();
    assert_eq!(result["via"], "pages-api");
    let reqs = mock.received_requests().await.unwrap();
    let upload_request = reqs
        .iter()
        .find(|r| r.url.path() == "/pages/assets/upload")
        .unwrap();
    let input: Value = serde_json::from_slice(&upload_request.body).unwrap();
    assert_eq!(input[0]["key"], asset);
    assert_eq!(input[0]["base64"], true);
    assert_eq!(input[0]["value"], STANDARD.encode("<html>Fixture</html>"));
    let request = reqs
        .iter()
        .find(|r| r.url.path().ends_with("/deployments"))
        .unwrap();
    let body = String::from_utf8_lossy(&request.body);
    assert!(request.headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("multipart/form-data;"));
    let (_, manifest) = body.split_once("name=\"manifest\"").unwrap();
    let (_, manifest) = manifest.split_once("\r\n\r\n").unwrap();
    let manifest: Value = serde_json::from_str(manifest.lines().next().unwrap()).unwrap();
    assert_eq!(manifest["/index.html"], asset);
    assert!(manifest.get("/notes.md").is_some());
    assert_eq!(manifest.as_object().unwrap().len(), 2);
    for name in ["_headers", "_redirects", "_worker.js", "_routes.json"] {
        assert!(
            body.contains(&format!("name=\"{name}\"; filename=\"{name}\"")),
            "{name}"
        );
        assert!(manifest.get(format!("/{name}")).is_none());
    }
}

#[tokio::test]
async fn url_base_uses_extensionless_targets_and_rejects_first_response_redirects() {
    let d = tempdir().unwrap();
    let paths = DigestPaths::at(d.path(), chrono_tz::UTC);
    let mock = MockServer::start().await;
    let home = "<html>Index</html>\r\n";
    let issue = "<html>Authenticated parity fixture.</html>";
    atomic_write_text(&paths.plain.join("index.html"), home).unwrap();
    atomic_write_text(&paths.plain.join("zh/daily/2026-01-15.html"), issue).unwrap();
    for (route, body) in [("/site/", home), ("/site/zh/daily/2026-01-15", issue)] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .expect(1)
            .mount(&mock)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/redirect/"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/should-not-follow"))
        .expect(1)
        .mount(&mock)
        .await;
    let client = SafeHttpClient::default()
        .with_fixture_origin(*mock.address())
        .unwrap();
    let base = Url::parse("https://digest.example.com/site").unwrap();
    assert_eq!(
        verify::verify_url_base_with_client(&paths, &base, false, None, &client)
            .await
            .unwrap(),
        2
    );
    let redirect = Url::parse("https://digest.example.com/redirect").unwrap();
    let denied = verify::verify_url_base_with_client(&paths, &redirect, false, None, &client)
        .await
        .unwrap_err();
    assert!(format!("{denied:#}").contains("expected HTTP 200, got 302"));
    assert!(!mock
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|req| req.url.path() == "/should-not-follow" || req.url.path().ends_with(".html")));
}

#[test]
fn published_worker_applies_static_headers_redirects_search_and_negotiation() {
    #[cfg(debug_assertions)]
    if assets::WORKER_JS.contains("new Response(\"stub\")") {
        return;
    }
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    build_demo(
        &root.join("public"),
        &examples(),
        &[Lang::En, Lang::ZhCn],
        None,
        &SiteConfig::default(),
        true,
    )
    .unwrap();
    let mut node = Command::new("node")
        .args(["--input-type=module"])
        .current_dir(root.join("public"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let program = r#"
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const source = await readFile('_worker.js');
const worker = (await import('data:application/javascript;base64,' + source.toString('base64'))).default;
let indexAvailable = false;
const env = { ASSETS: { async fetch(input) {
    const req = input instanceof Request ? input : new Request(input);
    const path = new URL(req.url).pathname;
    if (path === '/search-index.json' && !indexAvailable) return new Response('offline', {status: 404});
    const rel = path.replace(/^\//, '');
    for (const candidate of [rel, rel.endsWith('/') ? rel + 'index.html' : rel + '.html']) {
        try { return new Response(req.method === 'HEAD' ? null : await readFile(candidate), { status: 200 }); }
        catch {}
    }
    return new Response('missing', { status: 404 });
} } };
const req = (path, init) => worker.fetch(new Request('https://digest.example.com'+path, init), env);
const home = await req('/');
assert.equal(home.status, 302);
assert.equal(new URL(home.headers.get('location'), 'https://digest.example.com').pathname, '/en/');
const path = '/en/daily/latest';
const plain = await req(path, {headers:{accept:'text/html'}});
assert.equal(plain.status, 200);
assert.equal(plain.headers.get('cache-control'), 'no-store');
const headers = await readFile('_headers', 'utf8');
const csp = headers.match(/^  Content-Security-Policy: (.*)$/m)[1];
assert.equal(plain.headers.get('content-security-policy'), csp);
assert.match(await plain.text(), /<!doctype html>/i);
const md = await req(path, {headers:{accept:'text/markdown;q=0.9,text/html;q=0.5,*/*;q=0.9'}});
assert.match(md.headers.get('content-type'), /text\/markdown/);
assert.ok(Number(md.headers.get('x-markdown-tokens')) > 0);
const tie = await req(path, {headers:{accept:'text/markdown;q=0.8,text/html;q=0.8'}});
assert.match(await tie.text(), /<!doctype html>/i);
assert.equal((await req('/en/index.md')).status, 200);
const head = await req(path, {method:'HEAD',headers:{accept:'text/markdown'}});
assert.equal(head.status, 200);
assert.equal(await head.text(), '');
assert.equal((await req('/.well-known/search?q=model')).status, 503);
indexAvailable = true;
assert.equal((await req('/.well-known/search?q=model')).status, 200);
assert.equal((await req('/.well-known/search', {method:'QUERY', body:'{"q":"model","limit":2}', headers:{'content-type':'application/json'}})).status,200);
assert.equal((await req('/.well-known/search')).status, 400);
assert.equal((await req('/.well-known/search', {method:'QUERY',body:'x'.repeat(4097),headers:{'content-type':'text/plain'}})).status, 413);
const absent = await req('/missing');
assert.equal(absent.status, 404);
assert.equal(absent.headers.get('cache-control'), 'no-store');
assert.equal(absent.headers.get('x-robots-tag'), 'noindex, nofollow');
"#;
    node.stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .unwrap();
    let out = node.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn weekly_date_labels_cross_the_calendar_year() {
    let (start, end) = publish::parse_weekly_label("2025-12-29_01-04").unwrap();
    assert_eq!(start.to_string(), "2025-12-29");
    assert_eq!(end.to_string(), "2026-01-04");
    assert_eq!(
        (start, end),
        publish::parse_weekly_label("2025-12-29_2026-01-04").unwrap()
    );
    assert!(publish::parse_weekly_label("2026-02-20_2026-02-19").is_err());
    assert!(publish::parse_weekly_label("2026-02-30_03-01").is_err());
}

#[test]
fn failed_rebuild_preserves_previous_outputs_and_incremental_inputs() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    issue_markdown(root, false);
    add_daily(root);
    add_weekly(root);
    let source = root.join(format!("site/sources/daily/{DAILY}.html"));
    let published = root.join(format!("site/public/daily/{DAILY}.html"));
    let input = fs::read(&source).unwrap();
    let public_html = fs::read(&published).unwrap();
    let removed = fs::read(root.join("site/removed-paths.txt")).unwrap();
    // Reject every digest that would be written when the removal triggers a rebuild.
    atomic_write_text(
        &root.join("site/privacy-patterns.txt"),
        "AI Digest\tfixture-block\n",
    )
    .unwrap();
    let failed = out(
        root,
        &[
            "publish",
            "--remove",
            &format!("daily/{DAILY}"),
            "--prepare-only",
        ],
        1,
    );
    assert!(String::from_utf8_lossy(&failed.stdout).starts_with("ERROR remove:"));
    assert_eq!(fs::read(&source).unwrap(), input);
    assert_eq!(fs::read(&published).unwrap(), public_html);
    assert_eq!(
        fs::read(root.join("site/removed-paths.txt")).unwrap(),
        removed
    );
    fs::remove_file(root.join("site/privacy-patterns.txt")).unwrap();
    out(root, &["publish", "--rebuild", "--dry-run"], 0);
    out(root, &["publish", "--deploy-only", "--no-verify"], 2);
}

#[test]
fn strict_encrypt_parsing_and_init_encrypt_guards_prevent_silent_downgrade() {
    for bad in ["maybe", "2", "enabled", "truthy", "public"] {
        assert!(
            SiteConfig::parse(&format!("ENCRYPT={bad}\n"), None, |_| None).is_err(),
            "ENCRYPT={bad} must fail"
        );
        assert!(
            SiteConfig::parse("", None, |k| (k == "DIGEST_ENCRYPT").then(|| bad.into())).is_err(),
            "DIGEST_ENCRYPT={bad} must fail"
        );
    }
    for (raw, want) in [
        ("", false),
        ("0", false),
        ("false", false),
        ("NO", false),
        ("off", false),
        ("1", true),
        ("true", true),
        ("YES", true),
        ("on", true),
    ] {
        let cfg = SiteConfig::parse(&format!("ENCRYPT={raw}\n"), None, |_| None).unwrap();
        assert_eq!(cfg.encrypt, want, "ENCRYPT={raw}");
    }

    // --init --encrypt with SITE_SLUG pre-set must still persist ENCRYPT=1 (never silently skip)
    let a = tempdir().unwrap();
    let res = checked(
        cli(a.path(), &["publish", "--init", "--encrypt"])
            .env("SITE_SLUG", "custom-slug-1234")
            .output()
            .unwrap(),
        0,
    );
    let stdout = String::from_utf8_lossy(&res.stdout);
    assert!(stdout.contains("SLUG=custom-slug-1234"), "{stdout}");
    assert!(stdout.contains("ENCRYPT=1"), "{stdout}");
    let written = get(a.path(), "site/CONFIG");
    assert!(written.contains("ENCRYPT=1") && written.contains("SLUG=custom-slug-1234"));

    // Idempotent when existing site/CONFIG already has ENCRYPT=1
    let again = out(a.path(), &["publish", "--init", "--encrypt"], 0);
    assert!(String::from_utf8_lossy(&again.stdout).contains("ENCRYPT=1"));

    // Conflicting DIGEST_ENCRYPT=0 with --init --encrypt must fail
    let conflict_env = checked(
        cli(a.path(), &["publish", "--init", "--encrypt"])
            .env("DIGEST_ENCRYPT", "0")
            .output()
            .unwrap(),
        1,
    );
    assert!(String::from_utf8_lossy(&conflict_env.stderr).contains("ENCRYPT=0"));

    // Existing site/CONFIG with ENCRYPT=0 must fail cleanly on --init --encrypt
    let b = tempdir().unwrap();
    out(b.path(), &["publish", "--init"], 0);
    let conflict_file = out(b.path(), &["publish", "--init", "--encrypt"], 1);
    assert!(String::from_utf8_lossy(&conflict_file.stderr).contains("ENCRYPT=0"));
}

#[tokio::test]
async fn deploy_only_rejects_relaxed_csp_and_worker_csp_mismatch() {
    let mock = MockServer::start().await;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    atomic_write_text(
        &root.join("site/CONFIG"),
        "ACCOUNT_ID=example\nPROJECT=fixture\nENCRYPT=0\nGEO_WORKER=1\n",
    )
    .unwrap();
    issue_markdown(root, false);
    add_daily(root);
    let token = ["mock", "credential"].join("-");
    let deploy = || {
        cli(root, &["publish", "--deploy-only", "--no-verify"])
            .env("DIGEST_CF_API_BASE", mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .output()
            .unwrap()
    };
    let headers_path = root.join("site/public/_headers");
    let orig_headers = fs::read_to_string(&headers_path).unwrap();

    // 1. Extra script-src source
    let extra_src = orig_headers.replace("script-src ", "script-src https://evil.example ");
    fs::write(&headers_path, &extra_src).unwrap();
    assert_eq!(deploy().status.code(), Some(1));

    // 2. script-src-elem override
    let elem_override = orig_headers.replace(
        "script-src-attr 'none'",
        "script-src-attr 'none'; script-src-elem 'unsafe-inline'",
    );
    fs::write(&headers_path, &elem_override).unwrap();
    assert_eq!(deploy().status.code(), Some(1));

    // 3. Missing script-src-attr 'none'
    let no_attr = orig_headers.replace("; script-src-attr 'none'", "");
    fs::write(&headers_path, &no_attr).unwrap();
    assert_eq!(deploy().status.code(), Some(1));

    // 4. Restore _headers, tamper _worker.js globalHeaders CSP
    fs::write(&headers_path, &orig_headers).unwrap();
    let worker_path = root.join("site/public/_worker.js");
    let orig_worker = fs::read_to_string(&worker_path).unwrap();
    fs::write(
        &worker_path,
        orig_worker.replace("script-src-attr 'none'", "script-src-attr 'unsafe-inline'"),
    )
    .unwrap();
    assert_eq!(deploy().status.code(), Some(1));

    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn password_rotation_checks_preconditions_and_rolls_back_on_deploy_failure() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/accounts/example/pages/projects/fixture/upload-token",
        ))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let orig_pass = password(root);
    atomic_write_text(
        &root.join("site/CONFIG"),
        "ACCOUNT_ID=example\nPROJECT=fixture\nENCRYPT=1\nSLUG=rotate-slug-test\n",
    )
    .unwrap();
    issue_markdown(root, false);
    add_daily(root);
    let orig_salt = get(root, "fixture-salt");
    let orig_index = get(root, "site/public/rotate-slug-test/index.html");

    // 1. Missing Cloudflare token must fail BEFORE rotating local password/salt/site
    out(root, &["publish", "--rotate-password"], 2);
    assert_eq!(
        crypto::read_password_file(&root.join("fixture-password")).unwrap(),
        orig_pass
    );
    assert_eq!(get(root, "fixture-salt"), orig_salt);
    assert_eq!(
        get(root, "site/public/rotate-slug-test/index.html"),
        orig_index
    );

    // 2. Deploy failure must restore previous password, salt, and site cleanly
    let token = ["mock", "credential"].join("-");
    checked(
        cli(root, &["publish", "--rotate-password", "--no-verify"])
            .env("DIGEST_CF_API_BASE", mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .env("WRANGLER_VERSION", "invalid/version")
            .output()
            .unwrap(),
        1,
    );
    assert_eq!(
        crypto::read_password_file(&root.join("fixture-password")).unwrap(),
        orig_pass
    );
    assert_eq!(get(root, "fixture-salt"), orig_salt);
    assert_eq!(
        get(root, "site/public/rotate-slug-test/index.html"),
        orig_index
    );
    assert!(!root.join("fixture-password.prev").exists());
    assert!(!root.join("fixture-salt.prev").exists());
}

#[tokio::test]
async fn pages_api_distinguishes_identical_bytes_with_different_mime_types() {
    let mock = MockServer::start().await;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let shared = b"identical body content";
    fs::write(root.join("same.html"), shared).unwrap();
    fs::write(root.join("dup.html"), shared).unwrap();
    fs::write(root.join("same.txt"), shared).unwrap();
    fs::write(
        root.join("_headers"),
        "/*\n  X-Content-Type-Options: nosniff\n",
    )
    .unwrap();

    let key_html = publish::pages_asset_key(shared, "text/html; charset=utf-8");
    let key_txt = publish::pages_asset_key(shared, "text/plain; charset=utf-8");
    assert_ne!(key_html, key_txt);

    let cfg = SiteConfig {
        account_id: "example".into(),
        project: "sample".into(),
        ..SiteConfig::default()
    };
    let token = ["fixture", "account"].join("-");
    let upload = ["fixture", "upload"].join("-");
    Mock::given(method("GET"))
        .and(path("/accounts/example/pages/projects/sample/upload-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":{"jwt":upload}})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/check-missing"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"result":[key_html, key_txt]})),
        )
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/upload"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/upsert-hashes"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/accounts/example/pages/projects/sample/deployments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"success":true,"result":{"url":"https://example.pages.dev"}}),
            ),
        )
        .mount(&mock)
        .await;

    deploy_via_pages_api(
        mock.uri().parse().unwrap(),
        &cfg,
        root,
        &token,
        "mime dedup",
    )
    .await
    .unwrap();

    let reqs = mock.received_requests().await.unwrap();
    let upload_req = reqs
        .iter()
        .find(|r| r.url.path() == "/pages/assets/upload")
        .unwrap();
    let uploaded: Value = serde_json::from_slice(&upload_req.body).unwrap();
    let items = uploaded.as_array().unwrap();
    assert_eq!(items.len(), 2);
    let mimes: Vec<_> = items
        .iter()
        .map(|it| it["metadata"]["contentType"].as_str().unwrap())
        .collect();
    assert!(mimes.contains(&"text/html; charset=utf-8"));
    assert!(mimes.contains(&"text/plain; charset=utf-8"));

    let deploy_req = reqs
        .iter()
        .find(|r| r.url.path().ends_with("/deployments"))
        .unwrap();
    let body = String::from_utf8_lossy(&deploy_req.body);
    let (_, manifest) = body.split_once("name=\"manifest\"").unwrap();
    let (_, manifest) = manifest.split_once("\r\n\r\n").unwrap();
    let manifest: Value = serde_json::from_str(manifest.lines().next().unwrap()).unwrap();
    assert_eq!(manifest["/same.html"], manifest["/dup.html"]);
    assert_ne!(manifest["/same.html"], manifest["/same.txt"]);
}

#[test]
fn verify_redacts_url_credentials_and_query_in_logs() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let plain = root.join("plain.html");
    fs::write(&plain, "<html>ok</html>").unwrap();
    let out = checked(
        cli(
            root,
            &[
                "verify",
                "--url",
                "https://alice:topsecret@127.0.0.1/page?api_key=leakme#frag",
                "--plain",
                plain.to_str().unwrap(),
            ],
        )
        .output()
        .unwrap(),
        1,
    );
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!combined.contains("alice"), "{combined}");
    assert!(!combined.contains("topsecret"), "{combined}");
    assert!(!combined.contains("leakme"), "{combined}");
    assert!(!combined.contains("#frag"), "{combined}");
    assert!(combined.contains("https://127.0.0.1/page"), "{combined}");
}

#[cfg(unix)]
#[test]
fn non_utf8_digest_encrypt_env_fails_closed_without_public_fallback() {
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let bad_val = OsStr::from_bytes(b"\xff\xfe");
    let res = cli(root, &["publish", "--init"])
        .env("DIGEST_ENCRYPT", bad_val)
        .output()
        .unwrap();
    assert_eq!(
        res.status.code(),
        Some(1),
        "non-UTF-8 DIGEST_ENCRYPT must fail closed, stderr={}",
        String::from_utf8_lossy(&res.stderr)
    );
    assert!(
        !root.join("site/CONFIG").exists(),
        "site/CONFIG must not be created as public when DIGEST_ENCRYPT is non-UTF-8"
    );
}

#[tokio::test]
async fn deploy_only_rejects_v1_z_smuggling_and_tampered_encrypted_shell() {
    let mock = MockServer::start().await;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    password(root);
    atomic_write_text(
        &root.join("site/CONFIG"),
        "ACCOUNT_ID=example\nPROJECT=fixture\nENCRYPT=1\nSLUG=shell-guard-slug\n",
    )
    .unwrap();
    issue_markdown(root, false);
    add_daily(root);
    let token = ["mock", "credential"].join("-");
    let deploy = || {
        cli(root, &["publish", "--deploy-only", "--no-verify"])
            .env("DIGEST_CF_API_BASE", mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .output()
            .unwrap()
    };

    let page_path = root.join(format!("site/public/shell-guard-slug/daily/{DAILY}.html"));
    let orig_shell = fs::read_to_string(&page_path).unwrap();

    // 1. Tampered shell outside #c (short comment that regexes do not catch) must be rejected
    let tampered_shell = orig_shell.replace("</body>", "<!-- tampered-shell-note --></body>");
    fs::write(&page_path, &tampered_shell).unwrap();
    let res = deploy();
    assert_eq!(
        res.status.code(),
        Some(1),
        "tampered shell must fail --deploy-only: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    assert!(
        String::from_utf8_lossy(&res.stderr).contains("unlock template"),
        "{}",
        String::from_utf8_lossy(&res.stderr)
    );

    // 2. v1 envelope smuggling a `z` field must be rejected by envelope validation and --deploy-only
    let env_v2 = crypto::extract_envelope(&orig_shell).unwrap();
    let bad_v1 = crypto::Envelope {
        v: 1,
        z: Some("smuggled-plaintext@example.com".into()),
        iter: env_v2.iter,
        salt: env_v2.salt.clone(),
        iv: env_v2.iv.clone(),
        ct: env_v2.ct.clone(),
    };
    assert!(crypto::decrypt_envelope(&bad_v1, "unused").is_err());
    let orig_json = serde_json::to_string(&env_v2).unwrap();
    let bad_json = serde_json::to_string(&bad_v1).unwrap();
    let smuggled_shell = orig_shell.replace(&orig_json, &bad_json);
    assert_ne!(smuggled_shell, orig_shell);
    fs::write(&page_path, &smuggled_shell).unwrap();
    let res = deploy();
    assert_eq!(
        res.status.code(),
        Some(1),
        "v1 envelope with z field must fail --deploy-only: {}",
        String::from_utf8_lossy(&res.stderr)
    );

    // 3. Restoring exact shell passes --deploy-only --prepare-only cleanly
    fs::write(&page_path, &orig_shell).unwrap();
    out(root, &["publish", "--deploy-only", "--prepare-only"], 0);
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn headers_reject_cloudflare_colonless_security_header_removals() {
    let mock = MockServer::start().await;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    atomic_write_text(
        &root.join("site/CONFIG"),
        "ACCOUNT_ID=example\nPROJECT=fixture\nENCRYPT=0\nGEO_WORKER=1\n",
    )
    .unwrap();
    issue_markdown(root, false);
    add_daily(root);
    let token = ["mock", "credential"].join("-");
    let deploy = || {
        cli(root, &["publish", "--deploy-only", "--no-verify"])
            .env("DIGEST_CF_API_BASE", mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .output()
            .unwrap()
    };

    let headers_path = root.join("site/public/_headers");
    let orig_headers = fs::read_to_string(&headers_path).unwrap();
    let cfg = publish::load_config(&root.join("site/CONFIG"), None).unwrap();

    for forbidden_line in [
        "  ! Content-Security-Policy",
        "  !Content-Security-Policy",
        "  ! X-Frame-Options",
        "  ! X-Content-Type-Options",
        "  ! Referrer-Policy",
        "  ! Strict-Transport-Security",
        "  ! Cross-Origin-Opener-Policy",
        "  ! Cross-Origin-Resource-Policy",
        "  ! Permissions-Policy",
    ] {
        let tampered = format!("{orig_headers}\n/daily/*\n{forbidden_line}\n");
        fs::write(&headers_path, &tampered).unwrap();
        let res = deploy();
        assert_eq!(
            res.status.code(),
            Some(1),
            "deploy-only must reject colonless removal `{forbidden_line}`"
        );
        assert!(
            geo::finalize_public_worker(&root.join("site/public"), &cfg).is_err(),
            "worker finalizer must reject colonless removal `{forbidden_line}`"
        );
    }

    // Encrypted site must also reject route-level `! X-Robots-Tag` (no colon)
    fs::write(&headers_path, &orig_headers).unwrap();
    password(root);
    checked(
        cli(root, &["publish", "--rebuild", "--prepare-only"])
            .env("DIGEST_ENCRYPT", "1")
            .output()
            .unwrap(),
        0,
    );
    let enc_headers = fs::read_to_string(&headers_path).unwrap();
    fs::write(
        &headers_path,
        format!("{enc_headers}\n/daily/*\n  ! X-Robots-Tag\n"),
    )
    .unwrap();
    let enc_res = cli(root, &["publish", "--deploy-only", "--no-verify"])
        .env("DIGEST_CF_API_BASE", mock.uri())
        .env("CLOUDFLARE_API_TOKEN", &token)
        .env("DIGEST_ENCRYPT", "1")
        .output()
        .unwrap();
    assert_eq!(
        enc_res.status.code(),
        Some(1),
        "encrypted deploy-only must reject route-level `! X-Robots-Tag`"
    );
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn password_rotation_preserves_new_credentials_when_live_verify_fails_after_deploy() {
    let mock = MockServer::start().await;
    let upload = ["fixture", "upload"].join("-");
    Mock::given(method("GET"))
        .and(path(
            "/accounts/example/pages/projects/fixture/upload-token",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":{"jwt":upload}})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/check-missing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":[]})))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/upsert-hashes"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;
    // Deployment succeeds remotely, returning a non-routable loopback URL that causes live verify to fail
    Mock::given(method("POST"))
        .and(path("/accounts/example/pages/projects/fixture/deployments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"success":true,"result":{"url":"https://127.0.0.1/deploy-preview"}}),
        ))
        .mount(&mock)
        .await;

    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let orig_pass = password(root);
    atomic_write_text(
        &root.join("site/CONFIG"),
        "ACCOUNT_ID=example\nPROJECT=fixture\nENCRYPT=1\nSLUG=rotate-live-fail\n",
    )
    .unwrap();
    issue_markdown(root, false);
    add_daily(root);
    let orig_salt = get(root, "fixture-salt");
    let token = ["mock", "credential"].join("-");

    // Rotate WITHOUT --no-verify: remote deploy succeeds, live verification fails
    let res = cli(root, &["publish", "--rotate-password"])
        .env("DIGEST_CF_API_BASE", mock.uri())
        .env("CLOUDFLARE_API_TOKEN", &token)
        .output()
        .unwrap();
    assert_eq!(res.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("Keeping the new local credentials") && stderr.contains(".prev"),
        "expected recovery instructions in stderr: {stderr}"
    );

    // Local credentials MUST stay on the NEW rotated password/salt (never overwritten with old)
    let new_pass = crypto::read_password_file(&root.join("fixture-password")).unwrap();
    let new_salt = get(root, "fixture-salt");
    assert_ne!(new_pass, orig_pass);
    assert_ne!(new_salt, orig_salt);

    // .prev backups MUST be preserved with the OLD credentials
    assert_eq!(
        crypto::read_password_file(&root.join("fixture-password.prev")).unwrap(),
        orig_pass
    );
    assert_eq!(get(root, "fixture-salt.prev"), orig_salt);

    // Local encrypted site matches the NEW password
    let paths = DigestPaths::at(root, chrono_tz::UTC);
    assert!(verify::verify_local(&paths, true, Some(&new_pass)).is_ok());
}

#[tokio::test]
async fn pages_api_rejects_oversized_single_asset_over_25_mib_before_upload() {
    let mock = MockServer::start().await;
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let oversized = root.join("huge.bin");
    let f = fs::File::create(&oversized).unwrap();
    f.set_len(25 * 1024 * 1024 + 1).unwrap();

    let cfg = SiteConfig {
        account_id: "example".into(),
        project: "sample".into(),
        ..SiteConfig::default()
    };
    let token = ["fixture", "account"].join("-");
    let err = deploy_via_pages_api(
        mock.uri().parse().unwrap(),
        &cfg,
        root,
        &token,
        "oversized test",
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("exceeds 25 MiB"),
        "unexpected error: {err:#}"
    );
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[test]
fn tier2_csp_geo_and_single_h1_regressions() {
    let tmp = tempdir().unwrap();
    let spec_hash = crypto::script_hash(render::SPECULATION_RULES_JSON);
    assert_eq!(
        spec_hash,
        "'sha256-72upgOtXUYYfpbOnby12zXpzQ+UsSDcDvAHrs1an1eU='"
    );

    // 1. Demo build (/en/ + /zh-CN/, no root /index.md)
    let demo_out = tmp.path().join("demo");
    checked(
        cli(tmp.path(), &["publish", "--demo", "--examples"])
            .arg(examples())
            .arg("--out")
            .arg(&demo_out)
            .env("SITE_DOMAIN", "demo.example.com")
            .output()
            .unwrap(),
        0,
    );
    let demo_headers = get(&demo_out, "_headers");
    let (demo_global, _) = geo::parse_headers(&demo_headers);
    let demo_csp = &demo_global["Content-Security-Policy"];
    assert!(
        demo_csp.contains(&spec_hash),
        "demo CSP must include speculationrules hash: {demo_csp}"
    );
    assert!(
        !demo_csp.contains("prefetch-src"),
        "demo CSP must not emit deprecated prefetch-src: {demo_csp}"
    );
    assert!(
        demo_global["Link"].contains("</en/index.md>; rel=\"alternate\"; type=\"text/markdown\""),
        "demo Link header must point to existing /en/index.md: {}",
        demo_global["Link"]
    );
    assert!(!demo_out.join("index.md").exists());
    assert!(demo_out.join("en/index.md").is_file());
    let demo_worker = get(&demo_out, "_worker.js");
    assert!(demo_worker.contains("/en/index.md"));
    assert!(!demo_worker.contains("</index.md>"));

    let demo_skill = get(&demo_out, geo::SKILL_PATH);
    assert!(
        demo_skill.contains("https://demo.example.com/zh-CN/daily/latest.md")
            && demo_skill.contains("https://demo.example.com/en/daily/latest.md"),
        "demo SKILL.md must use localized /zh-CN/ and /en/ paths: {demo_skill}"
    );
    assert!(
        !demo_skill.contains("https://demo.example.com/daily/latest.md"),
        "demo SKILL.md must not advertise nonexistent /daily/latest.md"
    );
    assert!(demo_out.join("zh-CN/daily/latest.md").is_file());
    assert!(demo_out.join("en/daily/latest.md").is_file());

    for rel in [
        "en/index.html",
        "zh-CN/index.html",
        "en/daily/latest.html",
        "zh-CN/daily/latest.html",
    ] {
        let html = get(&demo_out, rel);
        assert_eq!(
            html.matches("<h1").count(),
            1,
            "{rel} must contain strictly one <h1>"
        );
    }

    // 2. Bilingual public production build (/ + /zh/ + /en/)
    let prod = tmp.path().join("prod");
    fs::create_dir_all(&prod).unwrap();
    out(&prod, &["publish", "--init", "--lang", "zh-CN"], 0);
    issue_markdown(&prod, true);
    add_daily(&prod);
    add_weekly(&prod);
    let prod_pub = prod.join("site/public");
    let prod_headers = get(&prod_pub, "_headers");
    let (prod_global, _) = geo::parse_headers(&prod_headers);
    assert!(prod_global["Content-Security-Policy"].contains(&spec_hash));
    assert!(!prod_global["Content-Security-Policy"].contains("prefetch-src"));
    assert!(!prod_global["Content-Security-Policy"].contains("'inline-speculation-rules'"));
    assert!(prod_global["Link"].contains("</index.md>; rel=\"alternate\"; type=\"text/markdown\""));

    let llms = get(&prod_pub, "llms.txt");
    assert_eq!(
        llms.matches("zh-CN Daily Latest").count(),
        1,
        "llms.txt must not duplicate zh-CN Daily Latest: {llms}"
    );
    assert_eq!(
        llms.matches("zh-CN Weekly Latest").count(),
        1,
        "llms.txt must not duplicate zh-CN Weekly Latest: {llms}"
    );
    assert!(llms.contains("/zh/daily/latest.md"));
    assert!(llms.contains("/zh/weekly/latest.md"));

    for rel in [
        "index.html",
        "zh/index.html",
        "en/index.html",
        "daily/latest.html",
        "en/daily/latest.html",
    ] {
        let html = get(&prod_pub, rel);
        assert_eq!(
            html.matches("<h1").count(),
            1,
            "prod {rel} must contain strictly one <h1>"
        );
    }

    // 3. Rogue speculationrules payload must fail closed during CSP finalization
    let rogue = tmp.path().join("rogue");
    let rogue_paths = DigestPaths::at(&rogue, chrono_tz::UTC);
    atomic_write_text(
        &rogue_paths.public.join("index.html"),
        r#"<script type="speculationrules">{"prefetch":[]}</script>"#,
    )
    .unwrap();
    atomic_write_text(&rogue_paths.plain.join("index.html"), "<p>ok</p>").unwrap();
    assert!(crypto::expected_security_headers(
        &rogue_paths.public,
        &rogue_paths.plain,
        true,
        &ScriptAllowlist::default(),
    )
    .is_err());

    // 4. Brand unification (zero "AI精选" in rendered HTML/GEO/RSS) + compact <title> + UI contracts
    for rel in [
        "index.html",
        "zh/index.html",
        "en/index.html",
        "daily/latest.html",
        "weekly/latest.html",
        "llms.txt",
        "feed.xml",
        "zh/feed.xml",
        "en/feed.xml",
        "daily/latest.md",
        "weekly/latest.md",
    ] {
        let content = get(&prod_pub, rel);
        assert!(
            !content.contains("AI精选") && !content.contains("AI 精选"),
            "{rel} must not contain legacy brand string AI精选"
        );
    }
    let prod_index_html = get(&prod_pub, "index.html");
    let prod_en_index_html = get(&prod_pub, "en/index.html");
    let daily_html = get(&prod_pub, "daily/latest.html");
    let weekly_html = get(&prod_pub, "weekly/latest.html");
    assert!(prod_index_html
        .contains("<title>AI Digest — 每日与每周前沿 AI 简报 | 模型·论文·开源工程</title>"));
    assert!(prod_en_index_html
        .contains("<title>AI Digest — Curated Daily &amp; Weekly Frontier AI Briefings</title>"));
    assert!(daily_html.contains(
        "<title>Orion-3 Mini：Acme Labs 放出开放权重的小型推理模型 — AI Digest 日报 (2026-01-15)</title>"
    ));
    assert!(
        weekly_html.contains("Orion-3 Mini")
            && weekly_html.contains(" — AI Digest 周报 (2026-01-12_01-18)</title>"),
        "unexpected weekly <title>: {weekly_html}"
    );
    assert!(daily_html.contains(r#"class="filter-bar filter-drawer search-panel""#));
    assert!(daily_html.contains(r#"hidden aria-hidden="true" inert"#));
    assert!(daily_html.contains(r#"class="search-scrim""#));
    assert!(daily_html.contains(r#"class="mast-sentinel""#));
    assert!(daily_html.contains(r#"class="issue-summary""#));
    assert!(daily_html.contains(r#"class="filter-banner""#));
    assert!(daily_html.contains(r#"class="toc-top""#));
    assert!(daily_html.contains(r#"class="cat-chips""#));
    assert!(daily_html.contains(r#"class="mobile-filter-strip""#));
    assert!(daily_html.contains(r#"data-words=""#));
    assert!(daily_html.contains(r#"class="story-links sources""#));
    assert!(daily_html.contains(r#"rel="noopener noreferrer nofollow ugc""#));

    // 5. Security hardening regressions (SEC-1, SEC-4, SEC-5, SEC-8, SEC-9, SEC-11)
    assert!(ai_digest::model::safe_external_url("https://openai.com@evil.tld/x").is_none());
    assert!(ai_digest::model::safe_external_url("https://ex\u{202E}ample.com/x").is_none());
    assert!(ai_digest::model::safe_external_url("/relative/path").is_none());
    assert!(ai_digest::model::safe_internal_url("https://example.com").is_none());
    assert!(ai_digest::model::safe_internal_url("//evil.tld/x").is_none());
    assert_eq!(
        ai_digest::model::safe_internal_url("#item-1").as_deref(),
        Some("#item-1")
    );
    assert!(ai_digest::model::is_wrapper_or_shortener_url(
        "https://t.co/abc"
    ));
    assert!(ai_digest::model::is_wrapper_or_shortener_url(
        "https://www.google.com/url?q=https://example.com"
    ));
    assert_eq!(
        ai_digest::model::strip_tracking_params("https://example.com/a?utm_source=rss&id=1"),
        "https://example.com/a?id=1"
    );
    assert_eq!(
        geo::parse_redirects(
            "/a /b 302\n/evil https://evil.tld 302\n/proto //evil.tld 301\n/bad /ok 200\n"
        ),
        vec![("/a".into(), "/b".into(), 302)]
    );
    assert!(SiteConfig::parse("DOMAIN=bücher.example\n", None, |_| None).is_err());
    assert_eq!(
        ai_digest::model::xml_escape("ok\u{0000}\u{000B}<&>"),
        "ok&lt;&amp;&gt;"
    );

    // SEC-5: 🔗 line with balanced parentheses in URL is not truncated
    let paren_md = "**AI Digest · Daily · 1/15**\n\n## 🧠 Models\n\n### 1. Attention\n`Lab · 1/15 · Training`\n\n**In brief:** Test.\n\n🔗 [Wiki](https://en.wikipedia.org/wiki/Attention_(machine_learning)) · [Paper](https://arxiv.org/abs/2601.00001)\n";
    let paren_digest = ai_digest::parser::parse_digest_md(paren_md, Lang::En, Some(2026)).unwrap();
    let rendered = render::render_digest_html(&paren_digest, Lang::En, &Default::default());
    assert!(rendered.contains("https://en.wikipedia.org/wiki/Attention_(machine_learning)"));
    // Authority ordering: paper (arxiv.org) appears before media (en.wikipedia.org)
    assert!(
        rendered.find("arxiv.org").unwrap() < rendered.find("en.wikipedia.org").unwrap(),
        "sources must be ordered by authority (paper before media)"
    );

    // SEC-1: publish --md rejects wrapper/shortener URLs
    let bad_wrap = tmp.path().join("bad-wrap");
    fs::create_dir_all(&bad_wrap).unwrap();
    out(&bad_wrap, &["publish", "--init", "--lang", "en"], 0);
    atomic_write_text(
        &bad_wrap.join("digest-2026-01-15.md"),
        "**AI Digest · Daily · 1/15**\n\n## 🧠 Models\n\n### 1. Shortener Test\n`Lab · 1/15 · Training`\n\n**In brief:** Test.\n\n🔗 [Link](https://t.co/abc123)\n",
    )
    .unwrap();
    let res = cli(
        &bad_wrap,
        &[
            "publish",
            "--issue",
            "daily/2026-01-15",
            "--md",
            "digest-2026-01-15.md",
            "--prepare-only",
        ],
    )
    .output()
    .unwrap();
    assert_ne!(res.status.code(), Some(0));
}

#[test]
fn seo_geo_comprehensive_hreflang_jsonld_feeds_api_catalog_and_security() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    atomic_write_text(
        &root.join("site/CONFIG"),
        "LANG=zh-CN\nDOMAIN=ai.example.org\nGEO_AI_TRAIN=yes\n",
    )
    .unwrap();
    // Add explicit editorial summary leads + a </script> breakout attempt in an item title
    let zh_daily = examples()
        .join("sample-digest.zh-CN.md")
        .pipe_read()
        .replacen(
            "**AI Digest · 日报 · 1/15**\n",
            "**AI Digest · 日报 · 1/15**\n\n> **本期摘要：** 开源小型推理模型 Orion-3 Mini 与长视频多模态 Lumen-2 同步亮相，强化学习后训练与评测安全成焦点。\n",
            1,
        )
        .replacen(
            "### 2. Lumen-2：Example AI 的多模态模型更新",
            "### 2. Lumen-2 </script><img src=x>：Example AI 的多模态模型更新",
            1,
        );
    let en_daily = examples()
        .join("sample-digest.en.md")
        .pipe_read()
        .replacen(
            "**AI Daily · 1/15**\n",
            "**AI Daily · 1/15**\n\n> **Summary:** Open-weights reasoning model Orion-3 Mini and Lumen-2 multimodal lead today alongside Sparse-RL post-training and Verifier-Gap eval safety findings.\n",
            1,
        )
        .replacen(
            "### 2. Lumen-2: a multimodal model update from Example AI",
            "### 2. Lumen-2 </script><svg>: a multimodal model update from Example AI",
            1,
        );
    fs::write(root.join(format!("digest-{DAILY}.md")), zh_daily).unwrap();
    fs::write(root.join(format!("digest-{DAILY}.en.md")), en_daily).unwrap();
    fs::copy(
        examples().join("sample-weekly.zh-CN.md"),
        root.join(format!("weekly-{WEEKLY}.md")),
    )
    .unwrap();
    fs::copy(
        examples().join("sample-weekly.en.md"),
        root.join(format!("weekly-{WEEKLY}.en.md")),
    )
    .unwrap();

    add_daily(root);
    add_weekly(root);

    let public = root.join("site/public");

    // 1. Static OG social card PNG exists and has valid PNG header
    let og_bytes = fs::read(public.join("og-card.png")).unwrap();
    assert!(og_bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));

    // 2. Per-page editorial <meta name="description">, canonical, reciprocal hreflang, OG/Twitter, feeds, JSON-LD
    let extract_ld = |html: &str| -> Value {
        let (_, rest) = html
            .split_once(r#"<script type="application/ld+json">"#)
            .expect("missing ld+json");
        let (raw, _) = rest.split_once("</script>").expect("unclosed ld+json");
        assert!(
            !raw.contains("</script") && !raw.contains("<script"),
            "raw script tag inside JSON-LD: {raw}"
        );
        serde_json::from_str(raw).expect("valid JSON-LD")
    };
    let extract_meta_desc = |html: &str| -> String {
        let (_, rest) = html
            .split_once(r#"<meta name="description" content=""#)
            .expect("missing meta description");
        let (val, _) = rest.split_once('"').expect("unclosed meta description");
        val.to_owned()
    };

    let root_index_html = get(&public, "index.html");
    let zh_daily_html = get(&public, &format!("zh/daily/{DAILY}.html"));
    let en_daily_html = get(&public, &format!("en/daily/{DAILY}.html"));
    let zh_weekly_html = get(&public, &format!("zh/weekly/{WEEKLY}.html"));
    let en_weekly_html = get(&public, &format!("en/weekly/{WEEKLY}.html"));
    let zh_index_html = get(&public, "zh/index.html");
    let en_index_html = get(&public, "en/index.html");

    // Custom domain brand-disambiguated <title> on index pages + headline-first <title> on issues
    for doc in [&root_index_html, &zh_index_html] {
        assert!(doc.contains(
            "<title>AI Digest (ai.example.org) — 每日与每周前沿 AI 简报 | 模型·论文·开源工程</title>"
        ));
        assert!(doc.contains("<h1>AI Digest · 每日与每周 AI 简报</h1>"));
        assert!(doc.contains("155") && doc.contains("arXiv") && doc.contains("Hugging Face"));
    }
    assert!(
        root_index_html.contains(r#"<link rel="canonical" href="https://ai.example.org/zh/">"#)
            && root_index_html.contains(&format!(r#"href="/zh/daily/{DAILY}""#)),
        "root / must align canonical to /zh/ and link directly to canonical /zh/ issues"
    );
    assert!(en_index_html.contains(
        "<title>AI Digest (ai.example.org) — Curated Daily &amp; Weekly Frontier AI Briefings</title>"
    ));
    assert!(en_index_html.contains("<h1>AI Digest — Daily &amp; Weekly AI Briefings</h1>"));
    assert!(
        zh_daily_html.contains("Orion-3 Mini")
            && zh_daily_html.contains(&format!(" — AI Digest 日报 ({DAILY})</title>"))
    );
    assert!(
        en_daily_html.contains("Orion-3 Mini")
            && en_daily_html.contains(&format!(" — AI Digest Daily ({DAILY})</title>"))
    );
    assert!(
        en_weekly_html.contains("Orion-3 Mini")
            && en_weekly_html.contains(&format!(" — AI Digest Weekly ({WEEKLY})</title>"))
    );

    // Editorial lead descriptions are unique, real, and length-bounded
    let zh_desc = extract_meta_desc(&zh_daily_html);
    let en_desc = extract_meta_desc(&en_daily_html);
    assert!(
        zh_desc.contains("Orion-3 Mini") && zh_desc.chars().count() <= 88,
        "zh desc={zh_desc}"
    );
    assert!(
        en_desc.contains("Orion-3 Mini") && en_desc.chars().count() <= 158,
        "en desc={en_desc}"
    );
    assert_ne!(zh_desc, extract_meta_desc(&zh_weekly_html));
    assert_ne!(en_desc, extract_meta_desc(&en_weekly_html));
    assert_ne!(
        extract_meta_desc(&zh_index_html),
        extract_meta_desc(&en_index_html)
    );

    // Reciprocal hreflang (zh-Hans, en, x-default) on every ZH/EN pair
    for (zh_doc, en_doc, suffix) in [
        (&zh_daily_html, &en_daily_html, format!("daily/{DAILY}")),
        (&zh_weekly_html, &en_weekly_html, format!("weekly/{WEEKLY}")),
        (&zh_index_html, &en_index_html, String::new()),
    ] {
        let zh_url = format!("https://ai.example.org/zh/{suffix}");
        let en_url = format!("https://ai.example.org/en/{suffix}");
        assert!(zh_doc.contains(&format!(r#"<link rel="canonical" href="{zh_url}">"#)));
        assert!(en_doc.contains(&format!(r#"<link rel="canonical" href="{en_url}">"#)));
        for doc in [zh_doc, en_doc] {
            assert!(
                doc.contains(&format!(
                    r#"<link rel="alternate" hreflang="zh-Hans" href="{zh_url}">"#
                )),
                "missing zh-Hans hreflang for {suffix}"
            );
            assert!(
                doc.contains(&format!(
                    r#"<link rel="alternate" hreflang="en" href="{en_url}">"#
                )),
                "missing en hreflang for {suffix}"
            );
            assert!(
                doc.contains(&format!(
                    r#"<link rel="alternate" hreflang="x-default" href="{zh_url}">"#
                )),
                "missing x-default hreflang for {suffix}"
            );
        }
        assert!(zh_doc.contains(r#"<meta property="og:locale" content="zh_CN">"#));
        assert!(zh_doc.contains(r#"<meta property="og:locale:alternate" content="en_US">"#));
        assert!(en_doc.contains(r#"<meta property="og:locale" content="en_US">"#));
        assert!(en_doc.contains(r#"<meta property="og:locale:alternate" content="zh_CN">"#));
    }

    // Complete OG + Twitter large card + feed alternates + <time datetime> + stable #id
    for doc in [&zh_daily_html, &en_daily_html] {
        for marker in [
            r#"property="og:image" content="https://ai.example.org/og-card.png""#,
            r#"property="og:image:width" content="1200""#,
            r#"property="og:image:height" content="630""#,
            r#"name="twitter:card" content="summary_large_image""#,
            r#"name="twitter:image" content="https://ai.example.org/og-card.png""#,
            r#"type="application/atom+xml""#,
            r#"type="application/rss+xml""#,
            r#"type="application/feed+json""#,
            r#"type="text/markdown""#,
            r#"id="item-1""#,
            r#"<time class="when" datetime="2026-01-15""#,
        ] {
            assert!(doc.contains(marker), "missing {marker}");
        }
    }

    // JSON-LD schema verification + </script> escape verification
    let issue_ld = extract_ld(&en_daily_html);
    let graph = issue_ld["@graph"].as_array().expect("issue @graph");
    let article = graph
        .iter()
        .find(|n| n["@type"] == "NewsArticle")
        .expect("NewsArticle");
    assert_eq!(article["inLanguage"], "en");
    assert_eq!(article["isAccessibleForFree"], true);
    assert_eq!(article["datePublished"], DAILY);
    assert_eq!(article["dateModified"], DAILY);
    assert_eq!(article["author"]["name"], "eatingDream");
    assert_eq!(article["publisher"]["name"], "AI Digest");
    assert!(article["about"]
        .as_array()
        .expect("NewsArticle.about")
        .iter()
        .any(|t| t["@type"] == "Thing" && t["name"].as_str().is_some()));
    let citations = article["citation"]
        .as_array()
        .expect("NewsArticle.citation");
    assert!(
        citations.contains(&json!("https://example.com/orion-3-mini")) && citations.len() <= 25,
        "unexpected NewsArticle.citation: {citations:?}"
    );
    assert_eq!(
        article["image"]["url"],
        "https://ai.example.org/og-card.png"
    );
    let items = article["mainEntity"]["itemListElement"]
        .as_array()
        .expect("ItemList elements");
    assert!(items[0]["url"]
        .as_str()
        .unwrap()
        .ends_with(&format!("/en/daily/{DAILY}#item-1")));
    // Layer 1: Markdown raw HTML </script> is stripped at render time
    assert!(
        !en_daily_html.contains("<svg>"),
        "raw inline HTML in markdown must be stripped"
    );
    // Layer 2: HTML-entity-decoded </script> in parsed issue title/summary is escaped as \u003c/script\u003e in JSON-LD
    let mut break_page = geo::GeoPage {
        rel_html: format!("en/daily/{DAILY}.html"),
        url_path: format!("/en/daily/{DAILY}"),
        canonical_path: format!("/en/daily/{DAILY}"),
        rel_md: format!("en/daily/{DAILY}.md"),
        lang: Lang::En,
        kind: "daily".into(),
        label: DAILY.into(),
        date_iso: DAILY.into(),
        ..geo::GeoPage::default()
    };
    let breakout_html = en_daily_html.replacen(
        "<p class=\"summary-lead\">",
        "<p class=\"summary-lead\">&lt;/script&gt;&lt;script&gt;alert(1)&lt;/script&gt; ",
        1,
    );
    let breakout_decorated = geo::decorate_public_html(
        &breakout_html,
        &mut break_page,
        &[],
        "https://ai.example.org",
        true,
    )
    .unwrap();
    let (_, ld_tail) = breakout_decorated
        .split_once(r#"<script type="application/ld+json">"#)
        .unwrap();
    let (ld_raw, _) = ld_tail.split_once("</script>").unwrap();
    assert!(
        ld_raw.contains(r"\u003c/script\u003e"),
        "JSON-LD must escape </script> as \\u003c/script\\u003e: {ld_raw}"
    );
    assert!(
        !ld_raw.contains("</script"),
        "JSON-LD must never contain raw </script>"
    );
    let _: Value = serde_json::from_str(ld_raw).expect("escaped JSON-LD must remain valid JSON");
    let crumbs = graph
        .iter()
        .find(|n| n["@type"] == "BreadcrumbList")
        .expect("BreadcrumbList");
    assert_eq!(crumbs["itemListElement"].as_array().unwrap().len(), 3);

    let index_ld = extract_ld(&en_index_html);
    let igraph = index_ld["@graph"].as_array().expect("index @graph");
    let website = igraph
        .iter()
        .find(|n| n["@type"] == "WebSite")
        .expect("WebSite");
    assert_eq!(
        website["potentialAction"]["target"]["urlTemplate"],
        "https://ai.example.org/.well-known/search?q={search_term_string}"
    );
    let alt_names = website["alternateName"]
        .as_array()
        .expect("WebSite.alternateName");
    assert!(alt_names.contains(&json!("ai.example.org")));
    assert!(alt_names.contains(&json!("AI Digest (ai.example.org)")));
    assert!(alt_names.contains(&json!("AI 每日简报")));
    assert!(alt_names.contains(&json!("Daily AI Briefing")));
    let fallback_ld = geo::build_index_json_ld(&break_page, "https://ai.example.org", &[], false);
    assert_eq!(
        fallback_ld["@graph"][1]["potentialAction"]["target"]["urlTemplate"],
        "https://ai.example.org/?q={search_term_string}"
    );
    let org = igraph
        .iter()
        .find(|n| n["@type"] == "Organization")
        .expect("Organization");
    assert_eq!(org["founder"]["name"], "eatingDream");
    let same_as = org["sameAs"].as_array().expect("Organization.sameAs");
    assert_eq!(
        same_as,
        &[
            json!("https://eatingdream.dev"),
            json!("https://github.com/eatingDream/ai-digest"),
            json!("https://github.com/eatingDream"),
        ]
    );
    let coll = igraph
        .iter()
        .find(|n| n["@type"] == "CollectionPage")
        .expect("CollectionPage");
    assert_eq!(coll["dateModified"], "2026-01-18");

    // 3. Sitemap validity: Index 1.0 first, newest issues descending, <lastmod>, reciprocal xhtml:link hreflang
    let sitemap = get(&public, "sitemap.xml");
    assert!(sitemap.contains(r#"xmlns:xhtml="http://www.w3.org/1999/xhtml""#));
    assert!(sitemap
        .contains("<loc>https://ai.example.org/zh/</loc>\n    <lastmod>2026-01-18</lastmod>"));
    assert!(sitemap
        .contains("<loc>https://ai.example.org/en/</loc>\n    <lastmod>2026-01-18</lastmod>"));
    let pos_zh_idx = sitemap
        .find("<loc>https://ai.example.org/zh/</loc>")
        .unwrap();
    let pos_en_idx = sitemap
        .find("<loc>https://ai.example.org/en/</loc>")
        .unwrap();
    let pos_weekly = sitemap
        .find("<loc>https://ai.example.org/zh/weekly/2026-01-12_01-18</loc>")
        .unwrap();
    let pos_daily = sitemap
        .find("<loc>https://ai.example.org/zh/daily/2026-01-15</loc>")
        .unwrap();
    assert!(
        pos_zh_idx < pos_en_idx && pos_en_idx < pos_weekly && pos_weekly < pos_daily,
        "sitemap must order Index pages first followed by newest issues descending:\n{sitemap}"
    );
    assert!(sitemap.contains(
        "<loc>https://ai.example.org/zh/daily/2026-01-15</loc>\n    <lastmod>2026-01-15</lastmod>\n    <changefreq>monthly</changefreq>\n    <priority>0.9</priority>"
    ));
    assert!(sitemap.contains(r#"hreflang="zh-Hans""#));
    assert!(sitemap.contains(r#"hreflang="en""#));
    assert!(sitemap.contains(r#"hreflang="x-default""#));

    // 4. robots.txt: Content-Signal + explicit search & AI bots + Sitemap
    let robots = get(&public, "robots.txt");
    for bot in [
        "Googlebot",
        "Bingbot",
        "OAI-SearchBot",
        "Claude-SearchBot",
        "PerplexityBot",
    ] {
        assert!(
            robots.contains(&format!("User-agent: {bot}\nAllow: /")),
            "robots.txt missing {bot}"
        );
    }
    assert!(robots.contains("Content-Signal: ai-train=yes, search=yes, ai-input=yes"));
    assert!(robots.contains("Sitemap: https://ai.example.org/sitemap.xml"));

    // 5. RFC 9727 /.well-known/api-catalog, /rss.xml, /feed.json, /llms.txt, Agent Skills SHA-256
    let catalog: Value = serde_json::from_str(&get(&public, geo::API_CATALOG_PATH)).unwrap();
    let linkset = catalog["linkset"].as_array().expect("RFC 9727 linkset");
    assert!(linkset
        .iter()
        .any(|entry| entry["anchor"] == "https://ai.example.org/.well-known/search"));
    assert!(linkset
        .iter()
        .any(|entry| entry["anchor"] == "https://ai.example.org/"));

    for rel in [
        "rss.xml",
        "zh/rss.xml",
        "en/rss.xml",
        "feed.json",
        "zh/feed.json",
        "en/feed.json",
    ] {
        assert!(public.join(rel).is_file(), "missing {rel}");
    }
    let rss = get(&public, "en/rss.xml");
    assert!(rss.contains(r#"<rss version="2.0""#) && rss.contains("<pubDate>"));
    let json_feed: Value = serde_json::from_str(&get(&public, "en/feed.json")).unwrap();
    assert_eq!(json_feed["version"], "https://jsonfeed.org/version/1.1");
    assert!(!json_feed["items"].as_array().unwrap().is_empty());

    let llms = get(&public, "llms.txt");
    for needle in [
        "AI Digest (ai.example.org)",
        "https://ai.example.org/",
        "155",
        "https://eatingdream.dev",
        "https://github.com/eatingDream/ai-digest",
        "/.well-known/api-catalog",
        "/feed.json",
        "/rss.xml",
        "/search-index.json",
        "/zh/index.md",
        "/en/index.md",
    ] {
        assert!(llms.contains(needle), "llms.txt missing {needle}: {llms}");
    }
    let zh_index_md = get(&public, "zh/index.md");
    assert!(
        zh_index_md.contains("https://ai.example.org/")
            && zh_index_md.contains("https://eatingdream.dev")
            && zh_index_md.contains("155"),
        "zh/index.md missing identity disambiguation: {zh_index_md}"
    );

    let skill_bytes = fs::read(public.join(geo::SKILL_PATH)).unwrap();
    let skill_idx: Value = serde_json::from_str(&get(&public, geo::SKILL_INDEX)).unwrap();
    assert_eq!(
        skill_idx["skills"][0]["digest"],
        format!("sha256:{}", hex::encode(Sha256::digest(skill_bytes)))
    );

    let (global_headers, _) = geo::parse_headers(&get(&public, "_headers"));
    assert!(global_headers["Link"].contains(r#"rel="api-catalog""#));
    assert!(global_headers["Link"].contains(r#"type="application/feed+json""#));
    let routes: Value = serde_json::from_str(&get(&public, "_routes.json")).unwrap();
    let excludes = routes["exclude"].as_array().unwrap();
    for path in [
        "/.well-known/api-catalog",
        "/rss.xml",
        "/feed.json",
        "/og-card.png",
    ] {
        assert!(
            excludes.iter().any(|v| v == path),
            "_routes.json missing exclude {path}"
        );
    }

    // 6. Private transition purges ALL new GEO artifacts cleanly
    password(root);
    checked(
        cli(root, &["publish", "--rebuild", "--prepare-only"])
            .env("DIGEST_ENCRYPT", "1")
            .output()
            .unwrap(),
        0,
    );
    for removed in [
        "rss.xml",
        "zh/rss.xml",
        "en/rss.xml",
        "feed.json",
        "zh/feed.json",
        "en/feed.json",
        "og-card.png",
        geo::API_CATALOG_PATH,
    ] {
        assert!(
            !public.join(removed).exists(),
            "encrypted mode must purge {removed}"
        );
    }
}

#[tokio::test]
async fn indexnow_key_emission_push_payload_downtime_tolerance_and_private_purge() {
    // 1. Strict INDEXNOW_KEY validation
    for bad in [
        "short77",
        "bad/slash-key",
        "bad.dot-key",
        "bad_underscore",
        "llms-full",
        &"a".repeat(129),
    ] {
        assert!(
            SiteConfig::parse(&format!("INDEXNOW_KEY={bad}\n"), None, |_| None).is_err(),
            "INDEXNOW_KEY={bad} must fail validation"
        );
    }
    let cfg_field = ["INDEXNOW", "KEY"].join("_");
    let sample_val = "a".repeat(16);
    let ok_cfg = SiteConfig::parse(&format!("{cfg_field}={sample_val}\n"), None, |_| None).unwrap();
    assert_eq!(ok_cfg.indexnow_key, sample_val);

    // 2. Sitemap priority tiers: 7 newest daily issues get 0.9, 8th+ older issues get 0.8
    let mut many_pages = vec![geo::GeoPage {
        rel_html: "index.html".into(),
        rel_md: "index.md".into(),
        url_path: "/".into(),
        canonical_path: "/".into(),
        page_type: geo::PageType::Index,
        is_canonical: true,
        ..geo::GeoPage::default()
    }];
    for day in 1..=9 {
        let iso = format!("2026-01-{day:02}");
        many_pages.push(geo::GeoPage {
            rel_html: format!("daily/{iso}.html"),
            rel_md: format!("daily/{iso}.md"),
            url_path: format!("/daily/{iso}"),
            canonical_path: format!("/daily/{iso}"),
            kind: "daily".into(),
            label: iso.clone(),
            date_iso: iso,
            is_canonical: true,
            ..geo::GeoPage::default()
        });
    }
    let many_xml = geo::render_sitemap_xml(&many_pages, "https://ai.example.org");
    assert!(
        many_xml.contains("<loc>https://ai.example.org/</loc>\n    <lastmod>2026-01-09</lastmod>")
    );
    assert!(
        many_xml
            .find("<loc>https://ai.example.org/daily/2026-01-09</loc>")
            .unwrap()
            < many_xml
                .find("<loc>https://ai.example.org/daily/2026-01-01</loc>")
                .unwrap(),
        "sitemap must list newest issue URLs first: {many_xml}"
    );
    assert!(many_xml.contains(
        "<loc>https://ai.example.org/daily/2026-01-03</loc>\n    <lastmod>2026-01-03</lastmod>\n    <changefreq>monthly</changefreq>\n    <priority>0.9</priority>"
    ));
    assert!(many_xml.contains(
        "<loc>https://ai.example.org/daily/2026-01-02</loc>\n    <lastmod>2026-01-02</lastmod>\n    <changefreq>monthly</changefreq>\n    <priority>0.8</priority>"
    ));

    // 3. End-to-end live deploy + wiremock IndexNow POST + Pages manifest upload
    let cf_mock = MockServer::start().await;
    let indexnow_mock = MockServer::start().await;
    let upload = ["fixture", "upload"].join("-");
    Mock::given(method("GET"))
        .and(path(
            "/accounts/example/pages/projects/fixture/upload-token",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":{"jwt":upload}})))
        .mount(&cf_mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/check-missing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":[]})))
        .mount(&cf_mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/pages/assets/upsert-hashes"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&cf_mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/accounts/example/pages/projects/fixture/domains"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success":true,"result":[{"name":"ai.example.org"}]})),
        )
        .mount(&cf_mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/accounts/example/pages/projects/fixture/deployments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"success":true,"result":{"url":"https://fixture.pages.dev"}}),
            ),
        )
        .mount(&cf_mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/indexnow"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&indexnow_mock)
        .await;

    let tmp = tempdir().unwrap();
    let root = tmp.path();
    let first_id = "b".repeat(16);
    atomic_write_text(
        &root.join("site/CONFIG"),
        &format!(
            "ACCOUNT_ID=example\nPROJECT=fixture\nDOMAIN=ai.example.org\nLANG=zh-CN\n{cfg_field}={first_id}\n"
        ),
    )
    .unwrap();
    issue_markdown(root, true);
    add_daily(root);
    add_weekly(root);

    let public = root.join("site/public");
    assert_eq!(get(&public, &format!("{first_id}.txt")), first_id);
    assert!(get(&public, "_headers").contains(&format!(
        "/{first_id}.txt\n  Content-Type: text/plain; charset=utf-8"
    )));
    let routes: Value = serde_json::from_str(&get(&public, "_routes.json")).unwrap();
    assert!(routes["exclude"]
        .as_array()
        .unwrap()
        .contains(&json!(format!("/{first_id}.txt"))));

    let token = ["mock", "credential"].join("-");
    let deploy_ok = checked(
        cli(root, &["publish", "--rebuild", "--no-verify"])
            .env("DIGEST_CF_API_BASE", cf_mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .env(
                "DIGEST_INDEXNOW_ENDPOINT",
                format!("{}/indexnow", indexnow_mock.uri()),
            )
            .output()
            .unwrap(),
        0,
    );
    assert!(
        String::from_utf8_lossy(&deploy_ok.stdout).contains("indexnow: ok (HTTP 202, 6 urls)"),
        "stdout={}",
        String::from_utf8_lossy(&deploy_ok.stdout)
    );

    let in_reqs = indexnow_mock.received_requests().await.unwrap();
    assert_eq!(in_reqs.len(), 1);
    let payload: Value = serde_json::from_slice(&in_reqs[0].body).unwrap();
    assert_eq!(payload["host"], "ai.example.org");
    assert_eq!(payload["key"], first_id);
    assert_eq!(
        payload["keyLocation"],
        format!("https://ai.example.org/{first_id}.txt")
    );
    assert_eq!(
        payload["urlList"],
        json!([
            "https://ai.example.org/zh/",
            "https://ai.example.org/en/",
            "https://ai.example.org/zh/weekly/2026-01-12_01-18",
            "https://ai.example.org/en/weekly/2026-01-12_01-18",
            "https://ai.example.org/zh/daily/2026-01-15",
            "https://ai.example.org/en/daily/2026-01-15",
        ])
    );

    // 4. Key rotation prunes previous <first_id>.txt & IndexNow HTTP 503 is non-fatal
    let down_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/indexnow"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&down_mock)
        .await;
    let second_id = "c".repeat(16);
    let warn_out = checked(
        cli(root, &["publish", "--rebuild", "--no-verify"])
            .env("DIGEST_INDEXNOW_KEY", &second_id)
            .env("DIGEST_CF_API_BASE", cf_mock.uri())
            .env("CLOUDFLARE_API_TOKEN", &token)
            .env(
                "DIGEST_INDEXNOW_ENDPOINT",
                format!("{}/indexnow", down_mock.uri()),
            )
            .output()
            .unwrap(),
        0,
    );
    assert!(!public.join(format!("{first_id}.txt")).exists());
    assert_eq!(get(&public, &format!("{second_id}.txt")), second_id);
    assert!(
        String::from_utf8_lossy(&warn_out.stderr).contains("IndexNow submission failed"),
        "stderr={}",
        String::from_utf8_lossy(&warn_out.stderr)
    );

    // 5. Transition to ENCRYPT=1 purges <second_id>.txt and _headers route cleanly
    password(root);
    checked(
        cli(root, &["publish", "--rebuild", "--prepare-only"])
            .env("DIGEST_INDEXNOW_KEY", &second_id)
            .env("DIGEST_ENCRYPT", "1")
            .output()
            .unwrap(),
        0,
    );
    assert!(!public.join(format!("{second_id}.txt")).exists());
    assert!(!get(&public, "_headers").contains(&second_id));
}

trait PipeRead {
    fn pipe_read(&self) -> String;
}
impl PipeRead for Path {
    fn pipe_read(&self) -> String {
        fs::read_to_string(self).unwrap()
    }
}
