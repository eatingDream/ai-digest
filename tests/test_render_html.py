import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

import i18n
import render_html as rh

ROOT = Path(__file__).resolve().parents[1]
SAMPLES = {
    "en": (ROOT / "examples" / "sample-digest.en.md", ROOT / "examples" / "sample-weekly.en.md"),
    "zh-CN": (ROOT / "examples" / "sample-digest.zh-CN.md", ROOT / "examples" / "sample-weekly.zh-CN.md"),
}


def test_parse_and_render_daily_and_weekly(tmp_path):
    en_d = rh.parse_digest(SAMPLES["en"][0].read_text(encoding="utf-8"), "en")
    zh_d = rh.parse_digest(SAMPLES["zh-CN"][0].read_text(encoding="utf-8"), "zh-CN")
    assert en_d.title == "AI Daily" and zh_d.title == "AI精选日报" and en_d.period == zh_d.period == "1/15"
    assert len(en_d.items) == len(zh_d.items) == 10 and set(en_d.items[0].fields) == set(zh_d.items[0].fields) == {"dek", "look", "detail"}
    expected_secs = ["sec-models", "sec-papers", "sec-tools", "sec-industry", "sec-infra", "sec-policy", "sec-voices"]
    assert [rh._section_id(s.title) for s in en_d.sections] == [rh._section_id(s.title) for s in zh_d.sections] == expected_secs

    for lang, needles, absent in (
        ("en", ['<html lang="en">', "Contents", "Why it matters", "Light", "End of issue", "Thursday, January 15, 2026"], ["值得看", "本期", "目录"]),
        ("zh-CN", ['<html lang="zh-CN">', "目录", "值得看", "浅色", "（本期完）", "2026 年 1 月 15 日 · 星期四"], ["Why it matters", "End of issue"]),
    ):
        out = tmp_path / f"d-{lang}.html"
        digest = rh.render_file(SAMPLES[lang][0], out, lang, year=2026)
        html = out.read_text(encoding="utf-8")
        assert html.lstrip().lower().startswith("<!doctype html") and '<link rel="stylesheet"' not in html and len(digest.items) == 10 and "Orion-3 Mini" in html
        assert all(n in html for n in needles) and all(a not in html for a in absent)

    for lang in ("en", "zh-CN"):
        w = rh.parse_digest(SAMPLES[lang][1].read_text(encoding="utf-8"), lang)
        assert i18n.looks_weekly(w.title) and w.must and w.themes and len(w.items) == 3 and "followup" in w.items[0].fields
    w_out = tmp_path / "w-en.html"
    rh.render_file(SAMPLES["en"][1], w_out, "en", year=2026)
    w_html = w_out.read_text(encoding="utf-8")
    assert all(n in w_html for n in ("Must read this week", "Themes of the week", "Follow-up", "Weekly", "Jan 12 – Jan 18, 2026"))

    mixed = rh.parse_digest("**AI Daily · 1/1**\n\n## Models\n\n### 1. T: x\n`a · 1/1 · b`\n\n**一句话：** ok\n\n**Why it matters:** fine\n", "en")
    assert set(mixed.items[0].fields) == {"dek", "look"}
    assert "本周后续" in rh.render_item(rh.Item(number="1", title="T", fields={"dek": "brief", "followup": "update"}), "zh")


def test_lang_precedence_year_and_period_edge_cases(tmp_path, monkeypatch):
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    out = tmp_path / "d.html"
    rh.render_file(SAMPLES["en"][0], out)
    assert '<html lang="en">' in out.read_text(encoding="utf-8")

    monkeypatch.setenv("DIGEST_LANG", "zh-CN")
    assert rh.main([str(SAMPLES["zh-CN"][0]), str(out)]) == 0 and '<html lang="zh-CN">' in out.read_text(encoding="utf-8")
    assert rh.main([str(SAMPLES["en"][0]), str(out), "--lang", "en"]) == 0 and '<html lang="en">' in out.read_text(encoding="utf-8")

    src = tmp_path / "digest-2027-01-15.md"
    src.write_text("**AI Daily · 1/15**\n\n## Models\n\n### 1. T: x\n`a · 1/15`\n\n**In brief:** ok\n", encoding="utf-8")
    rh.render_file(src, out, "en")
    assert "Friday, January 15, 2027" in out.read_text(encoding="utf-8")
    assert rh.main([str(src), str(out), "--lang", "en", "--year", "2028"]) == 0 and "Saturday, January 15, 2028" in out.read_text(encoding="utf-8")
    assert rh._period_info("12/28–01/03", weekly=True, lang="en", year=2026)["line"] == "Dec 28, 2026 – Jan 3, 2027"
    assert rh._period_info("12/28–01/03", weekly=True, lang="zh-CN", year=2026)["line"] == "2026 年 12 月 28 日 — 2027 年 1 月 3 日"
    assert rh._period_info("2/29", weekly=False, lang="en", year=2026)["line"] == "2/29"
    assert rh._short_title("Researchers publish a surprisingly long headline without colon").endswith("…")


def test_sanitizes_xss_unsafe_urls_and_rejects_malformed_markdown(tmp_path):
    xss_en = (
        "**AI Daily · 1/15**\n\n## 🧠 Models\n\n### 1. <script>alert(0)</script> Safe\n`Lab · 1/15 · Open`\n\n"
        "**In brief:** Hi <script>alert(1)</script> <img src=x onerror=alert(2)> "
        "[bad](javascript:alert(3)) [d](data:text/html,<svg>) [ok](https://example.com/ok).\n\n"
        "🔗 [Evil](javascript:alert(4)) · [Ref](https://example.com/ref)\n"
    )
    zh_md, en_md, bi_out = tmp_path / "d.md", tmp_path / "d.en.md", tmp_path / "bi.html"
    zh_md.write_text("**AI精选日报 · 1/15**\n\n## 🧠 模型\n\n### 1. <script>x</script>：安全\n`示例 · 1/15`\n\n**一句话：** 正常。\n", encoding="utf-8")
    en_md.write_text(xss_en, encoding="utf-8")
    rh.render_file(zh_md, bi_out, "zh-CN", en_md_path=en_md)
    for html in (rh.render_html(rh.parse_digest(xss_en, "en"), "en"), bi_out.read_text(encoding="utf-8")):
        assert "<script>alert(" not in html and "<script>x</script>" not in html and "<img src=x onerror=" not in html
        assert "javascript:" not in html.lower() and "data:text/html" not in html.lower()
        assert 'href="https://example.com/ok"' in html and 'href="https://example.com/ref"' in html and "<span>Evil</span>" in html
    fb = rh._fallback_markdown("[bad](javascript:alert(1)) [ok](https://example.com/a)")
    assert "javascript:" not in fb.lower() and 'href="https://example.com/a"' in fb

    for bad_md in ("", "   \n\n  ", "Random prose without headings.", "**AI Daily · 1/15**\n\n## Models\n\nZero items.\n"):
        with pytest.raises(ValueError):
            rh.parse_digest(bad_md, "en")
    bad_file, out_file = tmp_path / "garbage.md", tmp_path / "out.html"
    bad_file.write_text("Not a valid digest markdown file.\n", encoding="utf-8")
    proc = subprocess.run([sys.executable, str(ROOT / "render_html.py"), str(bad_file), str(out_file)], capture_output=True, text=True, cwd=ROOT)
    assert proc.returncode == 1 and "malformed digest markdown" in proc.stderr and "Traceback" not in proc.stderr and not out_file.exists()


def test_single_lang_filter_bar_full_toc_and_no_bilingual_chrome(tmp_path):
    for lang, placeholder, all_lbl in (("en", "Search title, summary, source, or tag…", "All"), ("zh-CN", "搜索标题、摘要、来源或标签…", "全部")):
        out = tmp_path / f"issue-{lang}.html"
        rh.render_file(SAMPLES[lang][0], out, lang, year=2026)
        html = out.read_text(encoding="utf-8")
        assert all(tok in html for tok in ('id="digestFilterBar"', 'id="digestFilterInput"', placeholder, f">{all_lbl}</button>",
                                           'data-tags="', 'data-tag="', "localStorage.getItem('digest-theme')", "location.hash.length>1", "scrollIntoView"))
        body = html.split("<body>", 1)[1].split("<script>", 1)[0]
        assert "lang-pane" not in body and "lang-switch" not in body and "localStorage.getItem('digest-lang')" not in html

    long_zh, long_en = "某某实验室发布首个超长标题模型用于验证侧栏完整显示不截断", "Example Labs publishes an extra-long headline verifying full sidebar display"
    out_zh = rh.render_html(rh.parse_digest(f"**AI精选日报 · 1/15**\n\n## 🧠 模型\n\n### 🆕 1. {long_zh}\n`甲 · 1/15`\n\n**一句话：** 示例。\n", "zh-CN"), "zh-CN")
    out_en = rh.render_html(rh.parse_digest(f"**AI Daily · 1/15**\n\n## 🧠 Models\n\n### 🆕 1. {long_en}\n`Lab · 1/15`\n\n**In brief:** Example.\n", "en"), "en")
    assert f'<span class="t">{long_zh}</span>' in out_zh and ".ts-items .t { min-width: 0; overflow-wrap: break-word; }" in out_zh
    assert ".ts-items a.has-new .t" not in out_zh and f'<span class="t">{long_en}</span>' in out_en
    assert "…" not in out_en.split('<nav class="toc-side"', 1)[1].split("</nav>", 1)[0]


def test_render_bilingual_daily_and_weekly_zero_duplicate_ids(tmp_path):
    node = shutil.which("node")
    for kind_idx, expected_items in ((0, 10), (1, 3)):
        out = tmp_path / f"bilingual-{kind_idx}.html"
        digest = rh.render_file(SAMPLES["zh-CN"][kind_idx], out, "zh-CN", year=2026, en_md_path=SAMPLES["en"][kind_idx])
        html = out.read_text(encoding="utf-8")
        assert len(digest.items) == expected_items
        ids = re.findall(r'\bid="([^"]+)"', html)
        assert ids and len(ids) == len(set(ids)), f"duplicate IDs: {[i for i in ids if ids.count(i) > 1]}"
        assert '<html lang="zh-CN">' in html and "localStorage.getItem('digest-lang')" in html and html.count('class="theme-switch lang-switch"') == 2
        assert '<div class="lang-pane lang-zh" data-lang="zh">' in html and '<div class="lang-pane lang-en" data-lang="en" lang="en" data-title="' in html
        assert 'aria-label="语言">' in html and 'aria-label="Language">' in html and 'aria-controls="en-tocSide"' in html
        assert all(f'id="{eid}"' in html for eid in ("content", "en-content", "tocSide", "en-tocSide", "sec-models", "en-sec-models",
                                                     "item-1", "en-item-1", "digestFilterBar", "en-digestFilterBar", "digestFilterInput", "en-digestFilterInput"))
        if kind_idx == 1:
            en_pane = html.split('<div class="lang-pane lang-en"', 1)[1]
            assert 'href="#en-item-1"' in en_pane and 'href="#item-1"' not in en_pane
        if node:
            scripts = re.findall(r"<script>(.*?)</script>", html, flags=re.S)
            assert len(scripts) == 3
            js = tmp_path / f"check-{kind_idx}.js"
            js.write_text("\n".join(f"{{{s}}}" for s in scripts), encoding="utf-8")
            proc = subprocess.run([node, "--check", str(js)], capture_output=True, text=True)
            assert proc.returncode == 0, proc.stderr


def test_render_bilingual_item_count_mismatch_warns(tmp_path, capsys):
    zh_md, en_md, out = tmp_path / "d.md", tmp_path / "d.en.md", tmp_path / "bi.html"
    zh_md.write_text("**AI精选日报 · 1/15**\n\n## 🧠 模型\n\n### 1. 一\n`甲 · 1/15`\n\n**一句话：** 一。\n\n### 2. 二\n`乙 · 1/15`\n\n**一句话：** 二。\n", encoding="utf-8")
    en_md.write_text("**AI Daily · 1/15**\n\n## 🧠 Models\n\n### 1. One\n`A · 1/15`\n\n**In brief:** One.\n", encoding="utf-8")
    rh.render_file(zh_md, out, "zh-CN", en_md_path=en_md)
    assert "WARNING: English has 1 items vs Chinese 2" in capsys.readouterr().err and out.is_file()
