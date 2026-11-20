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
    digest = rh.render_file(SAMPLES[lang][0], out, lang)
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
    rh.render_file(SAMPLES["en"][1], out, "en")
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
