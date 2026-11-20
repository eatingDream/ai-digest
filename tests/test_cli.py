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


@pytest.mark.parametrize("script", [c for c in CLIS if c not in ("publish.py", "fetch/page.py")])
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
    led.write_text(json.dumps({"items": []}))
    env = {"LEDGER_PATH": str(led)}
    r = run("fetch/ledger.py", "add", "--day", "2026-01-15", "--json", env=env,
            stdin="P|Orion-3 Mini|orion-3|model|https://example.com/orion|Open weights release\nS|Other|||https://example.com/o|Not substantial\n")
    assert r.returncode == 0, r.stderr
    assert json.loads(r.stdout) == {"added": 2, "updated": 0, "total": 2}
    r = run("fetch/ledger.py", "find", "orion", "--json", env=env)
    assert json.loads(r.stdout)["query"]["orion"][0]["id"] == "orion-3"
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


def test_version_flag_and_changelog_agree():
    import re
    import version
    for script in ("publish.py", "render_html.py", "fetch/fetch.py"):
        assert run(script, "--version").stdout.strip() == f"ai-digest {version.__version__}"
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    assert re.search(rf"^## \[{re.escape(version.__version__)}\] - \d{{4}}-\d{{2}}-\d{{2}}$", changelog, re.M)


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
