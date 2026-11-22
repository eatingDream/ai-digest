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
    sources = reg["source"]
    assert sources and all("id" in s and "type" in s for s in sources)
    ids = [s["id"] for s in sources]
    assert len(ids) == len(set(ids)), "duplicate source id in sources.toml"
    for s in sources:
        for key in ("orgs", "repos", "categories", "urls"):
            if key in s:
                assert len(s[key]) == len(set(s[key])), f"duplicate entry in {s['id']}.{key}"


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


def test_ctx_uses_thread_local_sessions_and_thread_safe_notes():
    import concurrent.futures as cf
    from datetime import datetime
    ctx = fx.Ctx({}, datetime(2026, 1, 15, tzinfo=timezone.utc), datetime(2026, 1, 16, tzinfo=timezone.utc), {}, False, False, None)
    try:
        with cf.ThreadPoolExecutor(4) as ex:
            sess_ids = list(ex.map(lambda i: (ctx.add_note(f"n{i}"), id(ctx.s))[1], range(8)))
        assert len(ctx.notes) == 8
        assert len(set(sess_ids)) >= 1
    finally:
        ctx.close()
    assert ctx._sessions == []


def test_compute_window_daily_weekly_and_custom():
    from types import SimpleNamespace
    s, e, lab = fx.compute_window(SimpleNamespace(daily=True, weekly=False, day="2026-01-15", start=None, end=None))
    assert lab == "2026-01-15" and (e - s).total_seconds() == 86400
    s2, e2, lab2 = fx.compute_window(SimpleNamespace(daily=False, weekly=True, day="2026-01-15", start=None, end=None))
    assert lab2 == "2026-01-18-weekly" and (e2 - s2).days == 7
    s3, e3, lab3 = fx.compute_window(SimpleNamespace(daily=False, weekly=False, day=None, start="2026-01-14T09:00", end="2026-01-15T09:00"))
    assert lab3 == "2026-01-15" and (e3 - s3).total_seconds() == 86400


def test_date_from_text_formats():
    assert fx.date_from_text("Published Oct 05, 2026").date().isoformat() == "2026-10-05"
    assert fx.date_from_text("2026年11月13日 公告").date().isoformat() == "2026-11-13"
    assert fx.date_from_text("https://example.com/news261113").date().isoformat() == "2026-11-13"
    assert fx.date_from_text("no date here") is None


def test_rebuild_triage_honors_no_fold_and_reuse_json(tmp_path, monkeypatch, capsys):
    import json
    from dataclasses import asdict
    from datetime import datetime
    d = tmp_path / "runs" / "2026-01-15" / "fetch"
    d.mkdir(parents=True)
    paper = fx.Cand(
        id="abcdef123456",
        title="Synthetic off-topic study",
        url="https://arxiv.org/abs/0000.00009",
        source="arxiv",
        channel="paper",
        kind="paper",
        extra={"arxiv_id": "0000.00009"},
    )
    (d / "candidates.jsonl").write_text(json.dumps(asdict(paper)) + "\n", encoding="utf-8")
    start = datetime(2026, 1, 15, 0, 0, tzinfo=fx.TZ_LOCAL)
    end = datetime(2026, 1, 16, 0, 0, tzinfo=fx.TZ_LOCAL)
    meta = {
        "window_start": fx.iso(start),
        "window_end": fx.iso(end),
        "generated_at": fx.iso(datetime(2026, 1, 16, 0, 15, tzinfo=fx.TZ_LOCAL)),
        "total_raw": 1,
        "total_dedup": 1,
        "notes": ["sample note"],
        "sources": [{"id": "arxiv", "status": "ok"}],
    }
    (d / "report.json").write_text(json.dumps(meta), encoding="utf-8")
    out_fold = tmp_path / "out_fold"
    out_nofold = tmp_path / "out_nofold"
    assert fx.main(["--rebuild", str(d), "--out", str(out_fold), "--lang", "en"]) == 0
    assert (out_fold / "triage-papers-rest.md").is_file()
    assert fx.main(["--rebuild", str(d), "--out", str(out_nofold), "--no-fold", "--lang", "en"]) == 0
    assert not (out_nofold / "triage-papers-rest.md").exists()
    assert "Synthetic off-topic study" in (out_nofold / "triage-papers.md").read_text(encoding="utf-8")

    # --reuse --json prints localized stderr (en) and valid JSON on stdout without hitting network
    (d / "triage.md").write_text("# ok\n", encoding="utf-8")
    monkeypatch.setenv("DIGEST_ROOT", str(tmp_path))
    capsys.readouterr()
    assert fx.main(["--daily", "--day", "2026-01-15", "--reuse", "--json", "--lang", "en"]) == 0
    captured = capsys.readouterr()
    assert "already fetched this window" in captured.err and "注：" not in captured.err
    payload = json.loads(captured.out)
    assert payload["reused"] is True and payload["dedup"] == 1
