import re
from datetime import date

import pytest

import i18n


def test_string_tables_have_identical_keys():
    assert set(i18n.STRINGS["en"]) == set(i18n.STRINGS["zh-CN"])
    assert set(i18n.FETCH["en"]) == set(i18n.FETCH["zh-CN"])


def test_placeholders_match_between_languages():
    field = re.compile(r"\{(\w+)")
    for table in (i18n.STRINGS, i18n.FETCH):
        for key, en in table["en"].items():
            if key == "month_label":  # {m} vs {month}: the English side names the month
                continue
            assert set(field.findall(en)) == set(field.findall(table["zh-CN"][key])), key


def test_english_strings_have_no_cjk():
    cjk = re.compile(r"[\u4e00-\u9fff]")
    for table in (i18n.STRINGS, i18n.FETCH):
        bad = [k for k, v in table["en"].items() if cjk.search(v)]
        assert not bad, bad


def test_resolve_lang_precedence(monkeypatch):
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    assert i18n.resolve_lang() == "en"
    assert i18n.resolve_lang(config="zh-CN") == "zh-CN"
    monkeypatch.setenv("DIGEST_LANG", "zh")
    assert i18n.resolve_lang(config="en") == "zh-CN"
    assert i18n.resolve_lang("en", "zh-CN") == "en"


def test_date_formats():
    d = date(2026, 1, 15)
    assert i18n.date_long("en", d) == "January 15, 2026"
    assert i18n.date_long("zh-CN", d) == "2026 年 1 月 15 日"
    assert i18n.weekday("en", d) == "Thursday" and i18n.weekday("zh-CN", d) == "星期四"
    assert i18n.date_range("en", date(2026, 1, 12), date(2026, 1, 18)) == "Jan 12 – Jan 18, 2026"
    assert i18n.date_range("zh-CN", date(2026, 1, 12), date(2026, 1, 18)) == "2026 年 1 月 12 日 — 1 月 18 日"
    assert i18n.year_week("en", 2026, 3) == "2026 · Week 3"
    assert i18n.year_week("zh-CN", 2026, 3) == "2026 年第 3 周"
    assert i18n.month_label("en", 2026, 1) == "January 2026"
    assert i18n.month_label("zh-CN", 2026, 1) == "2026年1月"


def test_count_items_and_paren():
    assert i18n.count_items("en", 1) == "1 item" and i18n.count_items("en", 3) == "3 items"
    assert i18n.count_items("zh-CN", 3) == "3 条"
    assert i18n.cnt_paren("en", 5) == " (5)" and i18n.cnt_paren("zh-CN", 5) == "（5）"


def test_section_ids_cover_both_languages():
    ids = {sid for _words, sid in i18n.SECTION_IDS}
    assert {"sec-models", "sec-papers", "sec-tools", "sec-industry", "sec-infra", "sec-policy", "sec-voices"} <= ids
    for words, _sid in i18n.SECTION_IDS:  # every section is recognisable in both languages
        assert any(w.isascii() for w in words) and any(not w.isascii() for w in words)


def test_issue_plurals():
    assert i18n.count_issues("en", 1) == "1 issue" and i18n.count_issues("en", 2) == "2 issues"
    assert i18n.issue_unit("en", 1) == "issue" and i18n.issue_unit("en", 0) == "issues"
    assert i18n.count_issues("zh-CN", 1) == i18n.count_issues("zh-CN", 1).replace("issue", "") == "1 期"
