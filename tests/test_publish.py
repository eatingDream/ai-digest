import os
import re
import subprocess
import sys
from pathlib import Path

import publish

ROOT = Path(__file__).resolve().parents[1]


def test_config_env_overrides(monkeypatch, tmp_path):
    monkeypatch.setattr(publish, "CONFIG_PATH", tmp_path / "CONFIG")
    monkeypatch.setenv("SITE_SLUG", "abc123")
    monkeypatch.setenv("SITE_DOMAIN", "d.example.org")
    monkeypatch.setenv("CF_PAGES_PROJECT", "proj")
    cfg = publish.load_config()
    assert cfg["SLUG"] == "abc123" and cfg["DOMAIN"] == "d.example.org" and cfg["PROJECT"] == "proj"


def test_privacy_scan_flags_email_and_custom(monkeypatch, tmp_path):
    f = tmp_path / "p.html"
    f.write_text("contact someone@example.com", encoding="utf-8")
    try:
        publish.privacy_scan([f])
        raise AssertionError("expected SystemExit")
    except SystemExit:
        pass
    pats = tmp_path / "pats.txt"
    pats.write_text("secretword\tcustom\n", encoding="utf-8")
    monkeypatch.setenv("DIGEST_PRIVACY_PATTERNS_FILE", str(pats))
    g = tmp_path / "q.html"
    g.write_text("a secretword b", encoding="utf-8")
    try:
        publish.privacy_scan([g])
        raise AssertionError("expected SystemExit")
    except SystemExit:
        pass


def _build_demo(tmp_path, *extra, lang_env=None):
    env = dict(os.environ, DIGEST_DEMO_PASSWORD="demo-password-123", SITE_SLUG="demo")
    env.pop("DIGEST_LANG", None)
    if lang_env:
        env["DIGEST_LANG"] = lang_env
    out = tmp_path / "dist"
    r = subprocess.run([sys.executable, str(ROOT / "publish.py"), "--demo", "--out", str(out), *extra],
                       env=env, capture_output=True, text=True, cwd=str(ROOT))
    assert r.returncode == 0, r.stderr
    return out


def _decrypt(page_path, password="demo-password-123"):
    import base64, gzip, hashlib, json
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    text = page_path.read_text(encoding="utf-8")
    i = text.index('id="c">') + len('id="c">')
    cfg = json.loads(text[i:text.index("</script>", i)])
    key = hashlib.pbkdf2_hmac("sha256", password.encode(), base64.b64decode(cfg["salt"]), cfg["iter"], dklen=32)
    packed = AESGCM(key).decrypt(base64.b64decode(cfg["iv"]), base64.b64decode(cfg["ct"]), None)
    return text, gzip.decompress(packed).decode("utf-8")


def test_demo_build_end_to_end_single_english(tmp_path):
    out = _build_demo(tmp_path, "--lang", "en")
    shell, plain = _decrypt(out / "demo" / "daily" / "2026-01-15.html")
    assert '"ct":' in shell and "Orion-3 Mini" not in shell  # encrypted
    assert '<html lang="en">' in shell and "Remember this device" in shell and "Wrong password" in shell
    assert '<html lang="en">' in plain and "Back to index" in plain and "Why it matters" in plain
    _, index = _decrypt(out / "demo" / "index.html")
    assert "Daily · by week" in index and "Weekly · by month" in index and "This week's recap" in index
    assert (out / "_redirects").read_text().startswith("/ /demo/")
    assert (out / "robots.txt").exists()


def test_demo_build_chinese(tmp_path):
    out = _build_demo(tmp_path, "--lang", "zh-CN")
    shell, plain = _decrypt(out / "demo" / "daily" / "2026-01-15.html")
    assert '<html lang="zh-CN">' in shell and "记住此设备" in shell and "密码不对" in shell
    assert "返回目录" in plain and "值得看" in plain and "Orion-3 Mini" in plain
    _, index = _decrypt(out / "demo" / "index.html")
    assert "日报 · 按周" in index and "周报 · 按月" in index and "本周周报" in index


def test_demo_default_builds_both_languages(tmp_path):
    out = _build_demo(tmp_path)  # no --lang, no $DIGEST_LANG -> English + Chinese in one tree
    assert (out / "_redirects").read_text().strip() == "/ /en/ 302"
    assert not (out / "demo").exists()
    shells, plains, salts = {}, {}, set()
    for lg in ("en", "zh-CN"):
        for rel in ("index.html", "daily/2026-01-15.html", "weekly/2026-01-12_01-18.html"):
            shell, plain = _decrypt(out / lg / rel)
            shells[lg, rel], plains[lg, rel] = shell, plain
            salts.add(re.search(r'"salt":"([^"]+)"', shell).group(1))
    assert len(salts) == 1  # one salt: unlocking one language keeps the device unlocked for the other
    assert '<html lang="en">' in plains["en", "index.html"] and '<html lang="zh-CN">' in plains["zh-CN", "index.html"]
    # switch links: password page, index and issue pages, pointing at the same page in the other language
    for rel, href_en, href_zh in (("index.html", "/zh-CN/", "/en/"),
                                  ("daily/2026-01-15.html", "/zh-CN/daily/2026-01-15", "/en/daily/2026-01-15")):
        assert f'href="{href_en}" hreflang="zh-CN"' in shells["en", rel]
        assert f'href="{href_en}" hreflang="zh-CN"' in plains["en", rel]
        assert f'href="{href_zh}" hreflang="en"' in shells["zh-CN", rel]
        assert f'href="{href_zh}" hreflang="en"' in plains["zh-CN", rel]
    assert "Why it matters" in plains["en", "daily/2026-01-15.html"]
    assert "值得看" in plains["zh-CN", "daily/2026-01-15.html"]
    headers = (out / "_headers").read_text()
    assert "/en/daily/*" in headers and "/zh-CN/daily/*" in headers


def test_demo_lang_all_flag_and_env_single(tmp_path):
    assert (_build_demo(tmp_path, "--lang", "all") / "zh-CN" / "index.html").is_file()
    (tmp_path / "e").mkdir()
    out = _build_demo(tmp_path / "e", lang_env="en")  # explicit language -> single site at /demo/
    assert (out / "demo" / "index.html").is_file() and not (out / "zh-CN").exists()


def test_single_language_site_has_no_switch_link(tmp_path):
    out = _build_demo(tmp_path, "--lang", "en")
    shell, plain = _decrypt(out / "demo" / "index.html")
    assert "hreflang" not in shell and "hreflang" not in plain


def test_demo_language_from_env_and_cli_precedence(tmp_path):
    out = _build_demo(tmp_path, lang_env="zh-CN")
    assert "返回目录" in _decrypt(out / "demo" / "daily" / "2026-01-15.html")[1]
    (tmp_path / "b").mkdir()
    out2 = _build_demo(tmp_path / "b", "--lang", "en", lang_env="zh-CN")
    assert "Back to index" in _decrypt(out2 / "demo" / "daily" / "2026-01-15.html")[1]


def test_config_lang_key_and_default(monkeypatch, tmp_path):
    cfg_path = tmp_path / "CONFIG"
    monkeypatch.setattr(publish, "CONFIG_PATH", cfg_path)
    monkeypatch.setattr(publish, "CLI_LANG", None)
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    cfg_path.write_text("SLUG=abc\n", encoding="utf-8")
    assert publish.load_config()["LANG"] == "en"
    cfg_path.write_text("SLUG=abc\nLANG=zh-CN\n", encoding="utf-8")
    assert publish.load_config()["LANG"] == "zh-CN"
    monkeypatch.setenv("DIGEST_LANG", "en")
    assert publish.load_config()["LANG"] == "en"
    publish.set_lang(None, None)


def test_localized_password_template():
    tmpl = (ROOT / "site" / "password_template.html").read_text(encoding="utf-8")
    en = publish.localize_template(tmpl, "en")
    zh = publish.localize_template(tmpl, "zh-CN")
    assert "{{" not in en and "{{" not in zh
    assert '<html lang="en">' in en and "{{js:" not in en
    assert "Password" in en and "密码" in zh and 'id="rm"' in en and 'id="rm"' in zh


def test_privacy_scan_english_circle_phrases(tmp_path):
    f = tmp_path / "p.html"
    f.write_text("<p>One of the people I follow on X said so</p>", encoding="utf-8")
    try:
        publish.privacy_scan([f])
        raise AssertionError("expected SystemExit")
    except SystemExit:
        pass


def test_demo_requires_password(tmp_path):
    env = {k: v for k, v in os.environ.items() if k != "DIGEST_DEMO_PASSWORD"}
    r = subprocess.run([sys.executable, str(ROOT / "publish.py"), "--demo", "--out", str(tmp_path / "x")],
                       env=env, capture_output=True, text=True, cwd=str(ROOT))
    assert r.returncode != 0


def test_demo_unlock_page_shows_the_public_password_hint(tmp_path):
    out = _build_demo(tmp_path)
    en, _ = _decrypt(out / "en" / "index.html")
    zh, _ = _decrypt(out / "zh-CN" / "index.html")
    assert "Demo site · password: <code>demo-password-123</code>" in en
    assert "演示站 · 密码：<code>demo-password-123</code>" in zh


def test_live_template_has_no_demo_hint():
    tmpl = (ROOT / "site" / "password_template.html").read_text(encoding="utf-8")
    assert 'class="demo-hint"' not in publish.localize_template(tmpl, "en")  # no extras -> empty (live pipeline)
    assert 'class="demo-hint"' not in publish.localize_template(tmpl, "en", publish.shell_extras(""))  # no demo password set
