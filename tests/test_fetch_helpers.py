from datetime import timezone

import fetch as fx


def test_canon_url_strips_tracking_and_www():
    a = fx.canon_url("https://www.example.com/post/?utm_source=x&id=3&ref=y")
    assert a == "https://example.com/post?id=3"


def test_canon_url_arxiv_variants_collapse():
    ids = {fx.canon_url(u) for u in (
        "https://arxiv.org/abs/2601.01234",
        "https://arxiv.org/pdf/2601.01234",
        "https://huggingface.co/papers/2601.01234",
    )}
    assert ids == {"arxiv:2601.01234"}


def test_norm_title():
    assert fx.norm_title("Hello, World! 你好") == "helloworld你好"
    assert len(fx.norm_title("x" * 200)) == 80


def test_clean_text_html_and_truncate():
    assert fx.clean_text("<p>a  <b>b</b>&amp;c</p>") == "a b&c"
    out = fx.clean_text("x" * 50, 10)
    assert out.endswith("…") and len(out) == 10


def test_parse_dt_variants():
    assert fx.parse_dt("2026-01-15T08:00:00Z").utcoffset().total_seconds() == 0
    d = fx.parse_dt("Thu, 15 Jan 2026 08:00:00 +0800")
    assert d.astimezone(timezone.utc).hour == 0
    assert fx.parse_dt("2026-01-15").day == 15
    assert fx.parse_dt("not a date") is None


def test_dedupe_merges_same_url():
    mk = lambda src, kind: fx.Cand(title="Same long title for dedupe test", url="https://example.com/a?utm_source=z",
                                   source=src, channel="lab", kind=kind)
    out = fx.dedupe([mk("s1", "lab_post"), mk("s2", "lab_post")])
    assert len(out) == 1 and out[0].also_in == ["s2"]


def test_sources_toml_parses():
    import tomllib
    from pathlib import Path
    reg = tomllib.loads((Path(fx.__file__).parent / "sources.toml").read_text(encoding="utf-8"))
    assert reg["source"] and all("id" in s and "type" in s for s in reg["source"])


def test_fetch_labels_follow_language(monkeypatch):
    monkeypatch.setattr(fx, "FL", "en")
    en = fx.tr("note_prefix")
    monkeypatch.setattr(fx, "FL", "zh-CN")
    zh = fx.tr("note_prefix")
    assert en == "- Note: " and zh == "- 注："


def test_fetch_lang_flag_sets_language(monkeypatch):
    monkeypatch.setattr(fx, "FL", "en")
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    import i18n
    assert i18n.resolve_lang("zh-CN") == "zh-CN"


def test_paper_signals_keyword_tags_localized(monkeypatch):
    c = fx.Cand(
        title="Orion-3 Technical Report: Open-Weights Release",
        url="https://arxiv.org/abs/0000.00001",
        source="arxiv",
        channel="paper",
        kind="paper",
        summary="We release open-weights checkpoints.",
        extra={"arxiv_id": "0000.00001"},
    )
    monkeypatch.setattr(fx, "FL", "en")
    assert fx.paper_signals(c, {}) == ["keyword:technical report/open weights / model family"]
    monkeypatch.setattr(fx, "FL", "zh-CN")
    assert fx.paper_signals(c, {}) == ["词:技术报告/开源权重/模型族"]


def test_paper_signals_empty_when_no_signal(monkeypatch):
    c = fx.Cand(
        title="A routine synthetic study on synthetic data",
        url="https://arxiv.org/abs/0000.00002",
        source="arxiv",
        channel="paper",
        kind="paper",
        summary="No frontier signals present.",
        extra={"arxiv_id": "0000.00002"},
    )
    monkeypatch.setattr(fx, "FL", "en")
    assert fx.paper_signals(c, {}) == []
