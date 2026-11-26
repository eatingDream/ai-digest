import concurrent.futures as cf
import json, os, socket, tomllib
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace

import pytest

import fetch as fx
import page as pg


def _paper(aid, title, summary="", **kw):
    return fx.Cand(title=title, url=f"https://arxiv.org/abs/{aid}", source="arxiv",
                   channel="paper", kind="paper", summary=summary, extra={"arxiv_id": aid}, **kw)


def test_url_title_text_and_dedupe():
    assert fx.canon_url("https://www.example.com/post/?utm_source=x&id=3&ref=y") == "https://example.com/post?id=3"
    assert {fx.canon_url(u) for u in ("https://arxiv.org/abs/2601.01234", "https://arxiv.org/pdf/2601.01234",
                                      "https://huggingface.co/papers/2601.01234")} == {"arxiv:2601.01234"}
    assert fx.norm_title("Hello, World! 你好") == "helloworld你好" and len(fx.norm_title("x" * 200)) == 80
    assert fx.clean_text("<p>a  <b>b</b>&amp;c</p>") == "a b&c" and (t := fx.clean_text("x" * 50, 10)).endswith("…") and len(t) == 10
    mk = lambda src: fx.Cand(title="Same long title for dedupe test", url="https://example.com/a?utm_source=z", source=src, channel="lab", kind="lab_post")
    assert len(m := fx.dedupe([mk("s1"), mk("s2")])) == 1 and m[0].also_in == ["s2"]


def test_date_parsing_and_window_computation():
    assert fx.parse_dt("2026-01-15T08:00:00Z").utcoffset().total_seconds() == 0
    assert fx.parse_dt("Thu, 15 Jan 2026 08:00:00 +0800").astimezone(timezone.utc).hour == 0
    assert fx.parse_dt("2026-01-15").day == 15 and fx.parse_dt("not a date") is None
    assert fx.date_from_text("Published Oct 05, 2026").date().isoformat() == "2026-10-05"
    assert fx.date_from_text("2026年11月13日 公告").date().isoformat() == "2026-11-13"
    assert fx.date_from_text("https://example.com/news261113").date().isoformat() == "2026-11-13" and fx.date_from_text("no date") is None

    s, e, lab = fx.compute_window(SimpleNamespace(daily=True, weekly=False, day="2026-01-15", start=None, end=None))
    assert lab == "2026-01-15" and (e - s).total_seconds() == 86400
    s2, e2, lab2 = fx.compute_window(SimpleNamespace(daily=False, weekly=True, day="2026-01-15", start=None, end=None))
    assert lab2 == "2026-01-18-weekly" and (e2 - s2).days == 7
    s3, e3, lab3 = fx.compute_window(SimpleNamespace(daily=False, weekly=False, day=None, start="2026-01-14T09:00", end="2026-01-15T09:00"))
    assert lab3 == "2026-01-15" and (e3 - s3).total_seconds() == 86400


def test_sources_toml_and_localized_paper_signals(monkeypatch):
    sources = tomllib.loads((Path(fx.__file__).parent / "sources.toml").read_text(encoding="utf-8"))["source"]
    assert sources and all("id" in s and "type" in s for s in sources) and len(sources) == len({s["id"] for s in sources})
    for s in sources:
        for k in ("orgs", "repos", "categories", "urls"):
            assert k not in s or len(s[k]) == len(set(s[k])), f"duplicate in {s['id']}.{k}"

    frontier = _paper("0000.00001", "Orion-3 Technical Report: Open-Weights Release", "We release open-weights checkpoints.")
    routine = _paper("0000.00002", "A routine synthetic study", "No signals.")
    monkeypatch.setattr(fx, "FL", "en")
    assert fx.tr("note_prefix") == "- Note: " and fx.paper_signals(frontier, {}) == ["keyword:technical report/open weights / model family"] and fx.paper_signals(routine, {}) == []
    monkeypatch.setattr(fx, "FL", "zh-CN")
    assert fx.tr("note_prefix") == "- 注：" and fx.paper_signals(frontier, {}) == ["词:技术报告/开源权重/模型族"]


def test_ctx_thread_safety_and_rebuild_triage(tmp_path, monkeypatch, capsys):
    ctx = fx.Ctx({}, datetime(2026, 1, 15, tzinfo=timezone.utc), datetime(2026, 1, 16, tzinfo=timezone.utc), {}, False, False, None)
    try:
        with cf.ThreadPoolExecutor(4) as ex:
            sess_ids = list(ex.map(lambda i: (ctx.add_note(f"n{i}"), id(ctx.s))[1], range(8)))
        assert len(ctx.notes) == 8 and len(set(sess_ids)) >= 1
    finally:
        ctx.close()
    assert ctx._sessions == []

    d = tmp_path / "runs" / "2026-01-15" / "fetch"
    d.mkdir(parents=True)
    (d / "candidates.jsonl").write_text(json.dumps(asdict(_paper("0000.00009", "Synthetic off-topic study", id="abcdef123456"))) + "\n", encoding="utf-8")
    (d / "report.json").write_text(json.dumps({
        "window_start": fx.iso(datetime(2026, 1, 15, 0, 0, tzinfo=fx.TZ_LOCAL)), "window_end": fx.iso(datetime(2026, 1, 16, 0, 0, tzinfo=fx.TZ_LOCAL)),
        "generated_at": fx.iso(datetime(2026, 1, 16, 0, 15, tzinfo=fx.TZ_LOCAL)), "total_raw": 1, "total_dedup": 1,
        "notes": ["sample note"], "sources": [{"id": "arxiv", "status": "ok"}],
    }), encoding="utf-8")
    out_fold, out_nofold = tmp_path / "out_fold", tmp_path / "out_nofold"
    assert fx.main(["--rebuild", str(d), "--out", str(out_fold), "--lang", "en"]) == 0 and (out_fold / "triage-papers-rest.md").is_file()
    assert fx.main(["--rebuild", str(d), "--out", str(out_nofold), "--no-fold", "--lang", "en"]) == 0
    assert not (out_nofold / "triage-papers-rest.md").exists() and "Synthetic off-topic study" in (out_nofold / "triage-papers.md").read_text(encoding="utf-8")

    (d / "triage.md").write_text("# ok\n", encoding="utf-8")
    monkeypatch.setenv("DIGEST_ROOT", str(tmp_path))
    capsys.readouterr()
    assert fx.main(["--daily", "--day", "2026-01-15", "--reuse", "--json", "--lang", "en"]) == 0
    cap = capsys.readouterr()
    assert "already fetched this window" in cap.err and "注：" not in cap.err and json.loads(cap.out)["reused"] is True


def test_page_fetch_text_and_many(monkeypatch, capsys):
    body = ('<html><head><title>Example Article</title><meta property="article:published_time" content="2026-01-15"></head>'
            '<body><nav>menu</nav><article><h1>Orion-3 Mini</h1><p>' + ("Synthetic benchmark gain of 61.2 on ExampleBench. " * 10) + "</p></article></body></html>").encode()
    def fake_get(url, **kw):
        raw = b"" if "empty" in url else (b"# Repo\nFast serving engine." if "raw.githubusercontent.com" in url else body)
        return SimpleNamespace(content=raw, text=raw.decode())

    monkeypatch.setattr(pg, "get", fake_get)
    monkeypatch.setattr(pg, "FL", "en")
    ok, txt = pg.fetch_page_text("https://example.com/post", find="ExampleBench")
    assert ok and "Example Article" in txt and "61.2" in txt and "JS-rendered" in pg.fetch_page_text("https://example.com/empty")[1]
    ok_gh, txt_gh = pg.fetch_page_text("https://github.com/example/fastserve", find="serving")
    assert ok_gh and "example/fastserve README" in txt_gh and "serving" in txt_gh
    pg.run_many(SimpleNamespace(url=["https://example.com/a@@ExampleBench", "https://example.com/empty"], chars=500, ctx=80, meta=False, find=None))
    assert "### https://example.com/a" in (out := capsys.readouterr().out) and "### https://example.com/empty" in out


def test_page_ssrf_blocks_private_targets_and_redirects(monkeypatch):
    for bad in ("file:///etc/passwd", "ftp://example.com/a", "http://localhost/admin", "http://sub.localhost/x",
                "http://127.0.0.1:8080/health", "http://169.254.169.254/latest/meta-data/", "http://10.0.0.1/", "http://[::1]/", "http://[::ffff:127.0.0.1]/"):
        with pytest.raises(ValueError):
            pg._validate_public_url(bad)

    def fake_getaddrinfo(host, port, type=0):
        addrs = {"public.example": "93.184.216.34", "rebind.example": "127.0.0.1"}
        if host in addrs:
            return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", (addrs[host], port))]
        raise socket.gaierror("unknown")

    monkeypatch.setattr(pg.socket, "getaddrinfo", fake_getaddrinfo)
    assert pg._validate_public_url("https://public.example/article") == "https://public.example/article"
    with pytest.raises(ValueError):
        pg._validate_public_url("https://rebind.example/secret")
    monkeypatch.setattr(pg, "_session", lambda: SimpleNamespace(get=lambda url, **kw: (
        SimpleNamespace(status_code=302, headers={"location": "http://169.254.169.254/latest/"}, raise_for_status=lambda: None)
        if not kw.get("allow_redirects") and url == "https://public.example/start" else (_ for _ in ()).throw(AssertionError(url)))))
    with pytest.raises(ValueError, match="disallowed"):
        pg.get("https://public.example/start")


def test_digest_root_isolates_ledger_and_state_files(tmp_path, monkeypatch):
    work = tmp_path / "isolated"
    monkeypatch.setenv("DIGEST_ROOT", str(work))
    monkeypatch.delenv("LEDGER_PATH", raising=False)
    assert fx._ledger_path() == work / "ledger.json" and fx._state_dir() == work / "state" and fx._triage_seen_path() == work / "state" / "triage-seen.json"
    fx.save_triage_seen([fx.Cand(title="Orion Repo", url="https://github.com/example/orion", source="gh_trending", channel="repo", kind="repo", extra={"discovery_only": True})], "2026-01-15", {})
    assert (work / "state" / "triage-seen.json").is_file() and fx.load_triage_seen() == {"https://github.com/example/orion": "2026-01-15"}
    (work / "ledger.json").write_text(json.dumps({"items": [{"id": "2601.00077", "name": "Orion Paper", "status": "pushed", "urls": ["https://arxiv.org/abs/2601.00077"]}]}), encoding="utf-8")
    assert fx.load_ledger_keys().get("arxiv:2601.00077") == "pushed"


def test_github_api_bearer_token_and_rate_limit_warning(monkeypatch):
    seen, start, end = [], datetime(2026, 1, 15, tzinfo=timezone.utc), datetime(2026, 1, 16, tzinfo=timezone.utc)
    def make_ctx(status, headers=None, text=""):
        resp = SimpleNamespace(status_code=status, headers=headers or {}, text=text, json=lambda: [])
        ctx = fx.Ctx({}, start, end, {}, False, False)
        ctx._local.session = SimpleNamespace(get=lambda url, headers=None, **kw: (seen.append(dict(headers or {})), resp)[1], close=lambda: None)
        return ctx

    for env_k, env_v, clear_k in (("GITHUB_TOKEN", "tok-fixture-1", "GH_TOKEN"), ("GH_TOKEN", "tok-fixture-2", "GITHUB_TOKEN")):
        monkeypatch.setenv(env_k, env_v)
        monkeypatch.delenv(clear_k, raising=False)
        fx._gh_api(make_ctx(200), "https://api.github.com/orgs/example/repos")
        assert seen[-1].get("Authorization") == f"Bearer {env_v}"
    monkeypatch.delenv("GH_TOKEN", raising=False)
    monkeypatch.setattr(fx, "FL", "en")
    ctx_rl = make_ctx(403, headers={"X-RateLimit-Remaining": "0"}, text="API rate limit exceeded")
    with pytest.raises(RuntimeError, match="403.*GITHUB_TOKEN|GITHUB_TOKEN.*403"):
        fx._gh_api(ctx_rl, "https://api.github.com/orgs/example/repos")
    assert any("GITHUB_TOKEN" in n and "5,000" in n for n in ctx_rl.notes)


def test_common_decoupling_ledger_url_canon_and_atomic_writes(tmp_path, monkeypatch):
    import ast
    import importlib
    import subprocess
    import sys

    import common
    import i18n

    root = Path(__file__).resolve().parents[1]
    wl_src = (root / "fetch" / "weekly_late.py").read_text(encoding="utf-8")
    tree = ast.parse(wl_src)
    imported = {
        alias.name
        for node in ast.walk(tree)
        if isinstance(node, ast.Import)
        for alias in node.names
    } | {
        node.module
        for node in ast.walk(tree)
        if isinstance(node, ast.ImportFrom) and node.module
    }
    assert "importlib" not in imported and "importlib.util" not in imported and "fetch" not in imported
    assert "fx.FL" not in wl_src

    # Verify importing weekly_late in a fresh subprocess never loads fetch.py
    probe = subprocess.run(
        [sys.executable, "-c", "import sys; sys.path.insert(0, 'fetch'); import weekly_late; assert 'fetch' not in sys.modules"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    assert probe.returncode == 0, probe.stderr

    # ledger.py add deduplicates tracking-param URLs and arXiv/HF equivalents via common.canon_url()
    led = tmp_path / "ledger.json"
    env = dict(os.environ, LEDGER_PATH=str(led), DIGEST_LANG="en")
    r1 = subprocess.run(
        [sys.executable, str(root / "fetch" / "ledger.py"), "add", "--day", "2026-01-15", "--json"],
        input="S|Post A|post-a|news|https://www.example.com/post?utm_source=rss&id=7|First\n",
        cwd=root, env=env, capture_output=True, text=True,
    )
    assert r1.returncode == 0 and json.loads(r1.stdout) == {"added": 1, "updated": 0, "total": 1}
    r2 = subprocess.run(
        [sys.executable, str(root / "fetch" / "ledger.py"), "add", "--day", "2026-01-16", "--json"],
        input="P|Post A Variant||news|https://example.com/post/?id=7&ref=social|Updated\n",
        cwd=root, env=env, capture_output=True, text=True,
    )
    assert r2.returncode == 0 and json.loads(r2.stdout) == {"added": 0, "updated": 1, "total": 1}
    saved = json.loads(led.read_text(encoding="utf-8"))
    assert len(saved["items"]) == 1 and saved["items"][0]["status"] == "pushed" and len(saved["items"][0]["urls"]) == 1

    # atomic_write_text preserves old contents on failure and leaves no stray temp files
    target = tmp_path / "state" / "state.json"
    common.atomic_write_text(target, '{"ok":1}')
    assert target.read_text(encoding="utf-8") == '{"ok":1}'
    monkeypatch.setattr(common.os, "replace", lambda *_a, **_kw: (_ for _ in ()).throw(OSError("boom")))
    with pytest.raises(OSError, match="boom"):
        common.atomic_write_text(target, '{"ok":2}')
    assert target.read_text(encoding="utf-8") == '{"ok":1}'
    assert [p.name for p in target.parent.iterdir()] == ["state.json"]

    # check_cands.py uses centralized i18n.CHECK_CANDS_STRONG_TIER_WORDS
    assert set(i18n.CHECK_CANDS_STRONG_TIER_WORDS) >= {"强", "必看", "strong", "must"}
    assert "CHECK_CANDS_STRONG_TIER_WORDS" in (root / "fetch" / "check_cands.py").read_text(encoding="utf-8")

