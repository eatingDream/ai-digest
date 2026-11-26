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
<meta name="theme-color" content="#faf8f4">
<title>{{pw_title}}</title>
<script>
(function () {
  var t = localStorage.getItem("digest-theme");
  var m = document.querySelector('meta[name="theme-color"]');
  if (m && t === "dark") m.setAttribute("content", "#141311");
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
    let extra_src = orig_headers.replace(
        "'inline-speculation-rules'",
        "'inline-speculation-rules' https://evil.example",
    );
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
