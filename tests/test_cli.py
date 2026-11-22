"""Agent-facing CLI contract: every script has --help with examples; --json is machine-readable."""
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
CLIS = ["publish.py", "render_html.py", "fetch/fetch.py", "fetch/cand.py", "fetch/check_cands.py",
        "fetch/ledger.py", "fetch/xparse.py", "fetch/weekly_late.py", "fetch/page.py"]


def run(*args, env=None, stdin=None):
    e = dict(os.environ, **(env or {}))
    return subprocess.run([sys.executable, *map(str, args)], capture_output=True, text=True, cwd=ROOT, env=e, input=stdin)


@pytest.mark.parametrize("script", CLIS)
def test_help_works_and_has_examples(script):
    r = run(script, "--help")
    assert r.returncode == 0, r.stderr
    assert "usage:" in r.stdout
    assert "xample" in r.stdout or "python3" in r.stdout  # at least one runnable example


@pytest.mark.parametrize("script", CLIS)
def test_help_is_english_by_default(script):
    r = run(script, "--help", env={"DIGEST_LANG": ""})
    assert not any("\u4e00" <= ch <= "\u9fff" for ch in r.stdout), script


def test_bad_language_is_a_usage_error():
    assert run("render_html.py", "examples/sample-digest.en.md", "/tmp/x.html", "--lang", "klingon").returncode == 2


def test_render_json(tmp_path):
    r = run("render_html.py", "examples/sample-digest.en.md", tmp_path / "o.html", "--json")
    d = json.loads(r.stdout)
    assert d["items"] == 10 and d["sections"] == 7 and d["lang"] == "en"


def test_ledger_json_roundtrip(tmp_path):
    led = tmp_path / "ledger.json"
    led.write_text(json.dumps({"items": []}), encoding="utf-8")
    env = {"LEDGER_PATH": str(led)}
    r = run("fetch/ledger.py", "add", "--day", "2026-01-15", "--json", env=env,
            stdin="P|Orion-3 Mini|orion-3|model|https://example.com/orion|Open weights release\nS|Other|||https://example.com/o|Not substantial\n")
    assert r.returncode == 0, r.stderr
    assert json.loads(r.stdout) == {"added": 2, "updated": 0, "total": 2}
    # Backup stays beside custom LEDGER_PATH rather than polluting repo logs/
    assert (tmp_path / "logs" / "ledger-backup" / "ledger-2026-01-15.json").is_file()
    # Updating existing pushed item with a new URL merges URLs and never downgrades pushed -> seen
    r2 = run("fetch/ledger.py", "add", "--day", "2026-01-16", "--json", env=env,
             stdin="S|Orion-3 Mini|orion-3|model|https://example.com/orion-paper|Paper link\n")
    assert r2.returncode == 0, r2.stderr
    assert json.loads(r2.stdout) == {"added": 0, "updated": 1, "total": 2}
    r = run("fetch/ledger.py", "find", "orion", "--json", env=env)
    item = json.loads(r.stdout)["query"]["orion"][0]
    assert item["id"] == "orion-3" and item["status"] == "pushed"
    assert item["urls"] == ["https://example.com/orion", "https://example.com/orion-paper"]
    r = run("fetch/ledger.py", "recent", "--days", "36500", "--json", env=env)
    assert json.loads(r.stdout)["count"] == 2


def test_ledger_missing_file_is_a_clean_error(tmp_path):
    r = run("fetch/ledger.py", "recent", env={"LEDGER_PATH": str(tmp_path / "nope.json")})
    assert r.returncode == 2 and "cannot read ledger" in r.stderr and "Traceback" not in r.stderr


def test_check_cands_json_and_exit_code(tmp_path):
    d = tmp_path / "runs" / "2026-01-15" / "fetch"
    d.mkdir(parents=True)
    ids = [f"2601.{n:05d}" for n in range(1, 13)]
    (d / "candidates.jsonl").write_text("".join(json.dumps({"extra": {"arxiv_id": i}, "in_ledger": False}) + "\n" for i in ids))
    f = tmp_path / "runs" / "2026-01-15" / "paper-cands.md.tmp"
    f.write_text("## Strong\n" + "".join(f"- {i} | t | r | x\n" for i in ids))
    r = run("fetch/check_cands.py", f, "--dir", d, "--json")
    out = json.loads(r.stdout)
    assert r.returncode == 0 and out["ok"] and out["n"] == 12 and out["unknown"] == []
    f.write_text("## Strong\n- 2601.99999 | t | r | x\n")
    r = run("fetch/check_cands.py", f, "--dir", d, "--json")
    out = json.loads(r.stdout)
    assert r.returncode == 1 and not out["ok"] and out["unknown"] == ["2601.99999"] and out["problems"]
    missing = run("fetch/check_cands.py", tmp_path / "missing.md")
    assert missing.returncode == 1 and "file not found" in missing.stderr and "Traceback" not in missing.stderr


def test_cand_xparse_and_weekly_late_clis(tmp_path):
    # cand.py lookup by short id and arXiv id
    d1 = tmp_path / "runs" / "2026-01-12" / "fetch"
    d2 = tmp_path / "runs" / "2026-01-14" / "fetch"
    d1.mkdir(parents=True)
    d2.mkdir(parents=True)
    p1 = {
        "id": "abc123456789", "title": "Orion Sparse RL", "url": "https://arxiv.org/abs/2601.00010",
        "source": "arxiv", "channel": "paper", "kind": "paper", "published": "2026-01-12T09:00+00:00",
        "summary": "Synthetic abstract detailing sparse RL gains.", "authors": ["A. Researcher"],
        "pref": 3, "also_in": [], "in_ledger": False, "extra": {"arxiv_id": "2601.00010"},
    }
    p2 = dict(p1, source="hf_daily", published="2026-01-14T08:00+00:00", extra={"arxiv_id": "2601.00010", "org": "Example Lab"})
    (d1 / "candidates.jsonl").write_text(json.dumps(p1) + "\n", encoding="utf-8")
    (d2 / "candidates.jsonl").write_text(json.dumps(p2) + "\n", encoding="utf-8")
    led = tmp_path / "ledger.json"
    led.write_text(json.dumps({"items": []}), encoding="utf-8")
    env = {"DIGEST_ROOT": str(tmp_path), "LEDGER_PATH": str(led), "DIGEST_LANG": "en"}

    rc = run("fetch/cand.py", "abc123", "--dir", d1, "--lang", "en", env=env)
    assert rc.returncode == 0 and "Orion Sparse RL" in rc.stdout and "authors:" in rc.stdout

    # weekly_late.py honors --chars and detects late HF signal
    rw = run("fetch/weekly_late.py", "--day", "2026-01-15", "--chars", "25", "--lang", "en", env=env)
    assert rw.returncode == 0 and "2601.00010" in rw.stdout and "HF late 01-14" in rw.stdout
    assert "Synthetic abstract detail" in rw.stdout

    # xparse.py parses epoch ms timestamps, next_token truncation, creates --out parent dir, localizes summary
    raw_x = tmp_path / "x.json"
    raw_x.write_text(json.dumps({
        "data": [
            {"id": "101", "author_id": "u1", "last_updated_at_ms": 1768464000000,
             "text": "Example Labs just published Orion-3 Mini open weights and eval report."},
            {"id": "102", "author_id": "u1", "created_at": "2026-01-15T08:00:00Z", "text": "RT @someone retweeted post that should be skipped."},
        ],
        "includes": {"users": [{"id": "u1", "username": "examplelabs"}]},
        "meta": {"next_token": "tok123"},
    }), encoding="utf-8")
    out_x = tmp_path / "nested" / "x-raw.md"
    rx = run("fetch/xparse.py", f"A1={raw_x}", "--out", out_x, "--lang", "en", env=env)
    assert rx.returncode == 0 and "(1 items," in rx.stdout and "truncated" in rx.stdout
    content = out_x.read_text(encoding="utf-8")
    assert "x.com/examplelabs/status/101" in content and "01-15 08:00" in content and "102" not in content


def test_version_flag_and_changelog_agree():
    import re
    import version
    for script in ("publish.py", "render_html.py", "fetch/fetch.py"):
        assert run(script, "--version").stdout.strip() == f"ai-digest {version.__version__}"
    manifest = json.loads((ROOT / ".release-please-manifest.json").read_text(encoding="utf-8"))
    assert manifest["."] == version.__version__
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    ver = re.escape(version.__version__)
    assert re.search(
        rf"^###? (?:\[{ver}\](?:\([^)]*\))?|{ver}) (?:-\s*\d{{4}}-\d{{2}}-\d{{2}}|\(\d{{4}}-\d{{2}}-\d{{2}}\))$",
        changelog,
        re.M,
    )


def test_verify_decrypt_help_and_version():
    import shutil
    import version
    node = shutil.which("node")
    if not node:
        pytest.skip("node not installed")
    h = subprocess.run([node, str(ROOT / "verify_decrypt.mjs"), "--help"], capture_output=True, text=True)
    assert h.returncode == 0 and "usage:" in h.stdout and "examples:" in h.stdout
    v = subprocess.run([node, str(ROOT / "verify_decrypt.mjs"), "--version"], capture_output=True, text=True)
    assert v.stdout.strip() == f"ai-digest {version.__version__}"
