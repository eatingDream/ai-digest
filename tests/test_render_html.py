from pathlib import Path

import pytest

import i18n
import render_html as rh

ROOT = Path(__file__).resolve().parents[1]
EX = ROOT / "examples"
SAMPLES = {
    "en": (EX / "sample-digest.en.md", EX / "sample-weekly.en.md"),
    "zh-CN": (EX / "sample-digest.zh-CN.md", EX / "sample-weekly.zh-CN.md"),
}


@pytest.mark.parametrize("lang", ["en", "zh-CN"])
def test_parse_daily_sample(lang):
    d = rh.parse_digest(SAMPLES[lang][0].read_text(encoding="utf-8"), lang)
    assert d.title == ("AI Daily" if lang == "en" else "AI精选日报")
    assert d.period == "1/15"
    assert len(d.items) == 10
    assert len(d.sections) == 7
    assert d.items[0].number == "1"
    # both languages map onto the same canonical fields
    assert set(d.items[0].fields) == {"dek", "look", "detail"}


@pytest.mark.parametrize("lang", ["en", "zh-CN"])
def test_parse_weekly_sample_has_front_matter(lang):
    d = rh.parse_digest(SAMPLES[lang][1].read_text(encoding="utf-8"), lang)
    assert i18n.looks_weekly(d.title)
    assert d.must and d.themes
    assert len(d.items) == 3
    assert "followup" in d.items[0].fields


def test_section_anchors_match_across_languages():
    en = rh.parse_digest(SAMPLES["en"][0].read_text(encoding="utf-8"), "en")
    zh = rh.parse_digest(SAMPLES["zh-CN"][0].read_text(encoding="utf-8"), "zh-CN")
    assert [rh._section_id(s.title) for s in en.sections] == [rh._section_id(s.title) for s in zh.sections]
    assert [rh._section_id(s.title) for s in en.sections] == [
        "sec-models", "sec-papers", "sec-tools", "sec-industry", "sec-infra", "sec-policy", "sec-voices"]


@pytest.mark.parametrize("lang,needles,absent", [
    ("en", ['<html lang="en">', "Contents", "Why it matters", "Light", "End of issue", "Thursday, January 15, 2026"],
     ["值得看", "本期", "目录"]),
    ("zh-CN", ['<html lang="zh-CN">', "目录", "值得看", "浅色", "（本期完）", "2026 年 1 月 15 日 · 星期四"],
     ["Why it matters", "End of issue"]),
])
def test_render_daily_ui_language(tmp_path, lang, needles, absent):
    out = tmp_path / "d.html"
    digest = rh.render_file(SAMPLES[lang][0], out, lang, year=2026)
    html = out.read_text(encoding="utf-8")
    assert html.lstrip().lower().startswith("<!doctype html")
    assert len(digest.items) == 10 and "Orion-3 Mini" in html
    for n in needles:
        assert n in html, n
    for n in absent:
        assert n not in html, n
    assert '<link rel="stylesheet"' not in html  # self-contained


def test_render_weekly_en_front_page(tmp_path):
    out = tmp_path / "w.html"
    rh.render_file(SAMPLES["en"][1], out, "en", year=2026)
    html = out.read_text(encoding="utf-8")
    for n in ("Must read this week", "Themes of the week", "Follow-up", "Weekly", "Jan 12 – Jan 18, 2026"):
        assert n in html, n


def test_default_language_is_english(tmp_path, monkeypatch):
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    out = tmp_path / "d.html"
    rh.render_file(SAMPLES["en"][0], out)
    assert '<html lang="en">' in out.read_text(encoding="utf-8")


def test_env_and_cli_language(tmp_path, monkeypatch):
    monkeypatch.setenv("DIGEST_LANG", "zh-CN")
    out = tmp_path / "d.html"
    assert rh.main([str(SAMPLES["zh-CN"][0]), str(out)]) == 0
    assert '<html lang="zh-CN">' in out.read_text(encoding="utf-8")
    assert rh.main([str(SAMPLES["en"][0]), str(out), "--lang", "en"]) == 0  # CLI beats env
    assert '<html lang="en">' in out.read_text(encoding="utf-8")


def test_invalid_language_rejected():
    with pytest.raises(ValueError):
        i18n.normalize_lang("klingon")
    assert i18n.normalize_lang("zh") == "zh-CN" and i18n.normalize_lang("EN-us") == "en"


@pytest.mark.parametrize("lang,title", [
    ("en", "**AI Daily · 1/1**\n\n## 🧠 Models\n\n### 1. <script>x</script>: Title\n`a · 1/1 · b`\n\n**In brief:** text\n"),
    ("zh-CN", "**AI精选日报 · 1/1**\n\n## 🧠 模型\n\n### 1. <script>x</script>：标题\n`a · 1/1 · b`\n\n**一句话：** 文本\n"),
])
def test_render_is_escaped(lang, title):
    html = rh.render_html(rh.parse_digest(title, lang), lang)
    assert "<script>x</script>" not in html


def test_labels_are_interchangeable_in_markdown():
    # Chinese labels inside an English-language issue (and vice versa) still parse.
    d = rh.parse_digest("**AI Daily · 1/1**\n\n## Models\n\n### 1. T: x\n`a · 1/1 · b`\n\n**一句话：** ok\n\n**Why it matters:** fine\n", "en")
    assert set(d.items[0].fields) == {"dek", "look"}


def test_short_title_head_before_colon():
    assert rh._short_title("某某实验室发布新模型：细节很多很多很多") == "某某实验室发布新模型"


def test_short_title_short_head_keeps_rest():
    t = rh._short_title("Epoch：智能体推理容量一年增长 4 倍以上")
    assert t.startswith("Epoch：") and t.endswith("…")


def test_short_title_keep_rest_for_duplicate_heads():
    a = rh._short_title("某某实验室发布新模型：甲细节很多很多很多", keep_rest=True)
    assert "：" in a


def test_short_title_truncates_long_plain_title():
    t = rh._short_title("这是一个没有冒号而且非常非常非常非常长的标题用来测试截断")
    assert t.endswith("…") and len(t) <= 17


def test_short_title_latin_uses_head_and_word_boundary():
    short = rh._short_title("OpenAI: a very long headline about open weights and reasoning")
    assert short.startswith("OpenAI:") and short.endswith("…")
    cut = rh._short_title("Researchers publish a surprisingly long headline without any colon at all")
    assert cut.endswith("…") and not cut[:-1].endswith(("headl", "surpris"))  # cut at a word boundary
    assert len(cut) <= 29


def test_year_inferred_from_filename_and_cli_override(tmp_path):
    src = tmp_path / "digest-2027-01-15.md"
    src.write_text("**AI Daily · 1/15**\n\n## Models\n\n### 1. T: x\n`a · 1/15`\n\n**In brief:** ok\n", encoding="utf-8")
    out = tmp_path / "out.html"
    rh.render_file(src, out, "en")
    assert "Friday, January 15, 2027" in out.read_text(encoding="utf-8")
    assert rh.main([str(src), str(out), "--lang", "en", "--year", "2028"]) == 0
    assert "Saturday, January 15, 2028" in out.read_text(encoding="utf-8")


def test_cross_year_weekly_and_invalid_leap_day():
    en = rh._period_info("12/28–01/03", weekly=True, lang="en", year=2026)
    assert en["line"] == "Dec 28, 2026 – Jan 3, 2027"
    zh = rh._period_info("12/28–01/03", weekly=True, lang="zh-CN", year=2026)
    assert zh["line"] == "2026 年 12 月 28 日 — 2027 年 1 月 3 日"
    # Invalid leap date in non-leap year degrades gracefully instead of raising ValueError
    fallback = rh._period_info("2/29", weekly=False, lang="en", year=2026)
    assert fallback["line"] == "2/29"


def test_render_item_accepts_lang_alias():
    it = rh.Item(number="1", title="T", fields={"dek": "brief", "followup": "update"})
    rendered = rh.render_item(it, "zh")
    assert "本周后续" in rendered


def test_render_sanitizes_raw_html_and_unsafe_urls():
    md = (
        "**AI Daily · 1/1**\n\n"
        "## 🧠 Models\n\n"
        "### 1. Safe Title\n"
        "`Lab · 1/1 · Open`\n\n"
        "**In brief:** Hello <script>alert(1)</script> and <img src=x onerror=alert(2)> "
        "with [bad](javascript:alert(3)), [data](data:text/html,<svg>) and [good](https://example.com/ok).\n\n"
        "🔗 [Evil](javascript:alert(4)) · [Good](https://example.com/ref)\n"
    )
    out = rh.render_html(rh.parse_digest(md, "en"), "en")
    assert "<script>alert(1)</script>" not in out
    assert "<img src=x onerror=alert(2)>" not in out
    assert "javascript:" not in out.lower()
    assert "data:text/html" not in out.lower()
    assert 'href="https://example.com/ok"' in out
    assert 'href="https://example.com/ref"' in out
    assert "<span>Evil</span>" in out
    # Fallback renderer matches the same URL safety policy
    fb = rh._fallback_markdown("[bad](javascript:alert(1)) [ok](https://example.com/a)")
    assert "javascript:" not in fb.lower() and 'href="https://example.com/a"' in fb


def test_malformed_markdown_rejected_in_api_and_cli(tmp_path):
    import subprocess, sys

    for bad_md in (
        "",
        "   \n\n  ",
        "Just some random prose without any headings or digest items.\nAnother line.",
        "**AI Daily · 1/15**\n\n## 🧠 Models\n\nSection heading with zero numbered items.\n",
    ):
        with pytest.raises(ValueError):
            rh.parse_digest(bad_md, "en")

    bad_file = tmp_path / "garbage.md"
    bad_file.write_text("Not a valid digest markdown file at all.\n", encoding="utf-8")
    out_file = tmp_path / "out.html"
    proc = subprocess.run(
        [sys.executable, str(ROOT / "render_html.py"), str(bad_file), str(out_file)],
        capture_output=True, text=True, cwd=str(ROOT),
    )
    assert proc.returncode == 1
    assert "malformed digest markdown" in proc.stderr
    assert "Traceback" not in proc.stderr
    assert not out_file.exists()


@pytest.mark.parametrize("lang,expected_placeholder,expected_all", [
    ("en", "Search title, summary, source, or tag…", "All"),
    ("zh-CN", "搜索标题、摘要、来源或标签…", "全部"),
])
def test_rendered_issue_includes_filter_bar_theme_sync_and_hash_scroll(tmp_path, lang, expected_placeholder, expected_all):
    out = tmp_path / f"issue-{lang}.html"
    rh.render_file(SAMPLES[lang][0], out, lang, year=2026)
    html = out.read_text(encoding="utf-8")
    assert 'id="digestFilterBar"' in html
    assert 'id="digestFilterInput"' in html
    assert expected_placeholder in html
    assert f'>{expected_all}</button>' in html
    assert 'data-tags="' in html and 'data-tag="' in html
    assert "localStorage.getItem('digest-theme')" in html
    assert "location.hash.length>1" in html and "scrollIntoView" in html
