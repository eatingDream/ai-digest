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


def test_sidebar_toc_displays_full_titles_without_ellipsis():
    md_zh = (
        "**AI精选日报 · 1/15**\n\n"
        "## 🧠 模型\n\n"
        "### 1. 某某实验室发布首个超长标题模型用于验证侧栏完整显示不截断\n"
        "`某某实验室 · 1/15 · 开源权重`\n\n"
        "**一句话：** 示例条目。\n\n"
        "### 🆕 2. 另一家研究机构发布带新标签的超长标题模型验证侧栏不出现省略号\n"
        "`研究机构 · 1/15 · 前沿发布`\n\n"
        "**一句话：** 带新标签的条目。\n"
    )
    out_zh = rh.render_html(rh.parse_digest(md_zh, "zh-CN"), "zh-CN")
    assert '<span class="t">某某实验室发布首个超长标题模型用于验证侧栏完整显示不截断</span>' in out_zh
    assert '<span class="t">另一家研究机构发布带新标签的超长标题模型验证侧栏不出现省略号</span>' in out_zh
    assert ".ts-items .t { min-width: 0; overflow-wrap: break-word; }" in out_zh
    assert ".ts-items a.has-new .t" not in out_zh

    md_en = (
        "**AI Daily · 1/15**\n\n"
        "## 🧠 Models\n\n"
        "### 🆕 1. Example Labs publishes an extra-long headline verifying full sidebar display\n"
        "`Example Labs · 1/15 · Open weights`\n\n"
        "**In brief:** Example entry.\n"
    )
    out_en = rh.render_html(rh.parse_digest(md_en, "en"), "en")
    assert (
        '<span class="t">Example Labs publishes an extra-long headline verifying full sidebar display</span>'
        in out_en
    )
    assert "…" not in out_en.split('<nav class="toc-side"', 1)[1].split("</nav>", 1)[0]


def test_single_language_output_has_no_bilingual_chrome(tmp_path):
    out = tmp_path / "single.html"
    rh.render_file(SAMPLES["zh-CN"][0], out, "zh-CN", year=2026)
    html = out.read_text(encoding="utf-8")
    body = html.split("<body>", 1)[1].split("<script>", 1)[0]
    assert "lang-pane" not in body
    assert "lang-switch" not in body
    assert 'data-lang="en"' not in body
    assert "localStorage.getItem('digest-lang')" not in html


@pytest.mark.parametrize("kind_idx", [0, 1])
def test_render_bilingual_daily_and_weekly_zero_duplicate_ids(tmp_path, kind_idx):
    import re
    import shutil
    import subprocess

    out = tmp_path / f"bilingual-{kind_idx}.html"
    digest = rh.render_file(SAMPLES["zh-CN"][kind_idx], out, "zh-CN", year=2026, en_md_path=SAMPLES["en"][kind_idx])
    html = out.read_text(encoding="utf-8")
    assert len(digest.items) == (10 if kind_idx == 0 else 3)

    # Zero duplicate HTML IDs across the entire document
    ids = re.findall(r'\bid="([^"]+)"', html)
    assert ids, "expected id attributes"
    assert len(ids) == len(set(ids)), f"duplicate IDs: {[i for i in ids if ids.count(i) > 1]}"

    # Pane structure, pre-paint scripts, language switcher, prefixed IDs & filter bars
    assert '<html lang="zh-CN">' in html
    assert "localStorage.getItem('digest-theme')" in html
    assert "localStorage.getItem('digest-lang')" in html
    assert '<div class="lang-pane lang-zh" data-lang="zh">' in html
    assert '<div class="lang-pane lang-en" data-lang="en" lang="en" data-title="' in html
    assert html.count('class="theme-switch lang-switch"') == 2
    assert 'aria-label="语言"><button type="button" data-lang="zh">中文</button><button type="button" data-lang="en">English</button>' in html
    assert 'aria-label="Language"><button type="button" data-lang="zh">中文</button><button type="button" data-lang="en">English</button>' in html
    for expected_id in (
        "content", "en-content",
        "report-start", "en-report-start",
        "tocToggle", "en-tocToggle",
        "tocBackdrop", "en-tocBackdrop",
        "tocSide", "en-tocSide",
        "tocClose", "en-tocClose",
        "sec-models", "en-sec-models",
        "item-1", "en-item-1",
        "toTop", "en-toTop",
        "digestFilterBar", "en-digestFilterBar",
        "digestFilterInput", "en-digestFilterInput",
        "digestFilterClear", "en-digestFilterClear",
        "digestFilterCount", "en-digestFilterCount",
        "digestFilterEmpty", "en-digestFilterEmpty",
    ):
        assert f'id="{expected_id}"' in html, expected_id
    assert 'aria-controls="tocSide"' in html and 'aria-controls="en-tocSide"' in html
    assert 'html[data-lang="en"] .lang-zh { display: none !important; }' in html
    assert 'html:not([data-lang="en"]) .lang-en { display: none !important; }' in html
    assert ".lang-en .look-k" in html

    # Weekly front-matter links inside EN pane use en- prefix
    if kind_idx == 1:
        en_pane = html.split('<div class="lang-pane lang-en"', 1)[1]
        assert 'href="#en-item-1"' in en_pane
        assert 'href="#item-1"' not in en_pane

    # Validate emitted inline JS syntax with Node if available
    node = shutil.which("node")
    if node:
        scripts = re.findall(r"<script>(.*?)</script>", html, flags=re.S)
        assert len(scripts) == 3  # theme pre-paint, lang pre-paint, footer
        for idx, body in enumerate(scripts):
            js_file = tmp_path / f"check-{kind_idx}-{idx}.js"
            js_file.write_text(body, encoding="utf-8")
            proc = subprocess.run([node, "--check", str(js_file)], capture_output=True, text=True)
            assert proc.returncode == 0, proc.stderr


def test_render_bilingual_sanitizes_xss_in_en_pane(tmp_path):
    zh_md = tmp_path / "digest-2026-01-15.md"
    en_md = tmp_path / "digest-2026-01-15.en.md"
    zh_md.write_text(
        "**AI精选日报 · 1/15**\n\n## 🧠 模型\n\n### 1. 安全测试\n`示例 · 1/15 · 开源`\n\n**一句话：** 正常内容。\n",
        encoding="utf-8",
    )
    en_md.write_text(
        "**AI Daily · 1/15**\n\n## 🧠 Models\n\n### 1. <script>alert(0)</script> Safe\n"
        "`Example · 1/15 · Open`\n\n"
        "**In brief:** Hi <script>alert(1)</script> <img src=x onerror=alert(2)> "
        "[bad](javascript:alert(3)) [d](data:text/html,<svg>) [ok](https://example.com/en).\n\n"
        "🔗 [Evil](javascript:alert(4)) · [Ref](https://example.com/ref)\n",
        encoding="utf-8",
    )
    out = tmp_path / "bi.html"
    rh.render_file(zh_md, out, "zh-CN", en_md_path=en_md)
    html = out.read_text(encoding="utf-8")
    assert "<script>alert(" not in html
    assert "<img src=x onerror=" not in html
    assert "javascript:" not in html.lower()
    assert "data:text/html" not in html.lower()
    assert 'href="https://example.com/en"' in html
    assert 'href="https://example.com/ref"' in html
    assert "<span>Evil</span>" in html


def test_render_bilingual_item_count_mismatch_warns(tmp_path, capsys):
    zh_md = tmp_path / "digest-2026-01-15.md"
    en_md = tmp_path / "digest-2026-01-15.en.md"
    zh_md.write_text(
        "**AI精选日报 · 1/15**\n\n## 🧠 模型\n\n"
        "### 1. 第一条\n`甲 · 1/15`\n\n**一句话：** 一。\n\n"
        "### 2. 第二条\n`乙 · 1/15`\n\n**一句话：** 二。\n",
        encoding="utf-8",
    )
    en_md.write_text(
        "**AI Daily · 1/15**\n\n## 🧠 Models\n\n"
        "### 1. First\n`A · 1/15`\n\n**In brief:** One.\n",
        encoding="utf-8",
    )
    out = tmp_path / "bi.html"
    rh.render_file(zh_md, out, "zh-CN", en_md_path=en_md)
    err = capsys.readouterr().err
    assert "WARNING: English has 1 items vs Chinese 2" in err
    assert out.is_file()
