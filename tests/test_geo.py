"""Contract, security, and Pages Worker runtime tests for public GEO (geo.py)."""
from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import geo
import publish


def _clean_env(work: Path, **extra: str) -> dict[str, str]:
    env = dict(os.environ, DIGEST_ROOT=str(work), DIGEST_TZ="UTC")
    for k in (
        "DIGEST_LANG",
        "SITE_SLUG",
        "SITE_DOMAIN",
        "DIGEST_LANG_ALT",
        "DIGEST_ROOT_LATEST",
        "DIGEST_ENCRYPT",
        "DIGEST_SITE_PASSWORD_FILE",
        "DIGEST_GEO_AI_TRAIN",
        "DIGEST_CONTENT_SIGNAL",
        "DIGEST_GEO_WORKER",
        "DIGEST_GEO_SEARCH",
    ):
        env.pop(k, None)
    env.update(extra)
    return env


def test_public_geo_full_bundle_and_worker_runtime(tmp_path):
    work = tmp_path / "geopub"
    env = _clean_env(work, SITE_DOMAIN="public.example.com")
    run_pub = lambda *a: subprocess.run(
        [sys.executable, str(ROOT / "publish.py"), *a],
        env=env,
        capture_output=True,
        text=True,
        cwd=ROOT,
    )

    assert run_pub("--init", "--lang", "zh-CN").returncode == 0
    shutil.copy(ROOT / "examples" / "sample-digest.zh-CN.md", work / "digest-2026-01-15.md")
    shutil.copy(ROOT / "examples" / "sample-digest.en.md", work / "digest-2026-01-15.en.md")
    shutil.copy(ROOT / "examples" / "sample-weekly.zh-CN.md", work / "weekly-2026-01-12_01-18.md")
    shutil.copy(ROOT / "examples" / "sample-weekly.en.md", work / "weekly-2026-01-12_01-18.en.md")
    assert run_pub("--issue", "daily/2026-01-15", "--md", "digest-2026-01-15.md", "--prepare-only").returncode == 0
    assert run_pub("--issue", "weekly/2026-01-12_01-18", "--md", "weekly-2026-01-12_01-18.md", "--prepare-only").returncode == 0

    pub = work / "site" / "public"

    # 1. robots.txt: Content-Signal, explicit AI crawlers, Sitemap, no Disallow: /
    robots = (pub / "robots.txt").read_text(encoding="utf-8")
    assert "Content-Signal: ai-train=yes, search=yes, ai-input=yes" in robots
    assert "Sitemap: https://public.example.com/sitemap.xml" in robots
    assert "Disallow: /" not in robots
    for bot in ("OAI-SearchBot", "Claude-SearchBot", "PerplexityBot", "GPTBot", "ClaudeBot"):
        assert f"User-agent: {bot}" in robots

    # 2. _headers: no global noindex, has Content-Signal/Link/Vary + strict SHA-256 CSP, 404 keeps noindex
    hdrs = (pub / "_headers").read_text(encoding="utf-8")
    global_block = hdrs.split("/favicon.svg", 1)[0]
    assert "X-Robots-Tag: noindex" not in global_block
    assert "/404.html\n  X-Robots-Tag: noindex, nofollow" in hdrs
    assert "Content-Signal: ai-train=yes, search=yes, ai-input=yes" in hdrs
    assert 'rel="describedby"' in hdrs and 'rel="sitemap"' in hdrs and 'rel="search"' in hdrs
    assert "Vary: Accept" in hdrs
    csp = next(l for l in hdrs.splitlines() if "Content-Security-Policy:" in l)
    assert "'unsafe-inline'" not in csp.split("script-src ", 1)[1].split(";", 1)[0]

    # 3. HTML <head> SEO/GEO tags & JSON-LD
    zh_day = (pub / "zh/daily/2026-01-15.html").read_text(encoding="utf-8")
    zh_lat = (pub / "zh/daily/latest.html").read_text(encoding="utf-8")
    idx = (pub / "index.html").read_text(encoding="utf-8")
    assert "noindex" not in zh_day and 'content="index,follow' in zh_day
    assert '<link rel="canonical" href="https://public.example.com/zh/daily/2026-01-15">' in zh_day
    assert '<link rel="canonical" href="https://public.example.com/zh/daily/2026-01-15">' in zh_lat
    assert 'hreflang="en" href="https://public.example.com/en/daily/2026-01-15"' in zh_day
    assert 'type="text/markdown" title="Markdown" href="https://public.example.com/zh/daily/2026-01-15.md"' in zh_day
    assert 'name="agent:endpoint" content="https://public.example.com/.well-known/search?q={query}"' in zh_day
    m_jld = re.search(r'<script type="application/ld\+json">(.*?)</script>', zh_day, re.S)
    assert m_jld is not None
    jld = json.loads(m_jld.group(1))
    assert jld["@type"] == "NewsArticle" and jld["inLanguage"] == "zh-CN" and jld["mainEntity"]["@type"] == "ItemList"
    m_idx_jld = re.search(r'<script type="application/ld\+json">(.*?)</script>', idx, re.S)
    assert m_idx_jld is not None
    idx_jld = json.loads(m_idx_jld.group(1))
    assert any(node.get("@type") == "WebSite" and "potentialAction" in node for node in idx_jld["@graph"])

    # 4. sitemap.xml, Atom feeds, llms.txt, llms-full.txt, .md mirrors, Agent Skills digest, search-index.json
    sm = ET.fromstring((pub / "sitemap.xml").read_text(encoding="utf-8"))
    locs = [el.text for el in sm.findall(".//{http://www.sitemaps.org/schemas/sitemap/0.9}loc")]
    assert "https://public.example.com/" in locs
    assert "https://public.example.com/zh/daily/2026-01-15" in locs
    assert "https://public.example.com/en/daily/2026-01-15" in locs
    assert not any(loc.endswith("/latest") for loc in locs)

    for feed_rel in ("feed.xml", "zh/feed.xml", "en/feed.xml"):
        feed_root = ET.fromstring((pub / feed_rel).read_text(encoding="utf-8"))
        assert feed_root.tag.endswith("feed") and len(feed_root.findall(".//{http://www.w3.org/2005/Atom}entry")) >= 1

    llms = (pub / "llms.txt").read_text(encoding="utf-8")
    assert llms.startswith("# AI Digest\n") and "/.well-known/search?q={query}" in llms
    assert "https://public.example.com/zh/daily/2026-01-15.md" in llms
    llms_full = (pub / "llms-full.txt").read_text(encoding="utf-8")
    assert "Orion-3 Mini" in llms_full

    en_md = (pub / "en/daily/2026-01-15.md").read_text(encoding="utf-8")
    assert en_md.startswith("---\n") and 'endpoint: "https://public.example.com/.well-known/search?q={query}"' in en_md
    assert "Orion-3 Mini" in en_md and "```json\n{" in en_md
    assert len(en_md.encode("utf-8")) < len((pub / "en/daily/2026-01-15.html").read_bytes()) // 2

    skill_idx = json.loads((pub / ".well-known/agent-skills/index.json").read_text(encoding="utf-8"))
    skill_md_bytes = (pub / ".well-known/agent-skills/ai-digest-reader/SKILL.md").read_bytes()
    assert skill_idx["skills"][0]["digest"] == "sha256:" + hashlib.sha256(skill_md_bytes).hexdigest()

    sidx = json.loads((pub / "search-index.json").read_text(encoding="utf-8"))
    assert len(sidx["docs"]) >= 4

    # 5. verify_decrypt.mjs --local still passes byte-for-byte
    v_pub = subprocess.run(["node", str(ROOT / "verify_decrypt.mjs"), "--local"], env=env, capture_output=True, text=True, cwd=ROOT)
    assert v_pub.returncode == 0 and "matched plaintext" in v_pub.stdout

    # 6. _worker.js syntax + live execution (HTML, Accept: text/markdown, /index.md fallback, GET & QUERY search, 404)
    worker_path = pub / "_worker.js"
    assert worker_path.is_file() and (pub / "_routes.json").is_file()
    assert subprocess.run(["node", "--check", str(worker_path)], capture_output=True, text=True).returncode == 0

    driver = f"""
import worker from {json.dumps(worker_path.as_uri())};
import {{ readFileSync, existsSync }} from "node:fs";
import {{ join }} from "node:path";
const pub = {json.dumps(str(pub))};
const env = {{
  ASSETS: {{
    async fetch(req) {{
      const u = new URL(typeof req === "string" ? req : req.url);
      const p = u.pathname;
      const cands = p === "/" ? ["index.html"] : p.endsWith("/") ? [p.slice(1) + "index.html"] : [p.slice(1), p.slice(1) + ".html", p.slice(1) + "/index.html"];
      for (const c of cands) {{
        const fp = join(pub, c);
        if (existsSync(fp)) {{
          const ct = c.endsWith(".json") ? "application/json; charset=utf-8" : c.endsWith(".md") ? "text/markdown; charset=utf-8" : "text/html; charset=utf-8";
          return new Response(readFileSync(fp), {{ status: 200, headers: {{ "Content-Type": ct }} }});
        }}
      }}
      return new Response(readFileSync(join(pub, "404.html")), {{ status: 404, headers: {{ "Content-Type": "text/html; charset=utf-8" }} }});
    }}
  }}
}};
const htmlR = await worker.fetch(new Request("https://public.example.com/en/daily/2026-01-15", {{ headers: {{ Accept: "text/html,application/xhtml+xml,*/*;q=0.8" }} }}), env);
const htmlBody = await htmlR.text();
const mdR = await worker.fetch(new Request("https://public.example.com/en/daily/2026-01-15", {{ headers: {{ Accept: "text/markdown" }} }}), env);
const mdBody = await mdR.text();
const idxMdR = await worker.fetch(new Request("https://public.example.com/en/daily/2026-01-15/index.md"), env);
const sGet = await worker.fetch(new Request("https://public.example.com/.well-known/search?q=Orion"), env);
const sGetBody = await sGet.text();
const sQuery = await worker.fetch(new Request("https://public.example.com/.well-known/search", {{ method: "QUERY", headers: {{ "Content-Type": "application/json" }}, body: JSON.stringify({{ q: "Orion", lang: "en" }}) }}), env);
const sQueryBody = await sQuery.text();
const sEmpty = await worker.fetch(new Request("https://public.example.com/.well-known/search"), env);
const notFound = await worker.fetch(new Request("https://public.example.com/no-such-page"), env);
console.log(JSON.stringify({{
  htmlStatus: htmlR.status,
  htmlExact: htmlBody === readFileSync(join(pub, "en/daily/2026-01-15.html"), "utf8"),
  htmlCsp: htmlR.headers.get("content-security-policy") || "",
  htmlRobots: htmlR.headers.get("x-robots-tag"),
  htmlLink: htmlR.headers.get("link") || "",
  mdStatus: mdR.status,
  mdCt: mdR.headers.get("content-type"),
  mdTok: Number(mdR.headers.get("x-markdown-tokens")),
  mdHasFm: mdBody.startsWith("---\\n"),
  idxMdStatus: idxMdR.status,
  sGetStatus: sGet.status,
  sGetAq: sGet.headers.get("accept-query"),
  sGetOk: sGetBody.startsWith('# Search results for: "Orion"') && sGetBody.includes("Orion-3 Mini"),
  sQueryStatus: sQuery.status,
  sQueryOk: sQueryBody.startsWith('# Search results for: "Orion"') && sQueryBody.includes("en/daily/2026-01-15.md"),
  sEmptyStatus: sEmpty.status,
  sEmptyCt: sEmpty.headers.get("content-type"),
  nfStatus: notFound.status,
  nfRobots: notFound.headers.get("x-robots-tag"),
}}));
"""
    res = subprocess.run(["node", "--input-type=module", "-e", driver], capture_output=True, text=True)
    assert res.returncode == 0, res.stderr
    out = json.loads(res.stdout)
    assert out["htmlStatus"] == 200 and out["htmlExact"] is True
    assert "'sha256-" in out["htmlCsp"] and out["htmlRobots"] is None and 'rel="describedby"' in out["htmlLink"]
    assert out["mdStatus"] == 200 and "text/markdown" in out["mdCt"] and out["mdTok"] > 50 and out["mdHasFm"] is True
    assert out["idxMdStatus"] == 200
    assert out["sGetStatus"] == 200 and "text/plain" in out["sGetAq"] and out["sGetOk"] is True
    assert out["sQueryStatus"] == 200 and out["sQueryOk"] is True
    assert out["sEmptyStatus"] == 400 and "text/markdown" in out["sEmptyCt"]
    assert out["nfStatus"] == 404 and out["nfRobots"] == "noindex, nofollow"


def test_encrypted_mode_purges_geo_ai_train_policy_and_public_demo(monkeypatch, tmp_path):
    # 1. Start public (ENCRYPT=0) with GEO_AI_TRAIN=no -> verify training crawlers disallowed while search allowed
    work = tmp_path / "toggle"
    env = _clean_env(work, SITE_DOMAIN="policy.example.com", DIGEST_GEO_AI_TRAIN="no")
    run_pub = lambda e, *a: subprocess.run(
        [sys.executable, str(ROOT / "publish.py"), *a],
        env=e,
        capture_output=True,
        text=True,
        cwd=ROOT,
    )
    assert run_pub(env, "--init", "--lang", "en").returncode == 0
    shutil.copy(ROOT / "examples" / "sample-digest.en.md", work / "digest-2026-01-15.md")
    assert run_pub(env, "--issue", "daily/2026-01-15", "--md", "digest-2026-01-15.md", "--prepare-only").returncode == 0
    pub = work / "site" / "public"
    robots_no_train = (pub / "robots.txt").read_text(encoding="utf-8")
    assert "Content-Signal: ai-train=no, search=yes, ai-input=yes" in robots_no_train
    assert "User-agent: OAI-SearchBot\nAllow: /" in robots_no_train
    assert "User-agent: GPTBot\nDisallow: /" in robots_no_train
    assert (pub / "sitemap.xml").is_file() and (pub / "_worker.js").is_file() and (pub / "daily/2026-01-15.md").is_file()

    # 2. Switch same site to ENCRYPT=1 -> every plaintext GEO artifact must be purged & robots/headers locked down
    pw = work / "pw"
    pw.write_text("very-secret-passphrase-2026-geo\n", encoding="utf-8")
    pw.chmod(0o600)
    enc_env = _clean_env(
        work,
        SITE_DOMAIN="policy.example.com",
        SITE_SLUG="privateslug123",
        DIGEST_ENCRYPT="1",
        DIGEST_SITE_PASSWORD_FILE=str(pw),
    )
    assert run_pub(enc_env, "--rebuild", "--prepare-only").returncode == 0
    assert (pub / "robots.txt").read_text(encoding="utf-8") == "User-agent: *\nDisallow: /\n"
    assert "X-Robots-Tag: noindex, nofollow" in (pub / "_headers").read_text(encoding="utf-8")
    for rel in geo.GEO_ROOT_FILES + geo.GEO_EXTRA_FILES:
        assert not (pub / rel).exists(), f"leaked GEO file in encrypted build: {rel}"
    assert list(pub.rglob("*.md")) == []

    # 3. Public multi-lang --demo builds merged GEO bundle + _worker.js redirect parity
    monkeypatch.delenv("DIGEST_DEMO_PASSWORD", raising=False)
    monkeypatch.delenv("DIGEST_LANG", raising=False)
    monkeypatch.setenv("SITE_DOMAIN", "demo.example.com")
    out = tmp_path / "pubdemo"
    assert publish.main(["--demo", "--out", str(out)]) == 0
    assert (out / "sitemap.xml").is_file() and (out / "llms.txt").is_file() and (out / "_worker.js").is_file()
    sm_txt = (out / "sitemap.xml").read_text(encoding="utf-8")
    assert "https://demo.example.com/en/daily/" in sm_txt and "https://demo.example.com/zh-CN/daily/" in sm_txt
    assert '"redirects":[["/","/en/",302]]' in (out / "_worker.js").read_text(encoding="utf-8").replace(" ", "")
