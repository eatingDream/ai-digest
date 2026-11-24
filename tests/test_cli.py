"""Agent-facing CLI contracts: --help/--version, machine-readable --json, and doc guards."""
import concurrent.futures as cf
import json, os, re, shutil, subprocess, sys
from pathlib import Path

import version

ROOT = Path(__file__).resolve().parents[1]
CLIS = ("publish.py", "render_html.py", "fetch/fetch.py", "fetch/cand.py", "fetch/check_cands.py",
        "fetch/ledger.py", "fetch/xparse.py", "fetch/weekly_late.py", "fetch/page.py")


def run(*args, env=None, stdin=None):
    return subprocess.run([sys.executable, *map(str, args)], capture_output=True, text=True,
                          cwd=ROOT, env=dict(os.environ, **(env or {})), input=stdin)


def test_cli_help_version_and_usage(tmp_path):
    node = shutil.which("node")
    vd = lambda *a, env=None: subprocess.run([node, str(ROOT / "verify_decrypt.mjs"), *a], capture_output=True, text=True, cwd=ROOT, env=env)
    pw = tmp_path / "pw"
    pw.write_text("synthetic-test-pass-0003\n", encoding="utf-8")
    nd_env = dict(os.environ, DIGEST_ROOT=str(tmp_path / "empty"), DIGEST_SITE_PASSWORD_FILE=str(pw), SITE_SLUG="noslug")

    with cf.ThreadPoolExecutor(8) as ex:
        helps = [ex.submit(lambda s=s: (s, run(s, "--help", env={"DIGEST_LANG": ""}))) for s in CLIS]
        vers = [ex.submit(run, s, "--version") for s in ("publish.py", "render_html.py", "fetch/fetch.py")]
        bad_lang = ex.submit(run, "render_html.py", "examples/sample-digest.en.md", tmp_path / "x.html", "--lang", "klingon")
        vd_futs = ([ex.submit(vd, "--help"), ex.submit(vd, "--version"), ex.submit(vd, "--url", "https://example.com/a"),
                    ex.submit(vd, "--url"), ex.submit(vd, "--local", env=nd_env)] if node else [])
        for fut in helps:
            script, r = fut.result()
            assert r.returncode == 0 and "usage:" in r.stdout and ("xample" in r.stdout or "python3" in r.stdout), script
            assert not any("\u4e00" <= ch <= "\u9fff" for ch in r.stdout), script
        assert all(f.result().stdout.strip() == f"ai-digest {version.__version__}" for f in vers) and bad_lang.result().returncode == 2
        if vd_futs:
            h, v, u1, u2, nd = (f.result() for f in vd_futs)
            assert h.returncode == 0 and "usage:" in h.stdout and "examples:" in h.stdout and v.stdout.strip() == f"ai-digest {version.__version__}"
            assert u1.returncode != 0 and "paired" in u1.stderr and u2.returncode != 0 and "missing value" in u2.stderr and nd.returncode != 0 and "not found" in nd.stderr

    assert json.loads((ROOT / ".release-please-manifest.json").read_text(encoding="utf-8"))["."] == version.__version__
    ver = re.escape(version.__version__)
    assert re.search(rf"^###? (?:\[{ver}\](?:\([^)]*\))?|{ver}) (?:-\s*\d{{4}}-\d{{2}}-\d{{2}}|\(\d{{4}}-\d{{2}}-\d{{2}}\))$",
                     (ROOT / "CHANGELOG.md").read_text(encoding="utf-8"), re.M)


def test_ledger_cli_init_roundtrip_and_errors(tmp_path):
    led = tmp_path / "deep" / "ledger.json"
    env = {"LEDGER_PATH": str(led), "DIGEST_LANG": "en"}
    lc = lambda *a, **kw: run("fetch/ledger.py", *a, **kw)
    assert lc("recent", "--json", env=env).returncode == 0 and not led.exists()
    assert json.loads(lc("find", "orion", "--json", env=env).stdout) == {"query": {"orion": []}}

    r1 = lc("add", "--day", "2026-01-15", "--json", env=env,
            stdin="P|Orion-3 Mini|orion-3|model|https://example.com/orion|Open\nS|Other|||https://example.com/o|Minor\n")
    assert r1.returncode == 0 and json.loads(r1.stdout) == {"added": 2, "updated": 0, "total": 2}
    assert (led.parent / "logs" / "ledger-backup" / "ledger-2026-01-15.json").is_file()
    r2 = lc("add", "--day", "2026-01-16", "--json", env=env, stdin="S|Orion-3 Mini|orion-3|model|https://example.com/orion-paper|Paper\n")
    assert r2.returncode == 0 and json.loads(r2.stdout) == {"added": 0, "updated": 1, "total": 2}
    item = json.loads(lc("find", "orion", "--json", env=env).stdout)["query"]["orion"][0]
    assert item["id"] == "orion-3" and item["status"] == "pushed" and item["urls"] == ["https://example.com/orion", "https://example.com/orion-paper"]
    assert json.loads(lc("recent", "--days", "36500", "--json", env=env).stdout)["count"] == 2

    work_root = tmp_path / "isolated_root"
    assert lc("add", "--day", "2026-01-16", "--json", env={"DIGEST_ROOT": str(work_root), "LEDGER_PATH": "", "DIGEST_LANG": "en"},
              stdin="S|Nova-2|nova-2|paper|https://arxiv.org/abs/2601.00042|Seen\n").returncode == 0
    assert (work_root / "ledger.json").is_file() and (work_root / "logs" / "ledger-backup" / "ledger-2026-01-16.json").is_file() and not (ROOT / "ledger.json").exists()

    bad = tmp_path / "corrupt.json"
    bad.write_text("{not valid json", encoding="utf-8")
    rc = lc("recent", env={"LEDGER_PATH": str(bad)})
    assert rc.returncode == 2 and "cannot read ledger" in rc.stderr and "Traceback" not in rc.stderr


def test_render_cli_json_and_bilingual_errors(tmp_path):
    r = run("render_html.py", "examples/sample-digest.en.md", tmp_path / "o.html", "--json")
    assert r.returncode == 0 and (d := json.loads(r.stdout))["items"] == 10 and d["sections"] == 7 and d["lang"] == "en"
    rb = run("render_html.py", "examples/sample-digest.zh-CN.md", tmp_path / "bi.html", "--md-en", "examples/sample-digest.en.md", "--json")
    assert rb.returncode == 0 and (db := json.loads(rb.stdout))["bilingual"] is True and db["lang"] == "zh-CN"
    rm = run("render_html.py", "examples/sample-digest.zh-CN.md", tmp_path / "m.html", "--md-en", tmp_path / "nope.en.md")
    assert rm.returncode == 1 and "not found" in rm.stderr and "Traceback" not in rm.stderr


def test_fetch_helper_clis(tmp_path):
    d1, d2 = tmp_path / "runs/2026-01-12/fetch", tmp_path / "runs/2026-01-14/fetch"
    for d in (d1, d2):
        d.mkdir(parents=True)
    ids = [f"2601.{n:05d}" for n in range(1, 13)]
    p1 = {"id": "abc123456789", "title": "Orion Sparse RL", "url": "https://arxiv.org/abs/2601.00010", "source": "arxiv", "channel": "paper", "kind": "paper",
          "published": "2026-01-12T09:00+00:00", "summary": "Synthetic abstract detailing sparse RL gains.", "authors": ["A. Researcher"], "pref": 3, "also_in": [], "in_ledger": False, "extra": {"arxiv_id": "2601.00010"}}
    (d1 / "candidates.jsonl").write_text(json.dumps(p1) + "\n" + "".join(json.dumps(dict(p1, id=i, extra={"arxiv_id": i})) + "\n" for i in ids if i != "2601.00010"), encoding="utf-8")
    (d2 / "candidates.jsonl").write_text(json.dumps(dict(p1, source="hf_daily", published="2026-01-14T08:00+00:00", extra={"arxiv_id": "2601.00010", "org": "Example Lab"})) + "\n", encoding="utf-8")

    (f := tmp_path / "paper-cands.md.tmp").write_text("## Strong\n" + "".join(f"- {i} | t | r | x\n" for i in ids), encoding="utf-8")
    assert (r_ok := run("fetch/check_cands.py", f, "--dir", d1, "--json")).returncode == 0 and json.loads(r_ok.stdout)["ok"] and json.loads(r_ok.stdout)["n"] == 12
    f.write_text("## Strong\n- 2601.99999 | t | r | x\n", encoding="utf-8")
    assert run("fetch/check_cands.py", f, "--dir", d1, "--json").returncode == 1
    assert (rm := run("fetch/check_cands.py", tmp_path / "missing.md")).returncode == 1 and "file not found" in rm.stderr and "Traceback" not in rm.stderr

    (led := tmp_path / "ledger.json").write_text('{"items": []}', encoding="utf-8")
    env = {"DIGEST_ROOT": str(tmp_path), "LEDGER_PATH": str(led), "DIGEST_LANG": "en"}
    assert (rc := run("fetch/cand.py", "abc123", "--dir", d1, "--lang", "en", env=env)).returncode == 0 and "Orion Sparse RL" in rc.stdout and "authors:" in rc.stdout
    assert (rw := run("fetch/weekly_late.py", "--day", "2026-01-15", "--chars", "25", "--lang", "en", env=env)).returncode == 0 and "2601.00010" in rw.stdout and "HF late 01-14" in rw.stdout and "Synthetic abstract detail" in rw.stdout

    raw_x, out_x = tmp_path / "x.json", tmp_path / "nested/x-raw.md"
    raw_x.write_text(json.dumps({
        "data": [{"id": "101", "author_id": "u1", "last_updated_at_ms": 1768464000000, "text": "Example Labs published Orion-3 Mini open weights."},
                 {"id": "102", "author_id": "u1", "created_at": "2026-01-15T08:00:00Z", "text": "RT @someone retweeted post."}],
        "includes": {"users": [{"id": "u1", "username": "examplelabs"}]}, "meta": {"next_token": "tok123"},
    }), encoding="utf-8")
    assert (rx := run("fetch/xparse.py", f"A1={raw_x}", "--out", out_x, "--lang", "en", env=env)).returncode == 0 and "(1 items," in rx.stdout and "truncated" in rx.stdout
    assert "x.com/examplelabs/status/101" in (tx := out_x.read_text(encoding="utf-8")) and "01-15 08:00" in tx and "102" not in tx


def test_bilingual_docs_and_x_recall_contracts():
    fr = (ROOT / "fetch" / "README.md").read_text(encoding="utf-8")
    assert "# AI Digest Recall Layer" in fr and "# AI精选 召回层" in fr
    for rel in ("README.md", "README.zh-CN.md"):
        assert "<account_1>" in (txt := (ROOT / rel).read_text(encoding="utf-8")) and "LANG_ALT" in txt, rel
    for rel in ("prompts/en/daily.md", "prompts/en/weekly.md", "prompts/zh-CN/daily.md",
                "prompts/zh-CN/weekly.md", "fetch/x-recall.md", "fetch/x-recall.zh-CN.md"):
        txt = (ROOT / rel).read_text(encoding="utf-8")
        assert "<account_1>" in txt and "<zh_account_1>" in txt and ("skip" in txt.lower() or "跳过" in txt), rel
