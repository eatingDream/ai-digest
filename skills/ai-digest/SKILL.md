---
name: ai-digest
description: >-
  Read and search published AI Digest briefings in clean Markdown (via llms.txt,
  Accept: text/markdown, and /.well-known/search) or operate the ai-digest
  Rust CLI fetch, curation, HTML rendering, and Cloudflare Pages publishing pipeline.
---

# AI Digest Skill

Use this skill when interacting with either:
1. A **published public `ai-digest` site** (reading or searching curated AI news), or
2. A **local `ai-digest` repository / binary** (running the daily/weekly fetch -> curate -> render -> publish workflow).

## 1. Consuming a Published `ai-digest` Site

Prefer native Markdown and search endpoints over scraping HTML (~70% fewer tokens):

- **Discover site structure**: `GET https://<SITE_DOMAIN>/llms.txt`
- **Search all archived briefings** (GET, HEAD, or RFC 10008 HTTP `QUERY`):
  ```sh
  curl --get "https://<SITE_DOMAIN>/.well-known/search" --data-urlencode "q=reasoning models"
  curl -X QUERY "https://<SITE_DOMAIN>/.well-known/search" \
    -H "Content-Type: application/json" -d '{"q":"reasoning models","lang":"en","limit":10}'
  ```
- **Read latest or dated issue as Markdown**:
  - Send `Accept: text/markdown` to any page URL, OR fetch its `.md` mirror directly:
    - `https://<SITE_DOMAIN>/zh/daily/latest.md` / `https://<SITE_DOMAIN>/en/daily/latest.md`
    - `https://<SITE_DOMAIN>/zh/weekly/latest.md` / `https://<SITE_DOMAIN>/en/weekly/latest.md`
- **Feeds & full bundle**: `/llms-full.txt`, `/feed.xml` (`/zh/feed.xml`, `/en/feed.xml`), `/sitemap.xml`.

## 2. Operating the Local `ai-digest` Pipeline

Run commands with the compiled `ai-digest` binary (`target/release/ai-digest` or `ai-digest` on `PATH`; set `DIGEST_ROOT` for isolated working data):

| Step | Command | Notes |
| --- | --- | --- |
| Fetch candidates | `ai-digest fetch --daily --day YYYY-MM-DD --json` | Pulls ~155 open sources into `runs/<P>/fetch/`; zero API keys needed |
| Check / record ledger | `ai-digest ledger recent --json` / `ai-digest ledger add --day YYYY-MM-DD` | Tracks pushed (`★`) vs. seen (`☆`) items |
| Curate Markdown | Follow `prompts/<en\|zh-CN>/{papers,daily,weekly}.md` | Read `filter-notes*.md`; use `ai-digest cand` & `ai-digest page`; never leak watchlists or secrets |
| Render HTML | `ai-digest render digest-YYYY-MM-DD.md html/daily-YYYY-MM-DD.html --json` | Supports `--lang zh-CN\|en` and `--md-en` |
| Prepare & publish | `ai-digest publish --issue daily/YYYY-MM-DD --md digest-YYYY-MM-DD.md [--prepare-only]` | Public GEO by default (`ENCRYPT=0`); `--init --encrypt` enables AES-256-GCM (`ENCRYPT=1`, disables public GEO) |
| Verify output | `ai-digest verify --local` (or `node web/dist/verify_decrypt.mjs --local`) | Exact-match on public sites; WebCrypto/Rust AES-GCM decrypt parity on encrypted sites |
| Offline demo build | `ai-digest publish --demo --out dist/` | Builds synthetic bilingual demo with zero secrets or network |

## 3. Guardrails

- Always run `npm --prefix web run typecheck && npm --prefix web test`, `AI_DIGEST_REQUIRE_WEB_DIST=1 cargo test --locked`, and `./scripts/check_sanitized.sh` before committing changes to the repo.
- Never place tokens, private keys, or personal cookies inside the repository or `site/CONFIG`; store credentials outside the repo with mode `0600`.
- When `ENCRYPT=1`, keep the site strictly `Disallow: /` and `noindex`; never emit plaintext `llms.txt`, `sitemap.xml`, `.md` mirrors, or `_worker.js`.
