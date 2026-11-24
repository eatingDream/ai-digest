import re
from datetime import date

import pytest

import i18n


def test_string_tables_parity_and_no_cjk_in_en():
    field, cjk = re.compile(r"\{(\w+)"), re.compile(r"[\u4e00-\u9fff]")
    for table in (i18n.STRINGS, i18n.FETCH):
        assert set(table["en"]) == set(table["zh-CN"])
        assert not [k for k, v in table["en"].items() if cjk.search(v)]
        for key, en in table["en"].items():
            if key != "month_label":  # {month} vs {m}
                assert set(field.findall(en)) == set(field.findall(table["zh-CN"][key])), key


def test_lang_resolution_and_section_ids(monkeypatch):
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    assert i18n.resolve_lang() == "en" and i18n.resolve_lang(config="zh-CN") == "zh-CN"
    monkeypatch.setenv("DIGEST_LANG", "zh")
    assert i18n.resolve_lang(config="en") == "zh-CN" and i18n.resolve_lang("en", "zh-CN") == "en"
    assert i18n.normalize_lang("zh") == "zh-CN" and i18n.normalize_lang("EN-us") == "en"
    with pytest.raises(ValueError):
        i18n.normalize_lang("klingon")

    ids = {sid for _words, sid in i18n.SECTION_IDS}
    assert {"sec-models", "sec-papers", "sec-tools", "sec-industry", "sec-infra", "sec-policy", "sec-voices"} <= ids
    assert all(any(w.isascii() for w in ws) and any(not w.isascii() for w in ws) for ws, _ in i18n.SECTION_IDS)


def test_date_and_count_formatting():
    d = date(2026, 1, 15)
    assert i18n.date_long("en", d) == "January 15, 2026" and i18n.date_long("zh-CN", d) == "2026 年 1 月 15 日"
    assert i18n.weekday("en", d) == "Thursday" and i18n.weekday("zh-CN", d) == "星期四"
    assert i18n.date_range("en", date(2026, 1, 12), date(2026, 1, 18)) == "Jan 12 – Jan 18, 2026"
    assert i18n.date_range("zh-CN", date(2026, 1, 12), date(2026, 1, 18)) == "2026 年 1 月 12 日 — 1 月 18 日"
    assert i18n.date_range("en", date(2026, 12, 28), date(2027, 1, 3)) == "Dec 28, 2026 – Jan 3, 2027"
    assert i18n.date_range("zh-CN", date(2026, 12, 28), date(2027, 1, 3)) == "2026 年 12 月 28 日 — 2027 年 1 月 3 日"
    assert i18n.year_week("en", 2026, 3) == "2026 · Week 3" and i18n.year_week("zh-CN", 2026, 3) == "2026 年第 3 周"
    assert i18n.month_label("en", 2026, 1) == "January 2026" and i18n.month_label("zh-CN", 2026, 1) == "2026年1月"
    assert i18n.count_items("en", 1) == "1 item" and i18n.count_items("en", 3) == "3 items" and i18n.count_items("zh-CN", 3) == "3 条"
    assert i18n.cnt_paren("en", 5) == " (5)" and i18n.cnt_paren("zh-CN", 5) == "（5）"
    assert i18n.count_issues("en", 1) == "1 issue" and i18n.count_issues("en", 2) == "2 issues" and i18n.count_issues("zh-CN", 1) == "1 期"
    assert i18n.issue_unit("en", 1) == "issue" and i18n.issue_unit("en", 0) == "issues"
