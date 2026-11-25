import base64, gzip, hashlib, json, os, re, shutil, subprocess, sys
from datetime import date
from pathlib import Path

import pytest
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

import publish
import render_html as rh

ROOT = Path(__file__).resolve().parents[1]


def _decrypt(page_path, password="demo-password-123"):
    text = page_path.read_text(encoding="utf-8")
    i = text.index('id="c">') + len('id="c">')
    cfg = json.loads(text[i:text.index("</script>", i)])
    key = hashlib.pbkdf2_hmac("sha256", password.encode(), base64.b64decode(cfg["salt"]), cfg["iter"], dklen=32)
    return text, gzip.decompress(AESGCM(key).decrypt(base64.b64decode(cfg["iv"]), base64.b64decode(cfg["ct"]), None)).decode("utf-8")


def _make_issue(tmp_path: Path, kind: str, label: str) -> publish.Issue:
    src = tmp_path / f"{kind}-{label}.html"
    src.write_text(
        f'<!DOCTYPE html><html><head><meta charset="utf-8"><title>{kind} {label}</title></head>'
        f"<body><header></header><main><h1>{kind} {label}</h1></main></body></html>",
        encoding="utf-8",
    )
    start, end = (publish.parse_daily_label(label),) * 2 if kind == "daily" else publish.parse_weekly_label(label)
    return publish.Issue(kind=kind, label=label, source=src, start=start, end=end, title=f"{kind} {label}")


def _pub_setup(tmp_path, password="synthetic-test-pass-0001", **extra):
    work, pw_file, salt_file = tmp_path / "work", tmp_path / "site-password", tmp_path / "site-salt"
    pw_file.write_text(password + "\n", encoding="utf-8")
    pw_file.chmod(0o600)
    env = dict(os.environ, DIGEST_ROOT=str(work), DIGEST_SITE_PASSWORD_FILE=str(pw_file), DIGEST_SITE_SALT_FILE=str(salt_file), DIGEST_TZ="UTC", **extra)
    for k in ("DIGEST_LANG", "SITE_SLUG", "DIGEST_LANG_ALT", "DIGEST_ROOT_LATEST"):
        if k not in extra:
            env.pop(k, None)
    run_pub = lambda *a, extra_env=None: subprocess.run(
        [sys.executable, str(ROOT / "publish.py"), *a], env=dict(env, **(extra_env or {})), capture_output=True, text=True, cwd=ROOT)
    return work, pw_file, env, run_pub


def test_config_init_overrides_and_validation(monkeypatch, tmp_path, capsys):
    cfg_path = tmp_path / "site" / "CONFIG"
    cfg_path.parent.mkdir(parents=True)
    monkeypatch.setattr(publish, "CONFIG_PATH", cfg_path)
    monkeypatch.setattr(publish, "CLI_LANG", None)
    clear = lambda *ks: [monkeypatch.delenv(k, raising=False) for k in ks]
    clear("DIGEST_LANG", "SITE_SLUG", "SITE_DOMAIN", "CF_PAGES_PROJECT", "CF_ACCOUNT_ID", "CF_PROJECT", "DIGEST_LANG_ALT", "DIGEST_ROOT_LATEST")

    monkeypatch.setenv("DIGEST_ROOT_LATEST", "1")
    assert publish.init_config_if_needed()["ROOT_LATEST"] == "1" and cfg_path.is_file()
    cfg_path.write_text("SLUG=abc12345\n", encoding="utf-8")
    assert publish.load_config()["LANG"] == "en"
    cfg_path.write_text("SLUG=abc12345\nLANG=zh-CN\n", encoding="utf-8")
    assert publish.load_config()["LANG"] == "zh-CN"
    for k, v in (("DIGEST_LANG", "en"), ("SITE_SLUG", "env12345"), ("SITE_DOMAIN", "d.example.org"), ("CF_PAGES_PROJECT", "proj")):
        monkeypatch.setenv(k, v)
    assert (cfg := publish.load_config())["LANG"] == "en" and cfg["SLUG"] == "env12345" and cfg["DOMAIN"] == "d.example.org" and cfg["PROJECT"] == "proj"
    clear("DIGEST_LANG", "SITE_SLUG", "SITE_DOMAIN", "CF_PAGES_PROJECT")
    publish.set_lang(None, None)

    for txt in ((ROOT / "site/CONFIG.example").read_text(encoding="utf-8"), "SLUG=<random-slug>\n", "SLUG={{secret}}\n",
                "SLUG=validslug123\nACCOUNT_ID=<CF_ACCOUNT_ID>\n", "SLUG=validslug123\nCF_PROJECT={{cf_proj}}\n",
                "SLUG=short77\n", "SLUG=bad/slash/slug\n", "SLUG=bad space\n", "SLUG=..\n"):
        cfg_path.write_text(txt, encoding="utf-8")
        with pytest.raises(SystemExit) as ex:
            publish.load_config()
        assert ex.value.code == 1 and "publish.py --init" in capsys.readouterr().err
    cfg_path.write_text("SLUG=validslug123\n", encoding="utf-8")
    monkeypatch.setenv("CF_ACCOUNT_ID", "<CF_ACCOUNT_ID>")
    with pytest.raises(SystemExit):
        publish.load_config()
    capsys.readouterr()
    clear("CF_ACCOUNT_ID")
    cfg_path.write_text("SLUG= Good_Slug-2026.~ok \nACCOUNT_ID=acc123\nPROJECT=my-digest\n", encoding="utf-8")
    assert publish.load_config()["SLUG"] == "Good_Slug-2026.~ok"


def test_privacy_scan_blocks_emails_phrases_custom_and_en_pane(monkeypatch, tmp_path):
    for name, body in (("e.html", "contact someone@example.com"), ("p.html", "<p>One of the people I follow on X said so</p>")):
        (f := tmp_path / name).write_text(body, encoding="utf-8")
        with pytest.raises(SystemExit):
            publish.privacy_scan([f])
    (pats := tmp_path / "pats.txt").write_text("secretword\tcustom\n", encoding="utf-8")
    (custom := tmp_path / "c.html").write_text("a secretword b", encoding="utf-8")
    monkeypatch.setenv("DIGEST_PRIVACY_PATTERNS_FILE", str(pats))
    with pytest.raises(SystemExit):
        publish.privacy_scan([custom])

    zh_md, en_md, bi = tmp_path / "d.md", tmp_path / "d.en.md", tmp_path / "bi.html"
    zh_md.write_text("**AI精选日报 · 1/15**\n\n## 🧠 模型\n\n### 1. 干净\n`示例 · 1/15`\n\n**一句话：** 无隐私。\n", encoding="utf-8")
    en_md.write_text("**AI Daily · 1/15**\n\n## 🧠 Models\n\n### 1. Clean\n`Example · 1/15`\n\n**In brief:** Contact leak@example.com from people I follow on X.\n", encoding="utf-8")
    rh.render_file(zh_md, bi, "zh-CN", en_md_path=en_md)
    with pytest.raises(SystemExit):
        publish.privacy_scan([bi])


def test_template_helpers_headlines_and_weekly_labels(tmp_path):
    tmpl = (ROOT / "site" / "password_template.html").read_text(encoding="utf-8")
    en, zh = publish.localize_template(tmpl, "en"), publish.localize_template(tmpl, "zh-CN")
    assert "{{" not in en and "{{" not in zh and "{{js:" not in en and '<html lang="en">' in en and "Password" in en and "密码" in zh and 'id="rm"' in en
    assert 'class="demo-hint"' not in en and 'class="demo-hint"' not in publish.localize_template(tmpl, "en", publish.shell_extras(""))
    assert all(s in tmpl for s in ('localStorage.getItem("digest-theme")', ':root:not([data-theme="light"])', ':root[data-theme="dark"]', "location.hash", "scrollIntoView"))
    assert "prefetch-src 'self'" in (th := (ROOT / "site" / "public" / "_headers").read_text(encoding="utf-8")) and "connect-src 'self'" in th

    (src := tmp_path / "2026-01-15.html").write_text('<article class="story"><h3><span class="story-no num">01</span> O&#x27;Reilly &amp; Co &mdash; <code>&lt;fast&gt;</code></h3></article>', encoding="utf-8")
    issue = publish.Issue(kind="daily", label="2026-01-15", source=src, start=date(2026, 1, 15), end=date(2026, 1, 15), title="t")
    assert publish.issue_headlines(issue) == ["O'Reilly & Co — <fast>"]
    assert publish.parse_weekly_label("2026-12-28_01-03") == publish.parse_weekly_label("2026-12-28_2027-01-03") == (date(2026, 12, 28), date(2027, 1, 3))


def test_demo_default_bilingual_build_and_globals_restore(monkeypatch, tmp_path):
    monkeypatch.setenv("DIGEST_DEMO_PASSWORD", "demo-password-123")
    monkeypatch.setenv("DIGEST_ROOT_LATEST", "1")
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    d, w, out = publish.DEMO_DAILY_LABEL, publish.DEMO_WEEKLY_LABEL, tmp_path / "dist"
    snap = lambda: (publish.SITE, publish.PUBLIC, publish.PLAIN, publish.SOURCES, publish.CONFIG_PATH, publish.ENC_CACHE_PATH,
                    publish._PASSWORD_OVERRIDE, publish._SALT_OVERRIDE, publish.CLI_LANG, publish.LANG_ALT, publish.LANG)
    before = snap()
    assert publish.main(["--demo", "--out", str(out)]) == 0 and snap() == before
    redirs = (out / "_redirects").read_text(encoding="utf-8").splitlines()
    assert redirs == ["/ /en/ 302"]
    for rel in ("latest.html", "daily/latest.html", "weekly/latest.html"):
        assert not (out / rel).exists()
    assert not (out / "demo").exists() and (out / "robots.txt").exists()
    headers = (out / "_headers").read_text(encoding="utf-8")
    assert headers.index("/en/daily/*") < headers.index("/en/daily/latest") and "/en/daily/latest\n  ! Cache-Control\n  Cache-Control: no-store" in headers

    shells, plains, salts = {}, {}, set()
    for lg in ("en", "zh-CN"):
        for rel in ("index.html", f"daily/{d}.html", f"weekly/{w}.html", "latest.html", "daily/latest.html", "weekly/latest.html"):
            shells[lg, rel], plains[lg, rel] = _decrypt(out / lg / rel)
            salts.add(re.search(r'"salt":"([^"]+)"', shells[lg, rel]).group(1))
    assert len(salts) == 1 and "Orion-3 Mini" not in shells["en", f"daily/{d}.html"]
    assert "Demo site · password: <code>demo-password-123</code>" in shells["en", "index.html"] and "演示站 · 密码：<code>demo-password-123</code>" in shells["zh-CN", "index.html"]
    assert "Daily · by week" in plains["en", "index.html"] and "Why it matters" in plains["en", f"daily/{d}.html"]
    assert "日报 · 按周" in plains["zh-CN", "index.html"] and "值得看" in plains["zh-CN", f"daily/{d}.html"]
    assert 'href="/en/"' in plains["en", "latest.html"] and f'href="/en/weekly/{w}"' in plains["en", "daily/latest.html"]
    assert 'href="/zh-CN/"' in plains["zh-CN", "latest.html"] and 'href="/zh-CN/"' in plains["zh-CN", "weekly/latest.html"]
    for rel, href_en, href_zh in (
        ("index.html", "/zh-CN/", "/en/"),
        (f"daily/{d}.html", f"/zh-CN/daily/{d}", f"/en/daily/{d}"),
        ("latest.html", "/zh-CN/latest", "/en/latest"),
        ("daily/latest.html", "/zh-CN/daily/latest", "/en/daily/latest"),
        ("weekly/latest.html", "/zh-CN/weekly/latest", "/en/weekly/latest"),
    ):
        assert f'href="{href_en}" hreflang="zh-CN"' in shells["en", rel] and f'href="{href_en}" hreflang="zh-CN"' in plains["en", rel]
        assert f'href="{href_zh}" hreflang="en"' in shells["zh-CN", rel] and f'href="{href_zh}" hreflang="en"' in plains["zh-CN", rel]


def test_demo_single_lang_cli_and_missing_password(monkeypatch, tmp_path):
    d, w, out = publish.DEMO_DAILY_LABEL, publish.DEMO_WEEKLY_LABEL, tmp_path / "dist"
    env = dict(os.environ, DIGEST_DEMO_PASSWORD="demo-password-123", SITE_SLUG="demo", DIGEST_ROOT_LATEST="1")
    env.pop("DIGEST_LANG", None)
    assert subprocess.run([sys.executable, str(ROOT / "publish.py"), "--demo", "--lang", "zh-CN", "--out", str(out)],
                          env=env, capture_output=True, text=True, cwd=ROOT).returncode == 0
    redirs = (out / "_redirects").read_text(encoding="utf-8").splitlines()
    assert redirs == ["/ /demo/ 302"]
    assert (out / "demo" / "latest.html").is_file() and (out / "demo" / "daily" / "latest.html").is_file() and (out / "demo" / "weekly" / "latest.html").is_file()
    assert not (out / "latest.html").exists() and not (out / "daily" / "latest.html").exists() and not (out / "zh-CN").exists() and (out / "robots.txt").exists()
    sh_idx, pl_idx = _decrypt(out / "demo" / "index.html")
    sh_day, pl_day = _decrypt(out / "demo" / "daily" / f"{d}.html")
    _, pl_latest = _decrypt(out / "demo" / "latest.html")
    assert "hreflang" not in sh_idx and "hreflang" not in pl_idx and "本周周报" in pl_idx
    assert '<html lang="zh-CN">' in sh_day and "记住此设备" in sh_day and "返回目录" in pl_day and "Orion-3 Mini" in pl_day
    assert 'href="/demo/"' in pl_latest and f'href="/demo/weekly/{w}"' in pl_latest and "Orion-3 Mini" in pl_latest
    monkeypatch.delenv("DIGEST_DEMO_PASSWORD", raising=False)
    with pytest.raises(SystemExit):
        publish.main(["--demo", "--out", str(tmp_path / "nopw")])


def test_incremental_publish_cache_verify_and_latest_cli(tmp_path):
    work, pw_file, env, run_pub = _pub_setup(tmp_path, SITE_SLUG="testslug", SITE_DOMAIN="digest.example.com")
    r1 = run_pub("--issue", "daily/2026-01-14", "--md", "examples/sample-digest.en.md", "--lang", "en", "--prepare-only")
    assert r1.returncode == 0 and "deploy: skipped (--prepare-only)" in r1.stdout and (work / "site" / "enc-cache.json").is_file()
    pub = work / "site" / "public"
    assert not (pub / "_redirects").exists()
    assert (pub / "testslug" / "daily" / "latest.html").is_file() and (pub / "testslug" / "latest.html").is_file()
    assert not (pub / "latest.html").exists() and not (pub / "daily" / "latest.html").exists()

    r2 = run_pub("--issue", "daily/2026-01-14", "--md", "examples/sample-digest.en.md", "--lang", "en", "--prepare-only")
    assert r2.returncode == 0 and "changed pages: none" in r2.stdout
    v1 = subprocess.run(["node", str(ROOT / "verify_decrypt.mjs"), "--local"], env=env, capture_output=True, text=True, cwd=ROOT)
    assert v1.returncode == 0 and "all 4 page(s) decrypted and matched plaintext" in v1.stdout

    pw_file.write_text("synthetic-test-pass-0002\n", encoding="utf-8")
    r3 = run_pub("--issue", "daily/2026-01-14", "--lang", "en", "--prepare-only")
    assert r3.returncode == 0 and "changed pages: none" not in r3.stdout
    assert _decrypt(pub / "testslug" / "daily" / "2026-01-14.html", "synthetic-test-pass-0002")[1] == (
        work / "site" / "plain" / "testslug" / "daily" / "2026-01-14.html").read_text(encoding="utf-8")

    r4 = run_pub("--issue", "daily/2026-01-15", "--md", "examples/sample-digest.en.md", "--lang", "zh-CN", "--prepare-only", extra_env={"DIGEST_ROOT_LATEST": "1"})
    assert r4.returncode == 0 and "changed pages: none" not in r4.stdout
    assert '<html lang="zh-CN">' in (pub / "testslug" / "daily" / "2026-01-15.html").read_text(encoding="utf-8")
    assert not (pub / "_redirects").exists()
    sh_root, pl_root = _decrypt(pub / "daily" / "latest.html", "synthetic-test-pass-0002")
    assert "testslug" not in sh_root and 'href="/testslug/"' in pl_root and 'href="/testslug/daily/2026-01-14"' in pl_root
    v2 = subprocess.run(["node", str(ROOT / "verify_decrypt.mjs"), "--local"], env=env, capture_output=True, text=True, cwd=ROOT)
    assert v2.returncode == 0 and "all 7 page(s) decrypted and matched plaintext" in v2.stdout
    headers = (pub / "_headers").read_text(encoding="utf-8")
    assert all(h in headers for h in ("X-Frame-Options: DENY", "frame-ancestors 'none'", "connect-src 'self'", "prefetch-src 'self'", "\n/daily/latest\n  Cache-Control: no-store\n"))


def test_bilingual_incremental_rerender_lang_alt_and_nav_idempotency(monkeypatch, tmp_path):
    (md := tmp_path / "digest-2026-01-15.md").write_text("x", encoding="utf-8")
    (en := tmp_path / "digest-2026-01-15.en.md").write_text("y", encoding="utf-8")
    monkeypatch.setattr(publish, "ROOT", tmp_path)
    monkeypatch.setattr(publish, "LANG", "zh-CN")
    assert publish._discover_en_md(md) == en and publish._discover_en_md(en) is None
    monkeypatch.setattr(publish, "LANG", "en")
    assert publish._discover_en_md(md) is None and publish._discover_en_md(md, en) == en
    with pytest.raises(SystemExit):
        publish._discover_en_md(md, tmp_path / "missing.en.md")

    bi = tmp_path / "bi.html"
    rh.render_file(ROOT / "examples" / "sample-digest.zh-CN.md", bi, "zh-CN", year=2026, en_md_path=ROOT / "examples" / "sample-digest.en.md")
    raw = bi.read_text(encoding="utf-8")
    issue = publish.Issue(kind="daily", label="2026-01-15", source=bi, start=date(2026, 1, 15), end=date(2026, 1, 15), title=publish.extract_title(raw))
    monkeypatch.setattr(publish, "LANG", "zh-CN")
    monkeypatch.setattr(publish, "LANG_ALT", None)
    once = publish.inject_nav(raw, issue, [issue])
    assert once == publish.inject_nav(once, issue, [issue]) and once.count(publish.NAV_TOP_START) == once.count(publish.NAV_BOT_START) == 2
    zh_p, en_p = once.split('<div class="lang-pane lang-en"', 1)
    assert "返回目录" in zh_p and "Back to index" not in zh_p and "Back to index" in en_p and "返回目录" not in en_p
    assert publish.strip_injections(once) == raw and len(publish.issue_headlines(issue)) == 10
    assert "开放权重 / 推理" in publish.issue_tags(issue) and "Open weights / Reasoning" not in publish.issue_tags(issue)

    work, _, _, run_pub = _pub_setup(tmp_path, password="synthetic-test-pass-7777")
    rd = lambda rel: (work / "site" / rel).read_text(encoding="utf-8")
    (work / "site").mkdir(parents=True)
    (work / "site" / "CONFIG").write_text("SLUG=bislug1234\nLANG=zh-CN\nLANG_ALT=en:https://digest.example.com/enslug1234/\n", encoding="utf-8")
    for dst, src in (("digest-2026-01-15.md", "sample-digest.zh-CN.md"), ("digest-2026-01-15.en.md", "sample-digest.en.md"),
                     ("digest-2026-01-14.md", "sample-digest.zh-CN.md"), ("digest-2026-01-15-draft.md", "sample-digest.zh-CN.md"), ("digest-2026-01-16.en.md", "sample-digest.en.md")):
        shutil.copy(ROOT / "examples" / src, work / dst)

    r1 = run_pub("--issue", "daily/2026-01-15", "--md", "digest-2026-01-15.md", "--prepare-only")
    assert r1.returncode == 0 and "published: daily/2026-01-15 (10 items) + English (digest-2026-01-15.en.md)" in r1.stdout
    plain15, plain_idx = rd("plain/bislug1234/daily/2026-01-15.html"), rd("plain/bislug1234/index.html")
    shell15, dec15 = _decrypt(work / "site/public/bislug1234/daily/2026-01-15.html", "synthetic-test-pass-7777")
    ids = re.findall(r'\bid="([^"]+)"', plain15)
    assert dec15 == plain15 and len(ids) == len(set(ids)) and 'id="en-digestFilterBar"' in plain15 and "10 条" in plain_idx and "20 条" not in plain_idx
    assert 'id="archiveFilterBar"' in plain_idx and 'href="https://digest.example.com/enslug1234/" hreflang="en"' in plain_idx
    assert all('href="https://digest.example.com/enslug1234/daily/2026-01-15" hreflang="en"' in s for s in (plain15, shell15))

    assert run_pub("--issue", "daily/2026-01-14", "--md", "digest-2026-01-14.md", "--lang", "en", "--prepare-only").returncode == 0
    assert '<div class="lang-pane' not in rd("plain/bislug1234/daily/2026-01-14.html") and run_pub("--rerender-all", "--prepare-only").returncode == 0
    assert sorted(p.name for p in (work / "site/sources/daily").glob("*.html")) == ["2026-01-14.html", "2026-01-15.html"]
    assert '<html lang="zh-CN">' in (p14 := rd("plain/bislug1234/daily/2026-01-14.html")) and "值得看" in p14 and "Why it matters" not in p14
    assert rd("plain/bislug1234/daily/2026-01-15.html").count(publish.NAV_TOP_START) == 2

    shutil.copy(ROOT / "examples/sample-digest.en.md", custom_en := work / "alt-en.md")
    assert "+ English (alt-en.md)" in run_pub("--issue", "daily/2026-01-14", "--md", "digest-2026-01-14.md", "--md-en", str(custom_en), "--prepare-only").stdout
    r_bad = run_pub("--issue", "daily/2026-01-14", "--md-en", str(custom_en), "--prepare-only")
    assert r_bad.returncode != 0 and "--md-en requires" in r_bad.stderr


def test_static_latest_pages_root_opt_in_and_removed_paths(monkeypatch, tmp_path):
    site, slug = tmp_path / "site", "privateslug01"
    for attr, path in (("SITE", site), ("PUBLIC", site / "public"), ("PLAIN", site / "plain"), ("SOURCES", site / "sources"), ("CONFIG_PATH", site / "CONFIG"), ("ENC_CACHE_PATH", site / "enc-cache.json")):
        monkeypatch.setattr(publish, attr, path)
    pw_file, salt_file = tmp_path / "pw", tmp_path / "salt"
    pw_file.write_text("synthetic-test-pass-latest\n", encoding="utf-8")
    monkeypatch.setenv("DIGEST_SITE_PASSWORD_FILE", str(pw_file))
    monkeypatch.setenv("DIGEST_SITE_SALT_FILE", str(salt_file))
    for k in ("DIGEST_ROOT_LATEST", "SITE_SLUG", "DIGEST_LANG_ALT"):
        monkeypatch.delenv(k, raising=False)
    site.mkdir(parents=True)
    (site / "CONFIG").write_text(f"SLUG={slug}\nLANG=en\nLANG_ALT=zh-CN:https://zh.example.com/altsecret01/\n", encoding="utf-8")
    assert publish.load_config()["ROOT_LATEST"] == "0"
    hdrs = lambda: (site / "public/_headers").read_text(encoding="utf-8")
    d1, d2 = _make_issue(tmp_path, "daily", "2026-01-14"), _make_issue(tmp_path, "daily", "2026-01-15")
    w1, w2 = _make_issue(tmp_path, "weekly", "2026-01-12_01-18"), _make_issue(tmp_path, "weekly", "2026-01-19_01-25")

    def build_and_encrypt():
        cfg = publish.load_config()
        publish.rebuild_all(cfg)
        publish.privacy_scan(publish.iter_plain_files() + publish.iter_unencrypted_public())
        publish.encrypt_all_pages(cfg)
        publish.assert_public_is_encrypted(cfg)
        return cfg

    publish.save_source(d1.source, "daily", "2026-01-14")
    build_and_encrypt()
    assert not (site / "public/_redirects").exists()
    assert (site / "public" / slug / "daily/latest.html").is_file() and (site / "public" / slug / "latest.html").is_file()
    assert not (site / "public" / slug / "weekly/latest.html").exists() and not (site / "public/latest.html").exists()
    assert hdrs().index(f"/{slug}/daily/*") < hdrs().index(f"/{slug}/daily/latest") and f"/{slug}/daily/latest\n  ! Cache-Control\n  Cache-Control: no-store\n" in hdrs() and "\n/latest\n" not in hdrs()
    _, pl_d1 = _decrypt(site / "public" / slug / "daily/latest.html", "synthetic-test-pass-latest")
    assert "2026-01-14" in pl_d1 and f'href="/{slug}/"' in pl_d1 and 'href="https://zh.example.com/altsecret01/daily/latest"' in pl_d1

    publish.save_source(d2.source, "daily", "2026-01-15")
    publish.save_source(w1.source, "weekly", "2026-01-12_01-18")
    publish.save_source(w2.source, "weekly", "2026-01-19_01-25")
    build_and_encrypt()
    _, pl_d2 = _decrypt(site / "public" / slug / "daily/latest.html", "synthetic-test-pass-latest")
    _, pl_w2 = _decrypt(site / "public" / slug / "weekly/latest.html", "synthetic-test-pass-latest")
    _, pl_any = _decrypt(site / "public" / slug / "latest.html", "synthetic-test-pass-latest")
    assert "2026-01-15" in pl_d2 and f'href="/{slug}/daily/2026-01-14"' in pl_d2 and f'href="/{slug}/weekly/2026-01-12_01-18"' in pl_d2
    assert "2026-01-19_01-25" in pl_w2 and f'href="/{slug}/weekly/2026-01-12_01-18"' in pl_w2 and "2026-01-15" in pl_any

    # Weekly-only fallback: /<slug>/latest serves latest weekly when no dailies exist
    for p in (site / "sources/daily").glob("*.html"):
        p.unlink()
    build_and_encrypt()
    assert not (site / "public" / slug / "daily/latest.html").exists()
    assert "2026-01-19_01-25" in _decrypt(site / "public" / slug / "latest.html", "synthetic-test-pass-latest")[1]

    # Opt-in ROOT_LATEST creates root encrypted latest pages without leaking SLUG or LANG_ALT in shells
    publish.save_source(d2.source, "daily", "2026-01-15")
    for val in ("1", "true", "YES", "On"):
        publish.write_config({"SLUG": slug, "LANG": "en", "LANG_ALT": "zh-CN:https://zh.example.com/altsecret01/", "ROOT_LATEST": val})
        assert publish.load_config()["ROOT_LATEST"] == "1"
    build_and_encrypt()
    assert "\n/latest\n  Cache-Control: no-store\n" in hdrs()
    for rel, needle in (("latest.html", "2026-01-15"), ("daily/latest.html", "2026-01-15"), ("weekly/latest.html", "2026-01-19_01-25")):
        sh, pl = _decrypt(site / "public" / rel, "synthetic-test-pass-latest")
        assert slug not in sh and "altsecret01" not in sh and needle in pl and f'href="/{slug}/"' in pl

    # Turning DIGEST_ROOT_LATEST back to 0 prunes root latest files cleanly
    monkeypatch.setenv("DIGEST_ROOT_LATEST", "0")
    build_and_encrypt()
    for rel in ("latest.html", "daily/latest.html", "weekly/latest.html"):
        assert not (site / "public" / rel).exists() and not (site / "plain" / rel).exists()

    # Env override DIGEST_ROOT_LATEST=1 works even when CONFIG omits ROOT_LATEST
    publish.write_config({"SLUG": slug, "LANG": "en"})
    monkeypatch.setenv("DIGEST_ROOT_LATEST", "1")
    build_and_encrypt()
    assert (site / "public/latest.html").is_file() and (site / "public/daily/latest.html").is_file()

    # removed-paths.txt skips active latest aliases (cannot hijack static latest pages)
    monkeypatch.delenv("DIGEST_ROOT_LATEST")
    (site / "CONFIG").unlink()
    (site / "removed-paths.txt").write_text("daily/2026-01-10\ndaily/2026-01-10\ndaily/latest\ndaily/latest.html\n", encoding="utf-8")
    publish.write_static_root(slug=slug)
    assert (site / "public/_redirects").read_text(encoding="utf-8").splitlines() == [
        f"/{slug}/daily/2026-01-10 /{slug}/ 302",
        f"/{slug}/daily/2026-01-10.html /{slug}/ 302",
    ]
    publish.privacy_scan(publish.iter_unencrypted_public())
    (site / "removed-paths.txt").unlink()
    publish.write_static_root(slug=slug, issues=[])
    assert not (site / "public/_redirects").exists()


def test_deploy_via_pages_api_uploads_redirects_and_excludes_control_files(monkeypatch, tmp_path):
    (pub := tmp_path / "public").mkdir()
    (pub / "_headers").write_text("/*\n  X-Frame-Options: DENY\n", encoding="utf-8")
    (pub / "_redirects").write_text("/slug1234/daily/2026-01-10 /slug1234/ 302\n", encoding="utf-8")
    (pub / "robots.txt").write_text("User-agent: *\nDisallow: /\n", encoding="utf-8")
    (pub / "slug1234").mkdir()
    (pub / "slug1234" / "latest.html").write_text("enc-latest", encoding="utf-8")
    monkeypatch.setattr(publish, "PUBLIC", pub)
    forms: list[bytes] = []

    def fake_cf_request(token, method, path, body=None, form=None, form_content_type=None):
        if path.endswith("/deployments"):
            forms.append(form)
            return {"success": True, "result": {"url": "https://example.pages.dev"}}
        return {"/upload-token": {"result": {"jwt": "synthetic-jwt"}}, "/check-missing": {"result": []},
                "/upsert-hashes": {"success": True}}[next(k for k in ("/upload-token", "/check-missing", "/upsert-hashes") if path.endswith(k))]

    monkeypatch.setattr(publish, "cf_request", fake_cf_request)
    cfg = {"ACCOUNT_ID": "acc123", "PROJECT": "proj123", "SLUG": "slug1234"}
    assert publish._deploy_via_pages_api(cfg, "tok", "deploy 1")["via"] == "pages-api"
    payload = forms[-1].decode("utf-8")
    assert 'name="_headers"; filename="_headers"' in payload and 'name="_redirects"; filename="_redirects"' in payload and "/slug1234/daily/2026-01-10 /slug1234/ 302" in payload
    manifest = json.loads(re.search(r'name="manifest"[^\r\n]*\r\n(?:[^\r\n]+\r\n)*\r\n(\{[^\r\n]+\})', payload).group(1))
    assert "/robots.txt" in manifest and "/slug1234/latest.html" in manifest and "/_headers" not in manifest and "/_redirects" not in manifest
    (pub / "_redirects").unlink()
    publish._deploy_via_pages_api(cfg, "tok", "deploy 2")
    assert 'name="_headers"; filename="_headers"' in (p2 := forms[-1].decode("utf-8")) and 'name="_redirects"' not in p2


def test_reader_ux_index_nav_and_unlock_shell_js_contracts(tmp_path):
    import subprocess

    work, _, _, run_pub = _pub_setup(tmp_path, password="synthetic-test-pass-7777")
    (work / "site").mkdir(parents=True)
    (work / "site" / "CONFIG").write_text("SLUG=uxslug1234\nLANG=zh-CN\n", encoding="utf-8")
    shutil.copy(ROOT / "examples" / "sample-digest.zh-CN.md", work / "digest-2026-01-14.md")
    shutil.copy(ROOT / "examples" / "sample-digest.zh-CN.md", work / "digest-2026-01-15.md")
    shutil.copy(ROOT / "examples" / "sample-digest.en.md", work / "digest-2026-01-15.en.md")
    assert run_pub("--issue", "daily/2026-01-14", "--md", "digest-2026-01-14.md", "--prepare-only").returncode == 0
    assert run_pub("--issue", "daily/2026-01-15", "--md", "digest-2026-01-15.md", "--prepare-only").returncode == 0

    plain15 = (work / "site/plain/uxslug1234/daily/2026-01-15.html").read_text(encoding="utf-8")
    plain_idx = (work / "site/plain/uxslug1234/index.html").read_text(encoding="utf-8")
    shell15 = (work / "site/public/uxslug1234/daily/2026-01-15.html").read_text(encoding="utf-8")

    # Top issue nav rides inside sticky <header class="mast"> with semantic data-nav links
    m_hdr = re.search(r'<header class="mast"[^>]*>.*?</header>', plain15, re.S)
    assert m_hdr and 'class="issue-nav top"' in m_hdr.group(0)
    assert 'data-nav="index"' in m_hdr.group(0) and 'data-nav="prev"' in m_hdr.group(0)
    assert 'data-lang-toggle' in plain15 and 'data-theme-toggle' in plain15 and 'data-copy-anchor' in plain15

    # Index shell uses single theme toggle, quick-jump latest pill, and content-visibility
    assert 'data-theme-toggle' in plain_idx and 'href="daily/latest"' in plain_idx
    assert 'content-visibility: auto' in plain_idx and 'backdrop-filter:' in plain_idx

    # Unlock shell includes spring loading/error feedback hooks
    assert "is-loading" in shell15 and "is-shake" in shell15 and 'class="spin"' in shell15

    # Single unified theme-color meta tag on both reader and index pages
    assert plain15.count('<meta name="theme-color"') == 1 and plain_idx.count('<meta name="theme-color"') == 1

    # CSS minifier shrinks raw stylesheet, is idempotent, and preserves modern motion/color rules
    import render_html
    raw_style = f"<style>{render_html._load_css()}</style>"
    min_css = publish.minify_css(raw_style)
    assert len(min_css) < len(raw_style) and publish.minify_css(min_css) == min_css
    for tok in ("@keyframes", "cubic-bezier(", "color-mix(", "backdrop-filter:", "content-visibility:", "calc("):
        assert tok in min_css and tok in plain15

    # Every inline <script> across issue, index, and unlock shell must parse cleanly
    for label, html_doc in (("issue", plain15), ("index", plain_idx), ("shell", shell15)):
        scripts = re.findall(r"<script([^>]*)>(.*?)</script>", html_doc, re.S)
        assert scripts, f"no scripts in {label}"
        for i, (attrs, body) in enumerate(scripts):
            if "speculationrules" in attrs or "application/json" in attrs:
                json.loads(body)
                continue
            js_file = tmp_path / f"{label}_{i}.js"
            js_file.write_text(body, encoding="utf-8")
            chk = subprocess.run(["node", "--check", str(js_file)], capture_output=True, text=True)
            assert chk.returncode == 0, f"JS syntax error in {label}[{i}]: {chk.stderr}"

