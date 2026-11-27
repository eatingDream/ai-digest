use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use ai_digest::{
    assets,
    crypto::{self, Envelope, ScriptAllowlist, UnlockOptions},
    geo::{self, GeoPage, PageType},
    i18n::{self, Lang},
    ledger::{self, AddOptions},
    math::render_inline_math,
    model::{self, atomic_write_text, DigestPaths, LangAlt, LinkItem, SiteConfig, WorkerConfig},
    net::{self, HttpError, SafeHttpClient},
    parser::{self, parse_digest_html, parse_digest_md},
    render::{self, ArchiveIssue, NavOptions, RenderOptions},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{NaiveDate, Utc};
use html5gum::Token;
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use tempfile::tempdir;
use url::Url;

const DAILY_EN: &str = include_str!("../examples/sample-digest.en.md");
const DAILY_ZH: &str = include_str!("../examples/sample-digest.zh-CN.md");
const WEEKLY_EN: &str = include_str!("../examples/sample-weekly.en.md");
const WEEKLY_ZH: &str = include_str!("../examples/sample-weekly.zh-CN.md");
const LEGACY_TEMPLATE: &str = r##"<!DOCTYPE html>
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

const LEGACY: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>AI Digest</title><meta name="digest-title" content="AI Weekly · 01/12–01/18"><style>.story{}</style>
<!-- digest-nav-css:start --><style>.issue-nav{display:flex}</style><!-- digest-nav-css:end -->
<!-- digest-meta:start --><meta name="robots" content="noindex,nofollow"><!-- digest-meta:end -->
<!-- digest-perf:start --><link rel="prefetch" href="/old"><!-- digest-perf:end -->
</head><body><header class="mast">
<div><h1><a class="np-link" href="./" data-brand-home><span class="ai">AI</span> Digest</a></h1>
<span class="edition">Weekly</span>
<span class="mast-meta"><span>Jan 12 – Jan 18, 2026</span> · <span>2 items · 1 min</span><span class="motto">Models &amp; Research</span></span>
</div><!-- digest-nav-slot --><!-- digest-nav:top:start --><nav>old</nav><!-- digest-nav:top:end -->
<!-- digest-lang-btn:start --><a href="/old-language">old</a><!-- digest-lang-btn:end -->
</header><main><section class="front"><ol class="must-list">
<li><a href="#item-1"><span class="must-n">1</span><span class="minw"><span class="must-title">Acme model (1/15)</span><span class="must-dek">Release &amp; weights</span></span></a></li>
</ol></section><section class="sec" id="sec-models"><h2>Models</h2>
<article class="story" id="item-1" data-tags="Open weights|Research">
<div class="story-meta"><span class="source">Acme Labs</span><span class="badge">Open weights</span><span class="when">1.15</span><button><svg><path d="x"/></svg></button></div>
<div class="story-head"><h3><span class="story-no">1.</span> Acme model</h3></div>
<div class="dek">An example release with <em>open</em> weights.</div>
<div class="look"><span class="look-k">Why it matters</span> An evaluation note.</div>
<div class="detail"><span class="detail-k">Details</span><p>First paragraph.</p>
<p>Second paragraph.</p></div>
<div class="extra"><span class="extra-k">Follow-up</span>A follow-up.</div><div class="extra"><span class="extra-k">More</span>Extra text.</div>
<div class="story-links"><a href="https://example.com/?q=1&amp;p=2"><svg><path d="x"/></svg>Blog</a></div>
</article></section><section class="sec" id="sec-papers"><h2>Papers</h2>
<article class="story" id="item-2" data-tags="Research"><h3><span class="story-no">2.</span> Paper</h3><div class="dek">Abstract.</div></article>
</section><div class="end">End of issue</div><!-- digest-nav:bottom:start -->old<!-- digest-nav:bottom:end -->
</main><!-- digest-lock:start --><script>/* old lock */</script><!-- digest-lock:end --></body></html>"##;

const MATH_ISSUE: &str = r#"**AI Daily · 1/15**
## Papers
### 1. An example about quantities
`Example · 1/15 · Math / Evals｜Math`
**In brief:** The bound is $L_N[1/3, O(1)]$.
**Why it matters:** Next, \(\frac{a}{b}\).
**Details:** `$x$` is code, [URL](https://example.com/$x$), and \(x_{y^2}\) is math.
### 2. Secondary headline
**In brief:** Another result.
"#;

fn day(value: &str) -> NaiveDate {
    value.parse().unwrap()
}

fn fixture(md: &str, lang: Lang) -> model::Digest {
    parse_digest_md(md, lang, Some(2026)).unwrap()
}

fn starts(html: &str) -> impl Iterator<Item = html5gum::StartTag> + '_ {
    model::html_tokens(html).filter_map(|token| match token {
        Token::StartTag(tag) => Some(tag),
        _ => None,
    })
}

fn attr<'a>(tag: &'a html5gum::StartTag, name: &[u8]) -> Option<&'a str> {
    tag.attributes
        .get(name)
        .map(|value| std::str::from_utf8(&value.0).unwrap())
}

#[test]
fn translation_keys_and_placeholders_match() {
    let format = Regex::new(r"\{([A-Za-z0-9_]+)(?::[^}]+)?\}").unwrap();
    let cjk = Regex::new(r"[\p{Han}\p{Hiragana}\p{Katakana}\p{Hangul}]").unwrap();
    for (keys, lookup, expected) in [
        (
            i18n::all_string_keys().collect::<Vec<_>>(),
            i18n::string as fn(Lang, &str) -> Option<&'static str>,
            103,
        ),
        (
            i18n::all_fetch_keys().collect::<Vec<_>>(),
            i18n::fetch_string as fn(Lang, &str) -> Option<&'static str>,
            103,
        ),
    ] {
        assert_eq!(keys.len(), expected);
        assert_eq!(keys.len(), keys.iter().collect::<HashSet<_>>().len());
        for key in keys {
            let en = lookup(Lang::En, key).unwrap();
            let zh = lookup(Lang::ZhCn, key).unwrap();
            let fields = |text: &str| {
                format
                    .captures_iter(text)
                    .map(|c| c[1].to_owned())
                    .collect::<BTreeSet<_>>()
            };
            assert!(!cjk.is_match(en), "{key}");
            if key != "month_label" {
                assert_eq!(fields(en), fields(zh), "{key}");
            }
        }
    }
    assert!(i18n::string(Lang::En, "pw_opening").is_some());
    assert!(i18n::string(Lang::En, "pw_unlocking").is_none());
    assert_eq!(
        ai_digest::version::version_string(),
        format!("ai-digest {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn language_resolution_and_dates_follow_reference() {
    for value in [
        "zh",
        " zh_cn ",
        "zh-Hans",
        "CN",
        "chinese",
        "中文",
        "简体中文",
    ] {
        assert_eq!(i18n::normalize_lang(value).unwrap(), Lang::ZhCn);
    }
    for value in ["", "en", "EN-GB", "English", "en-US"] {
        assert_eq!(i18n::normalize_lang(value).unwrap(), Lang::En);
    }
    assert!(!i18n::is_valid_lang("en-ZZ"));
    assert_eq!(
        i18n::resolve_lang(Some("zh"), Some("en")).unwrap(),
        Lang::ZhCn
    );
    assert_eq!(
        i18n::resolve_lang_with_env(Some("  "), Some("en-us"), Some("zh")).unwrap(),
        Lang::En
    );
    assert_eq!(
        i18n::resolve_lang_with_env(None, None, Some("zh")).unwrap(),
        Lang::ZhCn
    );
    assert_eq!(
        i18n::resolve_lang_with_env(None, None, None).unwrap(),
        Lang::En
    );
    let a = day("2026-01-15");
    let b = day("2026-01-18");
    let c = day("2027-01-02");
    assert_eq!(i18n::date_long(Lang::En, a), "January 15, 2026");
    assert_eq!(i18n::date_long(Lang::ZhCn, a), "2026 年 1 月 15 日");
    assert_eq!(i18n::weekday(Lang::En, a), "Thursday");
    assert_eq!(i18n::weekday(Lang::ZhCn, a), "星期四");
    assert_eq!(i18n::date_range(Lang::En, a, b), "Jan 15 – Jan 18, 2026");
    assert_eq!(
        i18n::date_range(Lang::En, a, c),
        "Jan 15, 2026 – Jan 2, 2027"
    );
    assert_eq!(
        i18n::date_range(Lang::ZhCn, a, c),
        "2026 年 1 月 15 日 — 2027 年 1 月 2 日"
    );
    assert_eq!(i18n::year_week(Lang::En, 2026, 3), "2026 · Week 3");
    assert_eq!(i18n::year_week(Lang::ZhCn, 2026, 3), "2026 年第 3 周");
    assert_eq!(i18n::month_label(Lang::En, 2026, 1), "January 2026");
    assert_eq!(i18n::month_label(Lang::ZhCn, 2026, 1), "2026年1月");
    assert_eq!(i18n::count_items(Lang::En, 1), "1 item");
    assert_eq!(i18n::count_items(Lang::ZhCn, 12), "12 条");
    assert_eq!(i18n::cnt_paren(Lang::En, 12), " (12)");
    assert_eq!(i18n::cnt_paren(Lang::ZhCn, 12), "（12）");
    assert_eq!(i18n::count_issues(Lang::En, 2), "2 issues");
    assert_eq!(i18n::issue_unit(Lang::ZhCn, 1), "期");
    let ids: BTreeSet<_> = i18n::SECTION_IDS.iter().map(|(_, id)| *id).collect();
    assert_eq!(
        ids,
        BTreeSet::from([
            "sec-models",
            "sec-papers",
            "sec-tools",
            "sec-infra",
            "sec-policy",
            "sec-industry",
            "sec-voices",
            "sec-evals"
        ])
    );
}

#[test]
fn math_supports_the_editorial_subset() {
    let output = render_inline_math(
        r"$L_N[1/3, O(1)]$ \(x_{y^2}=\frac{\alpha}{\Omega}+\sqrt{x}\) $\mathbb{R}\mathcal{F}\text{ prose }\,\sin\;x\leq\infty$",
    );
    for fragment in [
        r#"aria-label="L_N[1/3, O(1)]""#,
        "<i>L</i><sub><i>N</i></sub>[1/3, <i>O</i>(1)]",
        "<i>x</i><sub><i>y</i><sup>2</sup></sub>",
        r#"<span class="frac"><i>α</i>⁄Ω</span>"#,
        r#"√<span class="sqrt"><i>x</i></span>"#,
        "ℝℱ prose  sin <i>x</i>≤∞",
        "math math-wrap",
    ] {
        assert!(output.contains(fragment), "{fragment}: {output}");
    }
    let ops = render_inline_math(
        r"$\sum\prod\partial\nabla\ldots\cdots\mid\Pr\max\min\log\ln\lim\poly\det\cos\exp\to\ne\sim\approx\pm\cdot\times\geq\in$",
    );
    assert!(ops.contains("∑∏∂∇…⋯|Prmaxminloglnlimpolydetcosexp→≠∼≈±·×≥∈"));
}

#[test]
fn math_keeps_currency_and_escapes_success_and_fallback() {
    let prose = r"From $100–$200 or $5B, plus \$7 and $$literal$$.";
    assert_eq!(render_inline_math(prose), prose);
    let prose_with_math = "From $100–$200 or $5B to $x^2$.";
    let mixed = render_inline_math(prose_with_math);
    assert!(mixed.starts_with("From $100–$200 or $5B to "));
    assert!(mixed.contains("<i>x</i><sup>2</sup>"));
    for invalid in [
        r"$\unknown{x}$",
        "$x_{y^{z^2}}$",
        "$x_{y$",
        "$x_{}$",
        r"$\text{\unknown}$",
    ] {
        assert_eq!(
            render_inline_math(invalid),
            format!(r#"<code class="math-raw">{invalid}</code>"#)
        );
    }
    let escaped = render_inline_math(r#"$\text{<b a="x">&} + x<y$ $\unknown{<script>}$"#);
    assert!(escaped.contains(r#"aria-label="\text{&lt;b a=&quot;x&quot;&gt;&amp;} + x&lt;y""#));
    assert!(escaped.contains("&lt;b a=&quot;x&quot;&gt;&amp; + <i>x</i>&lt;<i>y</i>"));
    assert!(escaped.contains(r#"<code class="math-raw">$\unknown{&lt;script&gt;}$</code>"#));
    assert!(!escaped.contains("<script>"));
}

#[test]
fn validates_site_paths_configuration_and_source_schema() {
    let root = Path::new("fixture-root");
    let isolated = model::DigestPaths::with_env(root, |key| match key {
        "DIGEST_ROOT" => Some("isolated".into()),
        "LEDGER_PATH" => Some("ledger-parent/custom.json".into()),
        "DIGEST_TZ" => Some("Asia/Taipei".into()),
        _ => None,
    })
    .unwrap();
    assert_eq!(isolated.state, Path::new("isolated/state"));
    assert_eq!(
        isolated.ledger_backup,
        Path::new("ledger-parent/logs/ledger-backup")
    );
    assert_eq!(isolated.timezone, chrono_tz::Asia::Taipei);
    assert_eq!(
        model::DigestPaths::with_env(root, |_| None).unwrap().state,
        root.join("fetch/state")
    );
    let cfg = SiteConfig::parse(
        "SLUG=abcdefgh\nLANG=zh\nGEO_WORKER=off\nCONTENT_SIGNAL=search=no\nCF_ACCOUNT_ID=example\n",
        Some("en"),
        |key| match key {
            "SITE_SLUG" => Some(String::new()),
            "CF_PROJECT" => Some("first".into()),
            "CF_PAGES_PROJECT" => Some("last".into()),
            "DIGEST_GEO_AI_TRAIN" => Some("no".into()),
            "DIGEST_LANG_ALT" => Some("zh-CN:/zh/".into()),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(cfg.slug, "");
    assert_eq!(cfg.project, "last");
    assert_eq!(cfg.lang, Lang::En);
    assert_eq!(cfg.lang_alt.unwrap().base, "/zh/");
    assert!(!cfg.geo_worker && !cfg.geo_ai_train);
    assert_eq!(cfg.content_signal, "ai-train=no, search=no, ai-input=yes");
    for slug in [
        "short",
        ".hiddenxx",
        "invalid/path",
        "llms-full.txt",
        "_routes.json",
    ] {
        assert!(model::validate_slug(slug).is_err());
    }
    assert!(SiteConfig::parse("DOMAIN=<DOMAIN>", None, |_| None).is_err());
    assert!(SiteConfig::parse("LANG_ALT=//example.com/", None, |_| None).is_err());
    assert_eq!(
        model::canon_url("http://WWW.M.Example.com/a/?id=3&utm_other=2&empty=#part"),
        "https://example.com/a?id=3"
    );
    assert_eq!(
        model::canon_url("https://huggingface.co/papers/2601.12345"),
        "arxiv:2601.12345"
    );
    assert_eq!(
        model::norm_title(" AI 甲乙 / _ Example! "),
        "ai甲乙_example"
    );
    assert_eq!(model::clean_text("one\n\t two"), "one two");
    let cfg: model::SourcesConfig = toml::from_str(include_str!("../fetch/sources.toml")).unwrap();
    assert!(cfg.source.len() >= 150);
    assert_eq!(
        cfg.source.len(),
        cfg.source
            .iter()
            .map(|s| &s.id)
            .collect::<BTreeSet<_>>()
            .len()
    );
}

#[test]
fn markdown_raw_html_images_and_unsafe_links_are_dropped() {
    for url in [
        "javascript:alert(1)",
        "java\tscript:alert(1)",
        "vbscript:msgbox(1)",
        "data:text/html,<svg/>",
        "//example.com/x",
        "/\\example.com/x",
        "\\\\example.com/x",
        "http://",
        "https://example.com/\0x",
        "https://example.com/\r\nx",
        "https://example.com/&#x0a;x",
        "jav&#x09;ascript:alert(1)",
    ] {
        assert!(!model::is_safe_url(url), "{url:?}");
    }
    for url in [
        "https://example.com/a?b=1",
        "http://example.com/",
        "#item-1",
        "/zh/daily/2026-01-15",
        "issue.md",
    ] {
        assert!(model::is_safe_url(url), "{url}");
    }
    let md = concat!(
        "<script>bad()</script>\n\n",
        "Hello <img src=x onerror=bad()> ![alt](https://example.com/x.png) ",
        "<fixture",
        "@",
        "example.com> [unsafe](javascript:bad) ",
        "[valid](https://example.com/x \"quoted label\") [internal](#item-1)"
    );
    let output = render::render_markdown(md, false);
    for forbidden in [
        "<script",
        "<img",
        "bad()",
        "onerror=",
        "mailto:",
        "javascript:",
        "@example.com",
    ] {
        assert!(!output.contains(forbidden), "{forbidden}: {output}");
    }
    assert!(output.contains("unsafe"));
    assert!(output.contains(r#"<a href="https://example.com/x" title="quoted label" target="_blank" rel="noopener noreferrer nofollow ugc">valid</a>"#));
    assert!(output.contains(r##"<a href="#item-1">internal</a>"##));
    let mut digest = fixture(MATH_ISSUE, Lang::En);
    if let model::SectionEntry::Item(it) = &mut digest.sections[0].entries[0] {
        it.links.push(LinkItem {
            label: "<link>".into(),
            url: "javascript:bad".into(),
        });
    }
    let output = render::render_digest_html(&digest, Lang::En, &RenderOptions::default());
    assert!(output.contains("<span>&lt;link&gt;</span>"));
    for tag in starts(&output) {
        assert!(!tag
            .attributes
            .keys()
            .any(|name| name.0 == b"style" || name.0.starts_with(b"on")));
    }
}

#[tokio::test]
async fn ssrf_rejects_literals_and_local_hostnames() {
    let blocked = [
        "http://localhost/",
        "http://localhost./",
        "http://foo.localhost/",
        "http://foo.local/",
        "https://foo.internal/",
        "http://127.1/",
        "http://0.0.0.0/",
        "http://10.0.0.1/",
        "http://172.16.1.2/",
        "http://192.168.0.1/",
        "http://169.254.169.254/",
        "http://100.64.0.1/",
        "http://192.0.2.1/",
        "http://198.18.0.1/",
        "http://203.0.113.1/",
        "http://224.0.0.1/",
        "http://255.255.255.255/",
        "http://[::]/",
        "http://[::1]/",
        "http://[fc00::1]/",
        "http://[fe80::1]/",
        "http://[ff02::1]/",
        "http://[2001:db8::1]/",
        "http://[::ffff:127.0.0.1]/",
        "http://[::ffff:169.254.169.254]/",
    ];
    let client = SafeHttpClient::default();
    for target in blocked {
        let target = Url::parse(target).unwrap();
        assert!(
            matches!(client.get(&target).await, Err(HttpError::NonPublic)),
            "{target}"
        );
    }
    assert!(net::validate_target(&Url::parse("ftp://example.com/").unwrap()).is_err());
    assert!(net::validate_target(&Url::parse("http://1.1.1.1/").unwrap()).is_ok());
    assert!(net::validate_target(&Url::parse("http://[2606:4700::1111]/").unwrap()).is_ok());
}

#[test]
fn markdown_parses_all_examples_and_mixed_contract_edges() {
    for (md, lang, weekly, sections, items) in [
        (DAILY_EN, Lang::En, false, 7, 10),
        (DAILY_ZH, Lang::ZhCn, false, 7, 10),
        (WEEKLY_EN, Lang::En, true, 3, 3),
        (WEEKLY_ZH, Lang::ZhCn, true, 3, 3),
    ] {
        let digest = fixture(md, lang);
        assert_eq!(digest.weekly, weekly);
        assert_eq!(digest.sections.len(), sections);
        assert_eq!(digest.items().count(), items);
        assert_eq!(digest.sections[0].id, "sec-models");
        assert_eq!(digest.sections[1].id, "sec-papers");
        assert_eq!(digest.items().next().unwrap().when, "1.15");
        assert!(!digest.toc_groups.is_empty());
        let html = render::render_digest_html(&digest, lang, &RenderOptions::default());
        assert_eq!(
            starts(&html)
                .filter(|tag| attr(tag, b"class").is_some_and(|classes| classes
                    .split_ascii_whitespace()
                    .any(|class| class == "story--lead")))
                .count(),
            1
        );
        assert!(html.contains("<header class=\"folio issue-head\">"));
        assert!(html.contains("<nav class=\"dock\""));
        assert!(html.contains("<!-- digest-nav-slot -->"));
        assert!(!html.contains("lang-pane lang-"));
        let parsed = parse_digest_html(&html).unwrap();
        assert_eq!(parsed.items_count, items);
        if weekly {
            assert_eq!(digest.must.len(), 3);
            assert_eq!(digest.themes.len(), 2);
            assert_eq!(parsed.must.len(), 3);
            assert!(!digest.items().next().unwrap().followup.is_empty());
        }
    }
    let md = "## **AI Weekly · 12/28–01/03**\r\n\r\n#### Subheading\r\n### 🆕 3–4. Story\r\n`A · 12/29 · X／Y、Z｜X`\r\n**一句话：** Intro\r\n**Details:** Detail\r\n## Dropped this week\r\n### 5. Ignore";
    let digest = fixture(md, Lang::En);
    assert_eq!(digest.dates, [day("2026-12-28"), day("2027-01-03")]);
    let item = digest.items().next().unwrap();
    assert_eq!(item.id, "item-3-4");
    assert_eq!(item.tags, ["X", "Y", "Z"]);
    assert!(item.is_new);
    assert_eq!(
        (item.dek.as_str(), item.detail.as_str()),
        ("Intro", "Detail")
    );
    assert_eq!(digest.items().count(), 1);
    assert!(parser::period_dates("2/29", 2026).is_empty());
    assert!(parse_digest_md("## Section\nSome text", Lang::En, None).is_err());

    // Two-dot unified brand headers preserve weekly detection and period dates
    let zh_w = fixture(
        "**AI Digest · 周报 · 11/09–11/15**\n\n## 模型\n### 1. A\n`L · 11/12 · T`\n**一句话：** Hi",
        Lang::ZhCn,
    );
    assert!(zh_w.weekly);
    assert_eq!(zh_w.title, "AI Digest · 周报");
    assert_eq!(zh_w.period, "11/09–11/15");
    assert_eq!(zh_w.dates, [day("2026-11-09"), day("2026-11-15")]);
    let zh_d = fixture(
        "**AI Digest · 日报 · 11/26**\n\n## 模型\n### 1. A\n`L · 11/26 · T`\n**一句话：** Hi",
        Lang::ZhCn,
    );
    assert!(!zh_d.weekly);
    assert_eq!(zh_d.title, "AI Digest · 日报");
    assert_eq!(zh_d.period, "11/26");
    assert_eq!(zh_d.dates, [day("2026-11-26")]);
    let en_w = fixture("**AI Digest · Weekly · 11/09–11/15**\n\n## Models\n### 1. A\n`L · 11/12 · T`\n**TL;DR:** Hi\n\n🔗 [Don't Train It 论文](https://arxiv.org/abs/2611.28483) · [SELFWRITE 论文](https://arxiv.org/abs/2611.29513)", Lang::En);
    assert!(en_w.weekly);
    assert_eq!(en_w.dates, [day("2026-11-09"), day("2026-11-15")]);
    assert_eq!(en_w.items().next().unwrap().dek, "Hi");
    let en_html = render::render_digest_html(&en_w, Lang::En, &RenderOptions::default());
    // Mobile sticky filter strip must be a direct sibling after </header>, not trapped inside <header class="folio">
    let folio_end = en_html
        .find("</nav></header>")
        .expect("folio closing header");
    let strip_pos = en_html
        .find(r#"<div class="mobile-filter-strip""#)
        .expect("mobile filter strip");
    assert!(
        strip_pos > folio_end,
        "mobile-filter-strip must sit outside </header> to stay sticky across scroll"
    );
    // Same-host links in footer.sources must disambiguate instead of repeating arxiv.org
    assert!(
        en_html.contains("Don&#x27;t Train It") && en_html.contains("SELFWRITE"),
        "duplicate same-host source links must show disambiguated labels: {en_html}"
    );
}

#[test]
fn legacy_html_preserves_titles_tags_metadata_and_empty_must_anchor() {
    let parsed = parse_digest_html(LEGACY).unwrap();
    assert_eq!(parsed.title, "AI Weekly · 01/12–01/18");
    assert_eq!(parsed.edition, "Weekly");
    assert_eq!(
        parsed.meta_spans,
        ["Jan 12 – Jan 18, 2026", "2 items · 1 min"]
    );
    assert_eq!(parsed.motto, "Models & Research");
    assert_eq!(parsed.must[0].title, "Acme model (1/15)");
    assert_eq!(parsed.must[0].anchor, "");
    assert_eq!(parsed.must[0].dek, "Release & weights");
    assert_eq!(parsed.sections.len(), 2);
    assert_eq!(parsed.items_count, 2);
    let story = &parsed.items[0];
    assert_eq!(story.number, "1");
    assert_eq!(story.source, "Acme Labs");
    assert_eq!(story.when, "1.15");
    assert_eq!(story.look, "An evaluation note.");
    assert_eq!(story.detail, "First paragraph. Second paragraph.");
    assert_eq!(story.extra, "A follow-up. Extra text.");
    assert_eq!(
        story.links[0],
        ("Blog".into(), "https://example.com/?q=1&p=2".into())
    );
    assert_eq!(parsed.top_tags, ["Research", "Open weights"]);
}

fn historical_files(root: &Path) -> Vec<PathBuf> {
    model::files_under(root)
        .unwrap()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|s| s == "html"))
        .collect()
}

#[test]
fn optional_production_html_matches_legacy_parser_field_by_field() {
    let Some(root) = std::env::var_os("AI_DIGEST_PROD_SOURCES_DIR").map(PathBuf::from) else {
        return;
    };
    let files = historical_files(&root);
    assert_eq!(files.len(), 26, "expected the v0.9.0 source set");
    let mut counts = [0; 3];
    for (i, file) in files.iter().enumerate() {
        let lang = if file.strip_prefix(&root).unwrap().starts_with("en") {
            Lang::En
        } else {
            Lang::ZhCn
        };
        let html = fs::read_to_string(file).unwrap();
        let parsed = parser::parse_digest_html_with_lang(&html, Some(lang)).unwrap();
        assert!(!parsed.title.is_empty(), "historical file {i} empty title");
        assert!(
            !parsed.sections.is_empty(),
            "historical file {i} empty sections"
        );
        assert!(!parsed.items.is_empty(), "historical file {i} empty items");
        assert_eq!(parsed.items_count, parsed.items.len());
        counts[0] += parsed.sections.len();
        counts[1] += parsed.items.len();
        counts[2] += parsed.must.len();
        assert!(parsed.must.iter().all(|m| m.anchor.is_empty()));
    }
    assert_eq!(counts, [148, 698, 14]);
}

#[test]
fn opus_math_is_captured_once_and_primary_pane_is_selected() {
    let digest = fixture(MATH_ISSUE, Lang::En);
    let html = render::render_digest_html(&digest, Lang::En, &RenderOptions::default());
    assert!(html.contains(r#"aria-label="L_N[1/3, O(1)]""#));
    assert!(html.contains(r#"class="frac""#), "{html}");
    assert!(html.contains("<code>$x$</code>"));
    assert!(html.contains(r#"href="https://example.com/$x$""#));
    let parsed = parse_digest_html(&html).unwrap();
    assert_eq!(parsed.items[0].dek, "The bound is $L_N[1/3, O(1)]$.");
    assert_eq!(parsed.items[0].when, "1.15");
    assert_eq!(parsed.items[0].look, r"Next, $\frac{a}{b}$.");
    let mut page = issue_page(Lang::En);
    page.parsed = Some(parsed.clone());
    let ld = geo::build_issue_json_ld(&parsed, &page, "https://example.com");
    let mirror = geo::render_issue_markdown(&parsed, &page, "https://example.com", true, Some(&ld));
    assert_eq!(mirror.matches("$L_N[1/3, O(1)]$").count(), 1);
    assert!(!mirror.contains("LN[1/3, O(1)]"));
    let zh = fixture(WEEKLY_ZH, Lang::ZhCn);
    let en = fixture(MATH_ISSUE, Lang::En);
    let bilingual = render::render_bilingual_html(&zh, &en, &RenderOptions::default());
    let parsed = parse_digest_html(&bilingual).unwrap();
    assert_eq!(parsed.items_count, 3);
    assert_eq!(parsed.lang, Lang::ZhCn);
    assert_eq!(parsed.must[0].anchor, "#item-1");
    assert!(!parsed
        .items
        .iter()
        .any(|it| it.title.contains("quantities")));
}

#[test]
fn bilingual_prefixes_all_ids_and_local_references() {
    let zh = fixture(WEEKLY_ZH, Lang::ZhCn);
    let en = fixture(WEEKLY_EN, Lang::En);
    let html = render::render_bilingual_html(&zh, &en, &RenderOptions::default());
    let (_, pane) = html
        .split_once(r#"<div class="lang-pane lang-en""#)
        .unwrap();
    let ids: Vec<_> = starts(&html)
        .filter_map(|tag| attr(&tag, b"id").map(str::to_owned))
        .collect();
    let all: HashSet<_> = ids.iter().map(String::as_str).collect();
    assert_eq!(all.len(), ids.len());
    for tag in starts(&format!("<div class=\"lang-pane lang-en\"{pane}")) {
        for key in [
            b"id".as_slice(),
            b"href",
            b"aria-controls",
            b"data-copy-anchor",
        ] {
            let Some(value) = attr(&tag, key) else {
                continue;
            };
            let id = if key == b"href" {
                let Some(id) = value.strip_prefix('#') else {
                    continue;
                };
                id
            } else {
                value
            };
            assert!(id.starts_with("en-"), "English pane reference {id}");
            assert!(all.contains(id), "missing English pane target {id}");
        }
    }
}

#[test]
fn navigation_publish_is_idempotent_for_both_html_generations() {
    let opts = NavOptions {
        previous: Some(LinkItem {
            label: "Jan 14".into(),
            url: "/daily/2026-01-14".into(),
        }),
        home: Some(LinkItem {
            label: String::new(),
            url: "/".into(),
        }),
        lang_alt: Some(LangAlt {
            lang: Lang::ZhCn,
            base: "/zh/".into(),
        }),
        noindex: true,
        ..NavOptions::default()
    };
    let new = render::render_digest_html(
        &fixture(DAILY_EN, Lang::En),
        Lang::En,
        &RenderOptions::default(),
    );
    for raw in [LEGACY, &new] {
        let once = render::inject_nav(raw, &opts);
        assert_eq!(once, render::inject_nav(&once, &opts));
        for marker in ["digest-nav:top", "digest-nav:bottom", "digest-meta"] {
            assert_eq!(once.matches(&format!("<!-- {marker}:start -->")).count(), 1);
        }
        assert_eq!(once.matches("<!-- digest-nav-slot -->").count(), 1);
        assert_eq!(once.matches(r#"rel="prev" data-nav="prev""#).count(), 1);
        assert!(!once.contains("digest-nav-css:start"));
        assert!(!once.contains("digest-perf:start"));
        assert!(!once.contains("digest-lock:start"));
    }
    let bilingual = render::render_bilingual_html(
        &fixture(WEEKLY_ZH, Lang::ZhCn),
        &fixture(WEEKLY_EN, Lang::En),
        &RenderOptions::default(),
    );
    let once = render::inject_nav(&bilingual, &opts);
    assert_eq!(once, render::inject_nav(&once, &opts));
    assert_eq!(once.matches("digest-nav:top:start").count(), 2);
    assert_eq!(once.matches("digest-nav:bottom:start").count(), 2);
}

fn archive() -> String {
    let row = ArchiveIssue {
        date: day("2026-01-15"),
        weekly: false,
        href: "/daily/2026-01-15".into(),
        title: "Daily example".into(),
        headline: "Acme benchmark".into(),
        tags: vec!["Evals".into()],
        item_count: 10,
    };
    let weekly = ArchiveIssue {
        weekly: true,
        date: day("2026-01-18"),
        href: "/weekly/2026-01-12_01-18".into(),
        ..row.clone()
    };
    render::render_archive_html(&[row, weekly], Lang::En, &RenderOptions::default())
}

#[test]
fn archive_week_summary_is_text_only_and_404_has_three_links() {
    let html = archive();
    let summary = html
        .split_once("<summary>")
        .unwrap()
        .1
        .split_once("</summary>")
        .unwrap()
        .0;
    assert!(!summary.contains('<'));
    assert!(summary.contains("W03"));
    assert!(html.contains(r#"<details class="grp" data-week="2026-W03" open>"#));
    assert!(
        html.find(r#"<li class="archive-item" data-kind="weekly""#)
            .unwrap()
            < html
                .find(r#"<li class="archive-item" data-kind="daily""#)
                .unwrap()
    );
    let nf = render::render_404_html();
    assert!(nf.contains(r#"<main class="nf">"#));
    assert!(nf.contains(r#"content="noindex,nofollow""#));
    let links: Vec<_> = starts(&nf)
        .filter(|t| t.name.0 == b"a")
        .map(|tag| attr(&tag, b"href").unwrap().to_owned())
        .collect();
    assert_eq!(links, ["/latest", "/weekly/latest", "/"]);
}

const PASS: &str = "  合成测试口令 ordinary words  ";
const SALT: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const NONCE: [u8; 12] = [23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34];
const VECTOR_PLAIN: &str = "<h1>Example briefing 甲</h1>\n";
const NODE_V1: &str = "ZJGJY6kI/8Xiac69339ICj4T0ajSq/fwr6VR7asap2LiIabd9MBe6ExW2O6Idw==";
const NODE_V2: &str =
    "R3KwXexwnqiQFhhUjblSwhC2kge7GStoWcZ0F1lbq1LFTc8Q1h4fv/ZqgoL5+5YvAkSPP2BJqSOXSt+dCFHog7Ay";

fn vector_envelope(version: u8, ct: &str) -> Envelope {
    Envelope {
        v: version,
        z: (version == 2).then(|| "gzip".into()),
        iter: 600_000,
        salt: STANDARD.encode(SALT),
        iv: STANDARD.encode(NONCE),
        ct: ct.into(),
    }
}

#[test]
fn fixed_cross_runtime_vectors_cover_crypto_and_gzip_header() {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    let key = crypto::derive_key(PASS, &SALT);
    // Generated with Node's crypto and zlib APIs independently of the Rust code.
    assert_eq!(
        hex::encode(key),
        "46cc6d5682d2ee4ef2fa5976d0a6171ef6d3a2d94d4def7d974a5bfaa00ffd8f"
    );
    for (version, ct) in [(1, NODE_V1), (2, NODE_V2)] {
        assert_eq!(
            crypto::decrypt_envelope(&vector_envelope(version, ct), PASS).unwrap(),
            VECTOR_PLAIN.as_bytes()
        );
    }
    let data = VECTOR_PLAIN.as_bytes();
    let rust = crypto::encrypt_payload_with_nonce(data, &key, &SALT, NONCE).unwrap();
    assert_eq!(
        rust,
        crypto::encrypt_payload_with_nonce(data, &key, &SALT, NONCE).unwrap()
    );
    assert_eq!(
        crypto::decrypt_envelope_with_key(&rust, &key).unwrap(),
        data
    );
    let packed = Aes256Gcm::new((&key).into())
        .decrypt(
            &Nonce::from(NONCE),
            STANDARD.decode(&rust.ct).unwrap().as_slice(),
        )
        .unwrap();
    assert_eq!(&packed[..4], &[0x1f, 0x8b, 8, 0]);
    assert_eq!(&packed[4..8], &[0; 4]);
    assert_ne!(
        crypto::encrypt_payload(data, &key, &SALT).unwrap().iv,
        crypto::encrypt_payload(data, &key, &SALT).unwrap().iv
    );
    assert!(crypto::decrypt_envelope_with_key(&vector_envelope(3, NODE_V1), &key).is_err());
    assert!(crypto::decrypt_envelope_with_key(&vector_envelope(2, NODE_V1), &key).is_err());
    let mut invalid = rust.clone();
    let mut bytes = STANDARD.decode(&invalid.ct).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    invalid.ct = STANDARD.encode(bytes);
    assert!(crypto::decrypt_envelope_with_key(&invalid, &key).is_err());
    let mut invalid = rust;
    invalid.z = Some("deflate".into());
    assert!(crypto::decrypt_envelope_with_key(&invalid, &key).is_err());
    assert!(crypto::decrypt_envelope(&vector_envelope(1, NODE_V1), PASS.trim()).is_err());
}

#[test]
fn password_file_removes_only_one_line_ending_and_salt_reuses_bytes() {
    let tmp = tempdir().unwrap();
    let password = tmp.path().join("password");
    for (ending, expected) in [
        ("\r\n", PASS.into()),
        ("\n", PASS.into()),
        ("\r", format!("{PASS}\r")),
        ("\n\n", format!("{PASS}\n")),
    ] {
        crypto::write_secret_file(&password, &format!("{PASS}{ending}")).unwrap();
        assert_eq!(crypto::read_password_file(&password).unwrap(), expected);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&password).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&password, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(crypto::read_password_file(&password).is_err());
    }
    let path = tmp.path().join("salt");
    let salt = crypto::load_or_create_salt(&path).unwrap();
    assert_eq!(salt.len(), 16);
    assert_eq!(salt, crypto::load_or_create_salt(&path).unwrap());
    crypto::write_secret_file(&path, "not hex").unwrap();
    assert!(crypto::load_or_create_salt(&path).is_err());
}

#[test]
fn legacy_and_opus_unlock_shells_keep_exact_envelope_and_translation_contract() {
    let envelope = vector_envelope(1, NODE_V1);
    let alt = LangAlt {
        lang: Lang::En,
        base: "/en/".into(),
    };
    let opts = UnlockOptions {
        lang: Lang::ZhCn,
        lang_alt: Some(&alt),
        relative_page: "daily/item.html",
        ..UnlockOptions::default()
    };
    let builtin = crypto::wrap_envelope(&envelope, opts).unwrap();
    assert_eq!(crypto::extract_envelope(&builtin).unwrap(), envelope);
    for marker in [
        r#"id="box""#,
        r#"id="pw""#,
        r#"id="rm""#,
        r#"id="cfg""#,
        r#"class="err""#,
        "noindex,nofollow",
        "\"pw_opening\"",
        "正在解锁",
        "/en/daily/item",
    ] {
        assert!(builtin.contains(marker), "{marker}");
    }
    let config = builtin
        .split_once(r#"id="cfg">"#)
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let config: Value = serde_json::from_str(config).unwrap();
    assert!(config.get("pw_unlocking").is_none());
    assert_eq!(config["pw_opening"], i18n::t(Lang::ZhCn, "pw_opening"));
    assert!(!starts(&builtin).any(|tag| attr(&tag, b"class") == Some("demo-hint")));
    let opts = UnlockOptions {
        legacy_template: Some(LEGACY_TEMPLATE),
        demo_password: Some("<demo>"),
        ..opts
    };
    let legacy = crypto::wrap_envelope(&envelope, opts).unwrap();
    assert_eq!(crypto::extract_envelope(&legacy).unwrap(), envelope);
    assert!(legacy.contains("var txtOpening = \"正在解锁…\";"));
    assert!(legacy.contains("&lt;demo&gt;"));
    assert!(legacy.contains("/en/daily/item"));
    assert!(crypto::collect_script_hashes(&legacy, &ScriptAllowlist::default()).is_err());
    let mut trusted = ScriptAllowlist::default();
    trusted
        .allow_legacy_template(LEGACY_TEMPLATE, opts)
        .unwrap();
    assert_eq!(
        crypto::collect_script_hashes(&legacy, &trusted)
            .unwrap()
            .len(),
        2
    );
    for invalid in ["<html></html>", "__ENCRYPT_CFG_JSON__ __ENCRYPT_CFG_JSON__"] {
        assert!(crypto::wrap_envelope(
            &envelope,
            UnlockOptions {
                legacy_template: Some(invalid),
                ..opts
            }
        )
        .is_err());
    }
    assert!(
        crypto::assert_encrypted_shell(&builtin.replace(">打开<", ">Missing<"), Lang::ZhCn)
            .is_err()
    );
    let injected = builtin.replacen("</main>", "<p>Example briefing 甲</p></main>", 1);
    assert!(
        crypto::assert_no_plaintext_leak(VECTOR_PLAIN, &injected, &builtin, Lang::ZhCn).is_err()
    );
    assert_eq!(
        model::escape_json_for_script(r#"{ "x": "</script>" }"#),
        r#"{ "x": "\u003c/script\u003e" }"#
    );
    assert_eq!(
        model::escape_json_for_script("\u{2028}\u{2029}"),
        r"\u2028\u2029"
    );
}

#[test]
fn lock_and_csp_accept_known_scripts_and_reject_injection_before_hashing() {
    let doc = render::render_digest_html(
        &fixture(DAILY_EN, Lang::En),
        Lang::En,
        &RenderOptions::default(),
    );
    let lock = crypto::inject_lock_ui_html(&doc, Lang::En);
    assert_eq!(lock, crypto::inject_lock_ui_html(&lock, Lang::En));
    assert_eq!(lock.matches(" data-lock ").count(), 1);
    assert_eq!(lock.matches("digest-lock:start").count(), 1);
    assert_eq!(lock.matches("digest-lock-btn:start").count(), 1);
    assert_eq!(
        lock.matches(&format!("<script>{}</script>", assets::LOCK_JS))
            .count(),
        1
    );
    assert_eq!(render::strip_injections(&lock), doc);
    let shells = [
        lock.clone(),
        archive(),
        crypto::wrap_envelope(&vector_envelope(1, NODE_V1), UnlockOptions::default()).unwrap(),
        render::render_404_html(),
    ];
    let allowlist = ScriptAllowlist::default();
    let first = crypto::script_hash(assets::BOOT_JS);
    for shell in &shells {
        let found = crypto::collect_script_hashes(shell, &allowlist).unwrap();
        assert_eq!(found[0], first);
        let order = [
            "<meta charset=\"utf-8\">",
            "viewport-fit=cover",
            "name=\"theme-color\"",
            "<script>",
            "<title>",
            "rel=\"icon\"",
            "<style>",
        ];
        assert!(order
            .windows(2)
            .all(|keys| shell.find(keys[0]).unwrap() < shell.find(keys[1]).unwrap()));
    }
    assert_eq!(
        crypto::collect_script_hashes(&shells[3], &allowlist)
            .unwrap()
            .len(),
        1
    );
    assert!(crypto::collect_script_hashes("<script>unexpected()</script>", &allowlist).is_err());
    assert!(crypto::collect_script_hashes("<script>unexpected()", &allowlist).is_err());
    let ignored = r#"<script src="/fixture.js">ignored()</script><script type="application/json">{"later":"<b>"}</script><script type="application/ld+json">{}</script><script type="speculationrules">{}</script>"#;
    assert!(crypto::collect_script_hashes(ignored, &allowlist)
        .unwrap()
        .is_empty());
    assert!(crypto::collect_script_hashes(
        &format!("{}{ignored}", "<!-- <script>ignored()</script> -->"),
        &allowlist
    )
    .unwrap()
    .is_empty());
    let mut trusted = ScriptAllowlist::default();
    trusted.allow_legacy_scripts(["  legacy();\r\n\r  "]);
    assert_eq!(
        crypto::collect_script_hashes("<script>  legacy();\n\n  </script>", &trusted).unwrap(),
        [crypto::script_hash("  legacy();\r\n\r  ")]
    );
    assert!(crypto::collect_script_hashes("<script>legacy();\n\n</script>", &trusted).is_err());
    let oversized = " ".repeat(crypto::MAX_INLINE_JS_BYTES + 1);
    trusted.allow_legacy_scripts([oversized.as_str()]);
    assert!(
        crypto::collect_script_hashes(&format!("<script>{oversized}</script>"), &trusted).is_err()
    );

    let tmp = tempdir().unwrap();
    let paths = DigestPaths::at(tmp.path(), chrono_tz::UTC);
    atomic_write_text(&paths.plain.join("index.html"), &lock).unwrap();
    atomic_write_text(&paths.public.join("index.html"), &shells[2]).unwrap();
    let headers =
        crypto::finalize_csp_headers(&paths.public, &paths.plain, false, &allowlist).unwrap();
    let csp = &headers["Content-Security-Policy"];
    assert!(csp.contains(&first));
    assert!(csp.contains(&crypto::script_hash(assets::LOCK_JS)));
    assert!(csp.contains(&crypto::script_hash(assets::UNLOCK_JS)));
    assert!(csp.contains("script-src-attr 'none'"));
    assert!(!csp
        .split(';')
        .find(|s| s.trim_start().starts_with("script-src "))
        .unwrap()
        .contains("unsafe-inline"));
    let headers_path = paths.public.join("_headers");
    let prior = fs::read_to_string(&headers_path).unwrap();
    atomic_write_text(
        &paths.public.join("injected.html"),
        "<script>unknown()</script>",
    )
    .unwrap();
    assert!(crypto::finalize_csp_headers(&paths.public, &paths.plain, false, &allowlist).is_err());
    assert_eq!(fs::read_to_string(headers_path).unwrap(), prior);
}

#[test]
fn privacy_scans_raw_visible_bilingual_and_public_mirrors_and_fails_closed() {
    let tmp = tempdir().unwrap();
    let paths = DigestPaths::at(tmp.path(), chrono_tz::UTC);
    let file = paths.plain.join("index.html");
    let scan =
        || crypto::privacy_scan_with_patterns(&paths.site, &paths.plain, &paths.public, None);
    for (i, (text, label)) in [
        (concat!("<span data-label=\"person", "@", "example.com\">safe</span>"), "email"),
        ("<p>我<span>的</span><b>关</b>注</p>", "x-circle"),
        ("<p>name&#64;example&#46;com</p>", "email"),
        ("<div class=\"lang-pane lang-zh\">安全</div><div class=\"lang-pane lang-en\" hidden>People I <em>follow</em></div>", "x-circle"),
        (concat!("<p>", "/ho", "me/", "fixture/.config</p>"), "local-path"),
    ].into_iter().enumerate() {
        atomic_write_text(&file, text).unwrap();
        let error = scan().expect_err(&format!("privacy fixture {i}: {label}"));
        assert!(error.to_string().contains(label));
    }
    atomic_write_text(&file, "<p>Ordinary output</p>").unwrap();
    let mirror = paths.public.join("en/daily/example.md");
    atomic_write_text(&mirror, "My book<span>marks</span>").unwrap();
    assert!(scan().is_err());
    fs::remove_file(mirror).unwrap();
    let patterns = paths.site.join("privacy-patterns.txt");
    atomic_write_text(&patterns, "(?s)ordinary.{0,2}output\tfixture\n").unwrap();
    assert!(scan().unwrap_err().to_string().contains("fixture"));
    let override_file = tmp.path().join("patterns");
    atomic_write_text(&override_file, "absent\n").unwrap();
    assert!(crypto::privacy_scan_with_patterns(
        &paths.site,
        &paths.plain,
        &paths.public,
        Some(&override_file)
    )
    .is_ok());
    atomic_write_text(&override_file, "(\n").unwrap();
    assert!(crypto::privacy_scan_with_patterns(
        &paths.site,
        &paths.plain,
        &paths.public,
        Some(&override_file)
    )
    .unwrap_err()
    .to_string()
    .contains("invalid privacy pattern"));
    assert!(crypto::privacy_scan_with_patterns(
        &paths.site,
        &paths.plain,
        &paths.public,
        Some(&tmp.path().join("missing"))
    )
    .is_err());
}

#[test]
fn encrypted_tree_reuses_cache_and_legacy_node_verifies_exact_plaintext() {
    let tmp = tempdir().unwrap();
    let paths = DigestPaths::at(tmp.path(), chrono_tz::UTC);
    let input = paths.plain.join("index.html");
    let output = paths.public.join("index.html");
    let password = tmp.path().join("password");
    crypto::write_secret_file(&password, &format!("{PASS}\n")).unwrap();
    atomic_write_text(&paths.config, "ENCRYPT=1\n").unwrap();
    atomic_write_text(&input, &archive()).unwrap();
    atomic_write_text(&paths.public.join("404.html"), &render::render_404_html()).unwrap();
    let key = crypto::derive_key(PASS, &SALT);
    let opts = UnlockOptions::default();
    assert_eq!(
        crypto::encrypt_all_pages(&paths, &key, &SALT, opts).unwrap(),
        [PathBuf::from("index.html")]
    );
    let wrapped = fs::read_to_string(output).unwrap();
    let plain = fs::read(&input).unwrap();
    assert!(std::str::from_utf8(&plain).unwrap().contains("data-lock"));
    assert_eq!(
        crypto::decrypt_envelope_with_key(&crypto::extract_envelope(&wrapped).unwrap(), &key)
            .unwrap(),
        plain
    );
    assert_eq!(
        crypto::assert_public_is_encrypted(&paths.plain, &paths.public, "", Lang::En).unwrap(),
        1
    );
    assert!(crypto::assert_public_matches_plain(&paths.plain, &paths.public).is_err());
    assert!(crypto::encrypt_all_pages(&paths, &key, &SALT, opts)
        .unwrap()
        .is_empty());
    let cache = crypto::load_enc_cache(&paths.enc_cache).unwrap();
    let fingerprint = cache.get("index.html").unwrap();
    assert_eq!(
        fingerprint.split(':').map(str::len).collect::<Vec<_>>(),
        [64, 16, 16, 16, 8, 2]
    );
    if Command::new("node").arg("--version").output().is_ok() {
        let result = Command::new("node")
            .args(["web/dist/verify_decrypt.mjs", "--local"])
            .env("DIGEST_ROOT", tmp.path())
            .env("DIGEST_SITE_PASSWORD_FILE", password)
            .env("DIGEST_ENCRYPT", "1")
            .env("SITE_SLUG", "")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "legacy verifier failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout)
            .contains("all 1 page(s) decrypted and matched plaintext"));
    }
}

fn issue_page(lang: Lang) -> GeoPage {
    let prefix = lang.choose("en", "zh");
    let path = format!("{prefix}/daily/2026-01-15");
    GeoPage {
        rel_html: format!("{path}.html"),
        canonical_path: format!("/{path}"),
        url_path: format!("/{path}"),
        rel_md: format!("{path}.md"),
        lang,
        kind: "daily".into(),
        label: "2026-01-15".into(),
        date_iso: "2026-01-15".into(),
        feed_path: format!("/{prefix}/feed.xml"),
        ..GeoPage::default()
    }
}

#[test]
fn geo_bundle_includes_discovery_worker_csp_and_full_private_purge() {
    let tmp = tempdir().unwrap();
    let paths = DigestPaths::at(tmp.path(), chrono_tz::UTC);
    let cfg = SiteConfig {
        domain: "example.com".into(),
        geo_ai_train: false,
        content_signal: "ai-train=no, search=yes, ai-input=yes".into(),
        ..SiteConfig::default()
    };
    let mut en = issue_page(Lang::En);
    let mut zh = issue_page(Lang::ZhCn);
    en.alternates
        .insert("zh-CN".into(), zh.canonical_path.clone());
    zh.alternates.insert("en".into(), en.canonical_path.clone());
    let mut malformed_title = fixture(MATH_ISSUE, Lang::En);
    malformed_title.title = "An issue <with angle brackets>".into();
    let en_html = render::render_digest_html(&malformed_title, Lang::En, &RenderOptions::default());
    let en_decorated =
        geo::decorate_public_html(&en_html, &mut en, &[], "https://example.com", true).unwrap();
    assert_eq!(
        en_decorated,
        geo::decorate_public_html(&en_decorated, &mut en, &[], "https://example.com", true)
            .unwrap()
    );
    for marker in [
        "rel=\"canonical\"",
        "hreflang=\"zh-CN\"",
        "type=\"text/markdown\"",
        "application/atom+xml",
        "agent:instructions",
        "og:title",
        "twitter:card",
        "NewsArticle",
        "ItemList",
        r"\u003cwith angle brackets\u003e",
    ] {
        assert!(en_decorated.contains(marker), "{marker}");
    }
    assert_eq!(en_decorated.matches("digest-meta:start").count(), 1);
    assert!(
        en_decorated.find("<script>").unwrap() < en_decorated.find("application/ld+json").unwrap()
    );
    let zh_html = render::render_digest_html(
        &fixture(DAILY_ZH, Lang::ZhCn),
        Lang::ZhCn,
        &RenderOptions::default(),
    );
    let zh_decorated =
        geo::decorate_public_html(&zh_html, &mut zh, &[], "https://example.com", true).unwrap();
    let mut index = GeoPage {
        rel_html: "index.html".into(),
        rel_md: "index.md".into(),
        page_type: PageType::Index,
        ..GeoPage::default()
    };
    let index_html =
        geo::decorate_public_html(&archive(), &mut index, &[&en], "https://example.com", true)
            .unwrap();
    assert!(index_html.contains("WebSite") && index_html.contains("CollectionPage"));
    let alias = GeoPage {
        rel_html: "en/daily/latest.html".into(),
        rel_md: "en/daily/latest.md".into(),
        url_path: "/en/daily/latest".into(),
        is_canonical: false,
        ..en.clone()
    };
    let pages = [index, en, zh, alias];
    for (page, body) in pages
        .iter()
        .zip([&index_html, &en_decorated, &zh_decorated, &en_decorated])
    {
        atomic_write_text(&paths.plain.join(&page.rel_html), body).unwrap();
        atomic_write_text(&paths.public.join(&page.rel_html), body).unwrap();
    }
    let now = Utc::now();
    let extras = BTreeMap::from([("misc/extra.md".into(), "# Extra\n".into())]);
    atomic_write_text(&paths.public.join("_headers"), "/\n  Cache-Control: public, max-age=60\n/en/daily/latest\n  ! Cache-Control\n  Cache-Control: no-store\n").unwrap();
    atomic_write_text(
        &paths.public.join("_redirects"),
        "/removed / 302\n/older /en/ 301\n",
    )
    .unwrap();
    geo::write_public_geo_bundle(&paths.public, &pages, &cfg, true, now, &extras).unwrap();
    let read = |rel: &str| {
        fs::read_to_string(paths.public.join(rel)).unwrap_or_else(|_| panic!("missing {rel}"))
    };
    let robots = read("robots.txt");
    assert!(robots.contains("User-agent: OAI-SearchBot\nAllow: /"));
    assert!(robots.contains("User-agent: GPTBot\nDisallow: /"));
    assert!(robots.contains("Sitemap: https://example.com/sitemap.xml"));
    assert!(read("sitemap.xml").contains("xhtml:link"));
    assert!(!read("sitemap.xml").contains("/latest"));
    for rel in ["feed.xml", "zh/feed.xml", "en/feed.xml"] {
        assert!(read(rel).contains("http://www.w3.org/2005/Atom"));
    }
    assert!(read("llms.txt").contains("Latest Briefings"));
    assert!(read("llms-full.txt").len() <= geo::LLMS_FULL_BUDGET);
    assert!(read("llms-full.txt").contains("Source-URL:"));
    let skill = read(geo::SKILL_PATH);
    let idx: Value = serde_json::from_str(&read(geo::SKILL_INDEX)).unwrap();
    assert_eq!(
        idx["skills"][0]["digest"],
        format!("sha256:{}", hex::encode(Sha256::digest(&skill)))
    );
    assert_eq!(
        read("en/daily/2026-01-15.md")
            .matches("$L_N[1/3, O(1)]$")
            .count(),
        1
    );
    let search: Value = serde_json::from_str(&read("search-index.json")).unwrap();
    assert_eq!(search["docs"].as_array().unwrap().len(), 12);
    assert!(search["docs"][0]["p"]
        .as_str()
        .unwrap()
        .contains("L_N[1/3, O(1)]"));
    assert!(!search["docs"][0]["p"].as_str().unwrap().contains("$L_N"));
    assert!(geo::finalize_public_worker(&paths.public, &cfg).is_err());
    let headers = crypto::finalize_csp_headers(
        &paths.public,
        &paths.plain,
        true,
        &ScriptAllowlist::default(),
    )
    .unwrap();
    let headers_before = read("_headers");
    geo::write_public_geo_bundle(&paths.public, &pages, &cfg, true, now, &extras).unwrap();
    assert_eq!(headers_before, read("_headers"));
    let worker_file = geo::finalize_public_worker(&paths.public, &cfg)
        .unwrap()
        .unwrap();
    assert_eq!(worker_file, paths.public.join("_worker.js"));
    let worker = read("_worker.js");
    assert!(!worker.contains("__WORKER_CFG_JSON__"));
    assert!(worker.contains("\"globalHeaders\"") && worker.contains("\"cacheRules\""));
    assert!(worker.contains("\"/removed\",\"/\",302"));
    assert!(worker.contains(&headers["Content-Security-Policy"]));
    assert!(read("_routes.json").contains("/.well-known/agent-skills/*"));
    assert_eq!(
        crypto::assert_public_matches_plain(&paths.plain, &paths.public).unwrap(),
        4
    );
    let worker_off = SiteConfig {
        geo_worker: false,
        ..cfg.clone()
    };
    assert!(geo::finalize_public_worker(&paths.public, &worker_off)
        .unwrap()
        .is_none());
    assert!(!paths.public.join("_worker.js").exists());
    geo::finalize_public_worker(&paths.public, &cfg).unwrap();

    atomic_write_text(&paths.public.join(".well-known/stale/file.json"), "{}").unwrap();
    atomic_write_text(&paths.public.join("nested/doc.MD"), "plaintext").unwrap();
    let private = SiteConfig {
        encrypt: true,
        ..cfg
    };
    geo::write_public_geo_bundle(&paths.public, &pages, &private, true, now, &BTreeMap::new())
        .unwrap();
    assert_eq!(read("robots.txt"), "User-agent: *\nDisallow: /\n");
    assert!(!paths.public.join(".well-known").exists());
    for rel in geo::GEO_ROOT_FILES.iter().copied().chain([
        "en/feed.xml",
        "zh/feed.xml",
        "en/daily/latest.md",
        "misc/extra.md",
        "nested/doc.MD",
        "index.md",
    ]) {
        assert!(!paths.public.join(rel).exists(), "{rel}");
    }
    assert!(read("_headers").contains("X-Robots-Tag: noindex, nofollow"));
    assert!(!read("_headers").contains("Content-Signal:"));
    assert!(!read("_headers").contains("llms.txt"));
    assert!(paths.public.join("index.html").is_file());
}

#[test]
fn worker_requires_exactly_one_placeholder_and_serializes_seven_fields() {
    let placeholder = "__WORKER_CFG_JSON__";
    let cfg = WorkerConfig {
        origin: "https://example.com".into(),
        content_signal: geo::CONTENT_SIGNAL_DEFAULT.into(),
        link_header: "<b>".into(),
        search_enabled: true,
        global_headers: BTreeMap::new(),
        cache_rules: vec![("/latest".into(), "no-store".into())],
        redirects: vec![("/old".into(), "/en/".into(), 302)],
    };
    assert_eq!(assets::WORKER_JS.matches(placeholder).count(), 1);
    assert!(geo::inject_worker_config("const missing = null;", &cfg).is_err());
    assert!(geo::inject_worker_config(&format!("{placeholder} {placeholder}"), &cfg).is_err());
    let prefix = "const CFG = ";
    let suffix = ";\nexport default {};";
    let result =
        geo::inject_worker_config(&format!("{prefix}{placeholder}{suffix}"), &cfg).unwrap();
    let config = result
        .strip_prefix(prefix)
        .unwrap()
        .strip_suffix(suffix)
        .unwrap();
    let actual: Value = serde_json::from_str(config).unwrap();
    assert_eq!(
        actual
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "origin",
            "contentSignal",
            "linkHeader",
            "searchEnabled",
            "globalHeaders",
            "cacheRules",
            "redirects"
        ]
    );
    assert_eq!(actual["cacheRules"], json!([["/latest", "no-store"]]));
    assert_eq!(actual["redirects"], json!([["/old", "/en/", 302]]));
    assert!(config.contains(r"\u003cb\u003e"));
}

#[test]
fn built_worker_runtime_contract_when_frontend_assets_are_available() {
    if assets::WORKER_JS.contains("new Response(\"stub\")")
        || Command::new("node").arg("--version").output().is_err()
    {
        return;
    }
    let cfg = WorkerConfig {
        origin: "https://example.com".into(),
        content_signal: geo::CONTENT_SIGNAL_DEFAULT.into(),
        link_header: geo::build_link_header(true, "/feed.xml"),
        search_enabled: true,
        global_headers: BTreeMap::from([(
            "Content-Security-Policy".into(),
            "default-src 'none'".into(),
        )]),
        cache_rules: vec![("/latest".into(), "no-store".into())],
        redirects: vec![("/old".into(), "/".into(), 302)],
    };
    let source = STANDARD.encode(geo::render_pages_worker(&cfg).unwrap());
    let mut node = Command::new("node")
        .arg("--input-type=module")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = node.stdin.take().unwrap();
    writeln!(stdin, "const source = {source:?};").unwrap();
    stdin.write_all(br#"
import assert from 'node:assert/strict';
const { default: worker } = await import('data:application/javascript;base64,' + source);
const text = { '/index.md': '# Mirror', '/': '<p>HTML</p>' };
const env = { ASSETS: { fetch: async request => {
  const url = new URL(typeof request === 'string' ? request : request.url || request.href);
  return url.pathname in text ? new Response(text[url.pathname], {headers:{'content-type': url.pathname.endsWith('.md') ? 'text/markdown' : 'text/html'}}) : new Response('missing', {status:404});
}}};
const req = (path, options) => worker.fetch(new Request('https://example.com' + path, options), env);
const equal = await req('/', {headers:{accept:'text/markdown;q=0.8,text/html;q=0.8'}});
assert.equal(await equal.text(), '<p>HTML</p>');
assert.equal(await (await req('/', {headers:{accept:'text/markdown'}})).text(), '# Mirror');
assert.equal(await (await req('/', {method:'HEAD',headers:{accept:'text/markdown'}})).text(), '');
assert.equal((await req('/.well-known/search?q=model')).status, 503);
assert.equal((await req('/.well-known/search')).status, 400);
assert.equal((await req('/.well-known/search', {method:'POST'})).status, 405);
assert.equal((await req('/.well-known/search', {method:'QUERY',body:'x'.repeat(4097),headers:{'content-type':'text/plain'}})).status, 413);
assert.equal((await req('/old')).status, 302);
const missing = await req('/missing');
assert.equal(missing.status, 404);
assert.equal(missing.headers.get('cache-control'), 'no-store');
assert.equal(missing.headers.get('x-robots-tag'), 'noindex, nofollow');
"#).unwrap();
    drop(stdin);
    let result = node.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "worker runtime failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn ledger_crud_preserves_fields_backup_and_promoted_status() {
    let tmp = tempdir().unwrap();
    let paths = model::DigestPaths::at(tmp.path(), chrono_tz::Asia::Taipei);
    assert!(ledger::load_ledger(&paths.ledger).unwrap().items.is_empty());
    let original = r#"{"legacy_meta":{"keep":true},"items":[{"id":"2601.12345","name":"Fixture Legacy","category":"seen","first_seen":"2026-01-14","urls":["https://arxiv.org/pdf/2601.12345"],"weekly_review":{"keep":true}},{"id":"classic","name":"Classic","category":"paper","day":"2026-01-10","status":null}]}"#;
    atomic_write_text(&paths.ledger, original).unwrap();
    let d = day("2026-01-15");
    let log = paths.runs.join("2026-01-15/filter-log.md");
    let add = ledger::add_items(&paths, d,
        "# fixture\nP|Fixture Legacy|different|paper|https://huggingface.co/papers/2601.12345|promote\nS|Fixture Legacy|2601.12345|paper|https://example.com/other|again\nS|Classic|classic|paper||old\nS|New Tool||project|https://example.com/x?utm_source=fixture https://example.com/x|hold",
        AddOptions { lang: Lang::En, log: Some(&log), log_heading: Some("Addendum") }).unwrap();
    assert_eq!((add.added, add.updated, add.total), (1, 3, 3));
    let ledger_data = ledger::load_ledger(&paths.ledger).unwrap();
    assert_eq!(ledger_data.extra["legacy_meta"]["keep"], true);
    assert_eq!(ledger_data.extra["item_count"], 3);
    assert!(ledger_data.extra["previous_digest_at"]
        .as_str()
        .unwrap()
        .ends_with("+08:00"));
    let promoted = &ledger_data.items[0];
    assert_eq!(promoted["weekly_review"]["keep"], true);
    assert_eq!(promoted["pushed"], "2026-01-15");
    assert_eq!(ledger::ledger_status(promoted), "pushed");
    assert_eq!(ledger::ledger_status(&ledger_data.items[1]), "pushed");
    let found = ledger::find_items(&ledger_data, "FIXTURE legacy");
    assert_eq!(found.len(), 1);
    assert_eq!(
        ledger::find_items(&ledger_data, "example.com/other").len(),
        1
    );
    assert_eq!(ledger_data.items[2]["id"], "new-tool");
    assert_eq!(ledger_data.items[2]["urls"].as_array().unwrap().len(), 1);
    assert_eq!(
        ledger::recent_items(&ledger_data, d, 1, "seen", Some("project")).len(),
        1
    );
    assert_eq!(
        ledger::load_ledger_keys(&paths.ledger).unwrap()["arxiv:2601.12345"],
        "pushed"
    );
    let backup = paths.ledger_backup.join("ledger-2026-01-15.json");
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    let f = fs::read_to_string(log).unwrap();
    assert!(
        f.contains("## Addendum")
            && f.contains("### Pushed")
            && f.contains("- Fixture Legacy: promote")
    );
    ledger::add_items(&paths, d, "S|Another|another|||", AddOptions::default()).unwrap();
    assert_eq!(fs::read_to_string(backup).unwrap(), original);
    assert!(
        fs::read_to_string(&paths.ledger)
            .unwrap()
            .find("legacy_meta")
            .unwrap()
            < fs::read_to_string(&paths.ledger)
                .unwrap()
                .find("\"items\"")
                .unwrap()
    );
    atomic_write_text(&paths.ledger, "{").unwrap();
    assert_eq!(
        ledger::load_ledger(&paths.ledger).unwrap_err().exit_code(),
        2
    );
    let error = ledger::add_items(&paths, d, "P|No update", AddOptions::default()).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<ledger::LedgerReadError>()
            .unwrap()
            .exit_code(),
        2
    );
    assert_eq!(fs::read_to_string(&paths.ledger).unwrap(), "{");
}

#[test]
fn architecture_layers_only_depend_on_lower_layers() {
    let single = Regex::new(r"crate::([a-z_][a-z0-9_]*)").unwrap();
    let grouped = Regex::new(r"(?s)\buse crate::\{(.*?)\n\};").unwrap();
    let branch = Regex::new(r"(?m)^    ([a-z_][a-z0-9_]*)[,:]").unwrap();
    for (source, allowed) in [
        (include_str!("../src/version.rs"), &[][..]),
        (include_str!("../src/assets.rs"), &[][..]),
        (include_str!("../src/i18n.rs"), &[][..]),
        (include_str!("../src/math.rs"), &[][..]),
        (include_str!("../src/net.rs"), &[][..]),
        (include_str!("../src/model.rs"), &["i18n", "version"][..]),
        (include_str!("../src/parser.rs"), &["i18n", "model"][..]),
        (include_str!("../src/ledger.rs"), &["i18n", "model"][..]),
        (
            include_str!("../src/render.rs"),
            &["assets", "i18n", "math", "model"][..],
        ),
        (
            include_str!("../src/crypto.rs"),
            &["assets", "i18n", "model"][..],
        ),
        (
            include_str!("../src/geo.rs"),
            &["assets", "i18n", "model", "parser"][..],
        ),
    ] {
        let direct = single.captures_iter(source);
        let grouped = grouped
            .captures_iter(source)
            .flat_map(|m| branch.captures_iter(m.get(1).unwrap().as_str()));
        for imported in direct.chain(grouped) {
            assert!(
                allowed.contains(&&imported[1]),
                "dependency {} violates the layer",
                &imported[1]
            );
        }
    }
    assert!(assets::FAVICON_SVG.starts_with("<svg"));
}

#[test]
fn encrypted_shell_privacy_scan_cache_integrity_and_strict_envelope_limits() {
    let tmp = tempdir().unwrap();
    let paths = DigestPaths::at(tmp.path(), chrono_tz::UTC);
    let plain_path = paths.plain.join("index.html");
    let pub_path = paths.public.join("index.html");
    atomic_write_text(&plain_path, &archive()).unwrap();
    let key = crypto::derive_key(PASS, &SALT);
    let opts = UnlockOptions::default();
    crypto::encrypt_all_pages(&paths, &key, &SALT, opts).unwrap();

    // Clean encrypted shell passes privacy scan
    crypto::privacy_scan_with_patterns(&paths.site, &paths.plain, &paths.public, None).unwrap();

    // Leaked email inside encrypted shell outside #c must be caught by privacy_scan!
    let shell = fs::read_to_string(&pub_path).unwrap();
    let leaked_shell = shell.replacen(
        "</main>",
        &format!(
            "<p>contact: {}</p></main>",
            ["leak", "@", "example.com"].concat()
        ),
        1,
    );
    atomic_write_text(&pub_path, &leaked_shell).unwrap();
    let err = crypto::privacy_scan_with_patterns(&paths.site, &paths.plain, &paths.public, None)
        .unwrap_err();
    assert!(err.to_string().contains("email"), "{err:#}");

    // Tampered cached shell (without privacy pattern) must NOT be reused from enc-cache.json!
    let tampered = shell.replacen("</main>", "<p>tampered shell snippet</p></main>", 1);
    atomic_write_text(&pub_path, &tampered).unwrap();
    let changed = crypto::encrypt_all_pages(&paths, &key, &SALT, opts).unwrap();
    assert_eq!(changed, [PathBuf::from("index.html")]);
    assert!(!fs::read_to_string(&pub_path)
        .unwrap()
        .contains("tampered shell snippet"));

    // Strict envelope limits: exact iter == 600_000, exact salt == 16, exact iv == 12, gzip cap <= 32 MiB
    let valid = crypto::encrypt_payload_with_nonce(b"hello", &key, &SALT, NONCE).unwrap();
    for bad_iter in [crypto::PBKDF2_ITERATIONS - 1, crypto::PBKDF2_ITERATIONS + 1] {
        let mut env = valid.clone();
        env.iter = bad_iter;
        assert!(crypto::decrypt_envelope_with_key(&env, &key).is_err());
        assert!(crypto::decrypt_envelope(&env, PASS).is_err());
    }
    for bad_salt in [vec![1u8; 15], vec![1u8; 17]] {
        let mut env = valid.clone();
        env.salt = STANDARD.encode(bad_salt);
        assert!(crypto::decrypt_envelope_with_key(&env, &key).is_err());
    }
    for bad_iv in [vec![2u8; 11], vec![2u8; 13]] {
        let mut env = valid.clone();
        env.iv = STANDARD.encode(bad_iv);
        assert!(crypto::decrypt_envelope_with_key(&env, &key).is_err());
    }

    // Gzip bomb (> 32 MiB decompressed) must be rejected before unbounded allocation
    let bomb = vec![0u8; crypto::MAX_DECOMPRESSED_BYTES + 4096];
    let bomb_env = crypto::encrypt_payload_with_nonce(&bomb, &key, &SALT, NONCE).unwrap();
    let err = crypto::decrypt_envelope_with_key(&bomb_env, &key).unwrap_err();
    assert!(err.to_string().contains("size limit"), "{err:#}");
}

#[test]
fn reader_toc_collapsed_table_alignment_classes_and_bilingual_pane_hooks() {
    let zh = fixture(DAILY_ZH, Lang::ZhCn);
    let en = fixture(DAILY_EN, Lang::En);
    let single = render::render_digest_html(&en, Lang::En, &RenderOptions::default());

    // User mandate: ALL TOC sections expanded by default on SSR regardless of item count (Cmd+F friendly)
    assert_eq!(
        single.matches(r#"data-open="true""#).count(),
        en.sections.len()
    );
    assert_eq!(
        single
            .matches(r#"class="ts-sec" aria-expanded="true""#)
            .count(),
        en.sections.len()
    );
    assert_eq!(
        single.matches(r#"class="ts-panel" id=""#).count(),
        en.sections.len()
    );
    assert!(single.contains(r#"class="toc-expand-all" aria-expanded="true""#));

    // Even large 25+ / 54-item issues render ALL TOC sections expanded by default on SSR
    let mut big = en.clone();
    while big.items().count() < 30 {
        let next_n = big.items().count() + 1;
        let mut extra = en.sections[0].entries[0].clone();
        if let ai_digest::model::SectionEntry::Item(ref mut it) = extra {
            it.id = format!("extra-{next_n}");
            it.number = next_n.to_string();
        }
        big.sections[0].entries.push(extra);
    }
    let big_html = render::render_digest_html(&big, Lang::En, &RenderOptions::default());
    assert_eq!(
        big_html.matches(r#"data-open="true""#).count(),
        big.sections.len()
    );
    assert_eq!(
        big_html
            .matches(r#"class="ts-sec" aria-expanded="true""#)
            .count(),
        big.sections.len()
    );
    assert_eq!(
        big_html
            .matches(r#"class="ts-sec" aria-expanded="false""#)
            .count(),
        0
    );

    // Leading 3D emojis stripped cleanly from rendered section headings & TOC
    assert_eq!(render::strip_leading_emoji("🧠 模型与架构"), "模型与架构");
    assert_eq!(render::strip_leading_emoji("🛠️ 工具与开源"), "工具与开源");
    assert_eq!(render::strip_leading_emoji("⚡ 算力与芯片"), "算力与芯片");
    assert_eq!(render::strip_leading_emoji("🛡️ 安全与治理"), "安全与治理");
    assert_eq!(
        render::strip_leading_emoji("Models & Benchmarks"),
        "Models & Benchmarks"
    );

    assert!(single.contains(r#"<meta name="color-scheme" content="dark">"#));
    assert!(single.contains(r#"<script type="speculationrules">"#));
    assert!(single.contains(r#"<div class="filter-inner">"#));
    assert!(single.contains(r#"class="ctrl-btn to-top" data-to-top"#));
    assert!(single.contains(r#"class="ts-num num""#));
    assert!(single.contains(r#"class="sec-cnt num""#));

    // Markdown table alignment uses CSS classes ta-l/ta-c/ta-r and zero inline style=
    let table_html = render::render_markdown(
        "| Left | Center | Right | Plain |\n|:---|:---:|---:|---|\n| a | b | c | d |\n",
        false,
    );
    assert!(!table_html.contains("style="), "{table_html}");
    assert!(
        table_html.contains(r#"<th class="ta-l">Left</th>"#),
        "{table_html}"
    );
    assert!(
        table_html.contains(r#"<th class="ta-c">Center</th>"#),
        "{table_html}"
    );
    assert!(
        table_html.contains(r#"<th class="ta-r">Right</th>"#),
        "{table_html}"
    );
    assert!(
        table_html.contains(r#"<td class="ta-l">a</td>"#),
        "{table_html}"
    );
    assert!(
        table_html.contains(r#"<td class="ta-c">b</td>"#),
        "{table_html}"
    );
    assert!(
        table_html.contains(r#"<td class="ta-r">c</td>"#),
        "{table_html}"
    );

    // Bilingual inline CSS overhead <= 350 B and clean per-pane encapsulation
    let single_zh = render::render_digest_html(&zh, Lang::ZhCn, &RenderOptions::default());
    let bi = render::render_bilingual_html(&zh, &en, &RenderOptions::default());
    let single_style = single
        .split_once("<style>")
        .unwrap()
        .1
        .split_once("</style>")
        .unwrap()
        .0;
    let bi_style = bi
        .split_once("<style>")
        .unwrap()
        .1
        .split_once("</style>")
        .unwrap()
        .0;
    assert!(
        bi_style.len() - single_style.len() <= 350,
        "bilingual CSS delta {} > 350",
        bi_style.len() - single_style.len()
    );
    for (label, doc) in [
        ("single-en", &single),
        ("single-zh", &single_zh),
        ("bilingual", &bi),
    ] {
        let style_total: usize = doc
            .match_indices("<style>")
            .map(|(start, _)| {
                let rest = &doc[start + "<style>".len()..];
                rest.split_once("</style>")
                    .expect("closing </style>")
                    .0
                    .len()
            })
            .sum();
        assert!(
            style_total > 0 && style_total <= 22_528,
            "{label} inline <style> budget exceeded: {style_total} > 22528 bytes"
        );
        for token in model::html_tokens(doc) {
            if let Token::StartTag(tag) = token {
                for attr_name in tag.attributes.keys() {
                    let name = String::from_utf8_lossy(attr_name).to_ascii_lowercase();
                    assert_ne!(name, "style", "{label} contains inline style= attribute");
                    assert!(
                        !name.starts_with("on"),
                        "{label} contains inline event handler attribute {name}="
                    );
                }
            }
        }
    }
    assert!(bi.contains(r#"data-lang="zh-CN" data-lang-pane="zh-CN""#));
    assert!(bi.contains(r#"data-lang="en" data-lang-pane="en""#));
    let (before_zh, rest) = bi.split_once(r#"<div class="lang-pane lang-zh""#).unwrap();
    assert!(!before_zh.contains(r#"<a class="skip-link""#));
    let (zh_pane, en_pane) = rest
        .split_once(r#"<div class="lang-pane lang-en""#)
        .unwrap();
    for pane in [zh_pane, en_pane] {
        for hook in [
            "class=\"skip-link\"",
            "mast-meta",
            "class=\"toc-side\"",
            "class=\"filter-bar",
            "<main id=\"",
        ] {
            assert!(pane.contains(hook), "missing {hook} in lang-pane");
        }
    }
}

#[test]
fn qa_round1_regressions_hyphens_empty_sections_and_flow_drawer() {
    // 1. Technical hyphen protection helper
    assert_eq!(
        render::protect_technical_hyphens("SWE-2.1"),
        "SWE\u{2011}2.1"
    );
    assert_eq!(
        render::protect_technical_hyphens("Orion-3 Mini and Nemotron-Labs-Image"),
        "Orion\u{2011}3 Mini and Nemotron\u{2011}Labs\u{2011}Image"
    );
    assert_eq!(
        render::protect_technical_hyphens("plain - spaced --flag"),
        "plain - spaced --flag"
    );

    // 2. Rendered markdown preserves ASCII '-' in URLs, inline code, and code blocks
    let md = "Check [SWE-2.1](https://example.com/swe-2-1) and `agentkit-lite` and Nemotron-Labs-Image.\n\n```sh\nrun-tool --dry-run\n```";
    let html = render::render_markdown(md, false);
    assert!(
        html.contains(r#"href="https://example.com/swe-2-1""#),
        "URL href must preserve ASCII hyphen: {html}"
    );
    assert!(
        html.contains(">SWE\u{2011}2.1</a>"),
        "Link text must protect technical hyphen: {html}"
    );
    assert!(
        html.contains("<code>agentkit-lite</code>"),
        "Inline code must preserve ASCII hyphen: {html}"
    );
    assert!(
        html.contains("run-tool --dry-run"),
        "Fenced code block must preserve ASCII hyphen: {html}"
    );
    assert!(
        html.contains("Nemotron\u{2011}Labs\u{2011}Image"),
        "Prose text must protect technical hyphen: {html}"
    );

    // 3. Empty section omitted from .folio-sections AND .filter-drawer placed outside fixed </header>
    let md_with_empty = "\
**AI精选日报 · 11/26**

## 今日必读

## 🧠 模型

### 1. SWE-2.1：Nemotron-Labs-Image 联合发布
`Acme Labs · 11/26 · 强化学习 / 代码`

**一句话：** SWE-2.1 与 Orion-3 Mini 同步更新。

**值得看：** 保持 `swe-2.1` 代码标识不变。

🔗 [官方博客](https://example.com/swe-2-1)
";
    let digest = parser::parse_digest_md(md_with_empty, Lang::ZhCn, Some(2026)).unwrap();
    let doc = render::render_digest_html(&digest, Lang::ZhCn, &RenderOptions::default());

    let folio_nav = doc
        .split_once(r#"<nav class="folio-sections""#)
        .unwrap()
        .1
        .split_once("</nav>")
        .unwrap()
        .0;
    assert!(
        !folio_nav.contains("今日必读"),
        "empty section must be omitted from .folio-sections: {folio_nav}"
    );
    assert!(
        folio_nav.contains("模型"),
        "non-empty section must appear in .folio-sections: {folio_nav}"
    );

    // Filter drawer sits in normal document flow AFTER fixed </header>, before .reader-shell
    let mast_end = doc.find("</header>").expect("mast </header>");
    let filter_pos = doc
        .find(r#"class="filter-bar filter-drawer"#)
        .expect("filter drawer");
    let shell_pos = doc.find(r#"class="reader-shell""#).expect("reader shell");
    assert!(
        mast_end < filter_pos && filter_pos < shell_pos,
        "filter-drawer must sit between </header> ({mast_end}) and .reader-shell ({shell_pos}), got {filter_pos}"
    );

    // TOC and story protect technical hyphens while href keeps ASCII '-'
    assert!(doc.contains("SWE\u{2011}2.1"));
    assert!(doc.contains("Nemotron\u{2011}Labs\u{2011}Image"));
    assert!(doc.contains("Orion\u{2011}3 Mini"));
    assert!(doc.contains(r#"href="https://example.com/swe-2-1""#));
    assert!(doc.contains("<code>swe-2.1</code>"));

    // 4. strip_leading_emoji removes pictographic emojis but preserves technical symbols (⌘, ①)
    assert_eq!(render::strip_leading_emoji("🧠 模型"), "模型");
    assert_eq!(render::strip_leading_emoji("⌘ 键快捷键"), "⌘ 键快捷键");
    assert_eq!(render::strip_leading_emoji("① 核心结论"), "① 核心结论");

    // 5. Privacy scan still detects hyphenated secrets & custom hyphen rules in rendered HTML
    let fake_token = ["sk", "-", "1234567890abcdefghijklmnop"].concat();
    let leak_md = format!(
        "\
**AI精选日报 · 11/26**

## 🧠 模型

### 1. Leak-Check
`Lab · 11/26 · 安全`

**一句话：** Token {fake_token} and corp-secret-token appeared in prose.

🔗 [Note](https://example.com/note)
"
    );
    let leak_digest = parser::parse_digest_md(&leak_md, Lang::ZhCn, Some(2026)).unwrap();
    let leak_html = render::render_digest_html(&leak_digest, Lang::ZhCn, &RenderOptions::default());
    assert!(leak_html.contains(&fake_token.replace('-', "\u{2011}")));
    let td = tempfile::tempdir().unwrap();
    let plain = td.path().join("plain");
    let public = td.path().join("public");
    fs::create_dir_all(&plain).unwrap();
    fs::create_dir_all(&public).unwrap();
    fs::write(plain.join("leak.html"), &leak_html).unwrap();
    assert!(
        crypto::privacy_scan_with_patterns(td.path(), &plain, &public, None).is_err(),
        "privacy_scan must still catch hyphenated token secrets after U+2011 protection"
    );
    let custom_md = leak_md.replace(&fake_token, "benign-id");
    let custom_digest = parser::parse_digest_md(&custom_md, Lang::ZhCn, Some(2026)).unwrap();
    let custom_html =
        render::render_digest_html(&custom_digest, Lang::ZhCn, &RenderOptions::default());
    fs::write(plain.join("leak.html"), &custom_html).unwrap();
    let rules_file = td.path().join("custom-rules.txt");
    fs::write(&rules_file, "corp-secret-token\tcustom-corp\n").unwrap();
    assert!(
        crypto::privacy_scan_with_patterns(td.path(), &plain, &public, Some(&rules_file)).is_err(),
        "privacy_scan must still catch custom hyphenated rules after U+2011 protection"
    );

    // 6. Incremental upgrade: previous v1.0.0 + v1.0.1 inline script hashes remain allowlisted
    let allowlist = crypto::ScriptAllowlist::default();
    let legacy_doc = format!(
        "<!doctype html><html><head><script>{}</script></head><body></body></html>",
        "alert('untrusted')"
    );
    assert!(
        crypto::collect_script_hashes(&legacy_doc, &allowlist).is_err(),
        "arbitrary inline scripts must remain rejected"
    );
    for h in [
        "'sha256-K8AMXq9YtQjRNhY29wqFrBeVSAmso/fUzqfuqELyxS0='",
        "'sha256-za2UDgPTC4kgNKKeVfbNEujaIgnNHJnzdVgff5jcG5o='",
        "'sha256-2d1HlyBoJ0RTWBo1V8cJq5gqndqRo/+j5jGqg+hN+Vk='",
        "'sha256-ULWcGE48zLdDVllI0EYBwq+iTR3kAcDbzv+3QVUarYQ='",
        "'sha256-2Ivlf0XJIcWpuKux6MsYqWgB57xkKsnxmYqoT9r6D4g='",
        "'sha256-aXNfRvBaL19OMemVpb9AUUGmEbb1BwKS6xDxb4Pb5HU='",
        "'sha256-9HqZwpsqf7FMp1SP40jMRJ6HOhL5ezGD/GcJa+n99Qw='",
        "'sha256-bAOgKqYwVOp7VK1g8OY7p0/ZwTM83IkvpUD6b66i2Qw='",
    ] {
        assert!(
            allowlist.contains_hash(h),
            "previous release script hash {h} must remain allowlisted for incremental publish upgrades"
        );
    }

    // 7. v1.0.2 editorial TOC single-line .ts-title & redundant daily date hiding (while preserving parser roundtrip)
    assert!(
        doc.contains(
            r#"<span class="ts-title short">SWE‑2.1：Nemotron‑Labs‑Image 联合发布</span>"#
        ),
        "TOC item must wrap full title in .ts-title for balanced 2-line wrap + Cmd+F"
    );
    assert!(
        doc.contains(r#"<time class="when" hidden>11.26</time>"#),
        "daily story matching issue date must hide redundant visual date stamp"
    );
    let parsed_doc = parse_digest_html(&doc).unwrap();
    assert_eq!(parsed_doc.items[0].when, "11.26");

    // Non-matching date on a daily issue (or any weekly story) remains visible
    let md_prev_day = md_with_empty.replace("11/26 · 强化学习", "11/25 · 强化学习");
    let prev_digest = parser::parse_digest_md(&md_prev_day, Lang::ZhCn, Some(2026)).unwrap();
    let prev_doc = render::render_digest_html(&prev_digest, Lang::ZhCn, &RenderOptions::default());
    assert!(
        prev_doc.contains(r#"<time class="when">11.25</time>"#),
        "daily story from a different date must keep visible date stamp"
    );
}
