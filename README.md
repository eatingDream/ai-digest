<div align="center">

<img src="docs/logo.svg" alt="AI Digest logo" width="68" height="68">

# AI Digest

**An agent-driven pipeline that turns ~155 open sources into a daily & weekly AI digest — rendered to a single-file editorial reader and published as a hardened static site.**

[![CI](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/eatingDream/ai-digest?color=b85432)](https://github.com/eatingDream/ai-digest/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-4f6d5a)](LICENSE)
![Python](https://img.shields.io/badge/python-3.11%2B-3776AB?logo=python&logoColor=white)
![Cloudflare Pages](https://img.shields.io/badge/deploy-Cloudflare%20Pages-F38020?logo=cloudflare&logoColor=white)

English | [简体中文](README.zh-CN.md)

**[Live Demo](https://ai-digest-demo.pages.dev)** · [English Index](https://ai-digest-demo.pages.dev/en/) · [Latest EN Daily](https://ai-digest-demo.pages.dev/en/daily/latest) · [中文归档](https://ai-digest-demo.pages.dev/zh-CN/) · [最新中文日报](https://ai-digest-demo.pages.dev/zh-CN/daily/latest)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshot-dark.en.png">
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshot.en.png">
  <img src="docs/screenshot.en.png" alt="AI Digest rendered sample issue (synthetic data)" width="880">
</picture>

<sub>Built automatically from the synthetic sample issues in <code>examples/</code>. Adapts automatically to Light (warm paper) and Dark (obsidian charcoal + terracotta) modes, in both English and Simplified Chinese.</sub>

</div>

## Highlights

- **Warm editorial reader (`render_html.py`)** — single-file HTML with warm paper Light Mode & obsidian-charcoal/terracotta Dark Mode, full-bleed auto-hide TopBar (hides on scroll down, reveals on scroll up or 3 s pause), collapsible TopBar search drawer (`/` shortcut, zero body bloat) with frequency-sorted atomic tag chips, smooth live-tracked full-title sidebar TOC, copy-permalink anchors, clickable `AI Digest` brand home, and built-in `/favicon.svg`.
- **Native bilingual routes & static `latest`** — auto-pairs sibling `<stem>.en.md` files into symmetric `/zh/` and `/en/` site trees (or single-file `--md-en` bilingual pages / `LANG_ALT` companion links), plus direct HTTP 200 `/daily/latest`, `/weekly/latest`, `/zh/daily/latest`, and `/en/daily/latest` pages served with `no-store`.
- **Hardened public-first static publishing (`publish.py`)** — publishes a clean, password-free root site by default (`ENCRYPT=0`, `SLUG=`) with build-time SHA-256 hash-locked CSP (zero `'unsafe-inline'` scripts), HSTS/COOP/CORP/Permissions-Policy, Markdown XSS & raw `<img>` neutralization, and entity-aware privacy scanning. Opt-in client-side AES-256-GCM encryption + private random `SLUG` stays one flag away (`--init --encrypt` / `ENCRYPT=1`).
- **Wide open recall + safe optional X sources (`fetch/`)** — pulls ~155 zero-key RSS/Atom/arXiv/HF/GitHub/blog/HN/Reddit sources in 10–40 s with `ledger.py` history deduplication (`★` pushed / `☆` seen). Optional X/Twitter recall works **without a Grok Bot** via free or paid safe APIs (see [Safe X recall without a Grok Bot](#safe-x-recall-without-a-grok-bot)).

## Architecture

<p align="center"><img src="docs/architecture.en.svg" alt="AI Digest architecture diagram" width="100%"></p>

1. **Wide recall & triage (`fetch/`)** — parallel collection from ~155 open sources (`fetch/sources.toml`, zero API keys, popularity counts stripped) + ledger history tracking → `triage.md`, `triage-papers.md`, and `candidates.jsonl`.
2. **Content-first LLM curation (`prompts/<lang>/`)** — platform-agnostic daily/weekly prompts verify primary sources via SSRF-safe `fetch/page.py` against `filter-notes.md` and write seven editorial sections (`Models`, `Papers`, `Tools & Projects`, `Industry & Products`, `Compute & Infrastructure`, `Safety & Policy`, `Opinions & Essays`).
3. **Single-file render & hardening (`render_html.py`, `publish.py`)** — renders self-contained pages with atomic tags, collapsible TopBar search, `/zh/` & `/en/` routing, static `latest`, entity-aware privacy scan, and build-time SHA-256 hash-locked `_headers`.
4. **Edge deploy & live verification** — incremental Cloudflare Pages deploy followed by `verify_decrypt.mjs` checking every published route live (exact-match in public mode, real WebCrypto AES-GCM decryption in encrypted mode).

## Live demo

The public demo at **[ai-digest-demo.pages.dev](https://ai-digest-demo.pages.dev)** is password-free, updated automatically on every `main` push from the synthetic issues in `examples/`, and ships both `/en/` and `/zh-CN/` archives plus static `latest` links. Reproduce it locally with one command:

```bash
python3 publish.py --demo --out dist/          # dist/en/ + dist/zh-CN/ (pass --lang en for single)
```

## Quick start

Requirements: Python 3.11+ (CI uses 3.12), Node 20 + `npx` (only for `wrangler` deploys and `verify_decrypt.mjs`).

```bash
# Install dependencies in your preferred Python environment (Conda / venv)
pip install -r requirements-dev.txt            # requests, lxml, markdown, cryptography, pytest

# 1. Render a sample issue (add --lang zh-CN for Chinese, or --md-en for a single-file bilingual toggle)
python3 render_html.py examples/sample-digest.en.md /tmp/sample.html
python3 render_html.py examples/sample-digest.zh-CN.md /tmp/bilingual.html --md-en examples/sample-digest.en.md

# 2. Build the public demo site locally (zero secrets; set DIGEST_DEMO_PASSWORD='demo' to test encryption)
python3 publish.py --demo --out dist/

# 3. Run the test suite
pytest -q
```

### Run your own digest

```bash
# 1. Initialize config (public root site by default; add --encrypt for password + random SLUG)
python3 publish.py --init                      # or: cp site/CONFIG.example site/CONFIG
export CLOUDFLARE_API_TOKEN=...                # scope: Account · Cloudflare Pages · Edit
# Optional password mode (only when ENCRYPT=1):
# install -m 600 /dev/stdin ~/.config/ai-digest/site-password

# 2. Fetch candidates (set DIGEST_TZ first; GITHUB_TOKEN/GH_TOKEN optional for higher API limits)
python3 fetch/fetch.py --daily                 # weekly: --weekly

# 3. Curate with an AI agent: run prompts/en/daily.md (or weekly.md; prompts/zh-CN/ for Chinese) -> digest-<P>.md

# 4. Publish (auto-discovers sibling <stem>.en.md on zh-CN sites for /zh/ & /en/ trees + static latest)
python3 publish.py --issue daily/2026-01-15 --md digest-2026-01-15.md
```

See [fetch/README.md](fetch/README.md) for channels and helper CLIs, and [site/README.md](site/README.md) for optional encryption, `/zh/` & `/en/` routing, `latest` pages, `--rebuild`, `--rerender-all`, `--rotate-password`, and `--remove`.

## Use with AI agents

[AGENTS.md](AGENTS.md) is the entry point for coding agents (repo map, pipeline, conventions, safety rules). Open the repo root in any agent runner with shell and file tools, set `DIGEST_LANG` and `DIGEST_TZ`, and run `prompts/<lang>/{papers,daily,weekly}.md` (each begins with a run contract; CLIs support `--help` and `--json`).

### Safe X recall without a Grok Bot

X/Twitter recall is **100% optional** and never requires a Grok Bot or personal X cookies. Full setup details and query templates live in [fetch/x-recall.md](fetch/x-recall.md):

- **Free & zero-risk paths**:
  - **Open-feeds-only mode (default)** — prompts skip X cleanly and curate from ~155 open RSS/Atom/arXiv/HF/GitHub/lab-blog/HN/Reddit feeds.
  - **Read-only web search grounding** — query `site:x.com/<handle>` over your date window using free-tier search tools (Gemini Search Grounding, Brave Search, Tavily, Exa, Perplexity) without logging into X.
  - **Official cross-posted RSS & self-hosted bridges** — subscribe to Bluesky (`https://bsky.app/profile/<handle>/rss`), Mastodon, Substack, or self-hosted [RSSHub](https://docs.rsshub.app/) public read-only routes in `fetch/sources.toml`. *(Note: X's official Free API tier does not allow general timeline/search reads.)*
- **Paid & official API paths**:
  - **xAI Grok API (`x_search` tool)** *(recommended paid)* — pay-as-you-go official API (`api.x.ai`, typically pennies/day) with handle & date filters and zero X account risk.
  - **Official X API v2 (Pay-Per-Use / Basic)** — read-only Bearer Token (`GET /2/tweets/search/recent`), whose JSON plugs directly into `fetch/xparse.py`.
  - **Third-party read-only search/X APIs & MCPs** — Exa, Tavily, Perplexity Sonar, TwitterAPI.io, or Apify (no personal X credentials).
- **Red lines**: never pass personal `auth_token`/`ct0` cookies to scrapers, keep all automation strictly read-only, store API keys outside the repo (`chmod 600`), and never leak personal watch lists into published issues (unreplaced `<account_1>` / `<zh_account_1>` placeholders in `fetch/x-recall.md` and `x-sources.md` are skipped automatically).

## Configuration

Environment variables take precedence over `site/CONFIG`.

| Variable | `CONFIG` key | Description | Default |
|---|---|---|---|
| `SITE_DOMAIN` | `DOMAIN` | Site domain | `digest.example.com` |
| `SITE_SLUG` | `SLUG` | Optional path prefix (empty = root `/`; non-empty requires `>= 8` URL-safe chars) | empty (`--init --encrypt` generates random) |
| `DIGEST_ENCRYPT` | `ENCRYPT` | Opt-in client-side AES-256-GCM password encryption (`1`/`true`/`yes`/`on`) | `0` (public) |
| `CF_ACCOUNT_ID` | `ACCOUNT_ID` | Cloudflare account ID | — |
| `CF_ZONE_ID` | `ZONE_ID` | Zone of the domain (optional) | — |
| `CF_PAGES_PROJECT` | `PROJECT` | Cloudflare Pages project name | `ai-digest` |
| `CLOUDFLARE_API_TOKEN` / `_FILE` | — | Deploy token (Pages · Edit only) | — |
| `DIGEST_SITE_PASSWORD_FILE` | — | Site password file (mode 600; used only when `ENCRYPT=1`) | `~/.config/ai-digest/site-password` |
| `DIGEST_SITE_SALT_FILE` | — | PBKDF2 salt file (auto-generated when `ENCRYPT=1`) | `~/.config/ai-digest/site-salt` |
| `DIGEST_LANG` | `LANG` | Digest & site UI language: `en` or `zh-CN` (`--lang` overrides) | `en` |
| `DIGEST_LANG_ALT` | `LANG_ALT` | Optional companion site URL/path in the other language (e.g. `zh-CN:/zh-CN/`) | — |
| `DIGEST_ROOT_LATEST` | `ROOT_LATEST` | When `SLUG` is set, opt-in root `/latest`, `/daily/latest`, `/weekly/latest` (and `/zh/...`, `/en/...`) static pages (`1`/`true`/`yes`/`on`) | `0` (off) |
| `DIGEST_TZ` | — | IANA time zone defining a natural calendar day | `UTC` |
| `DIGEST_ROOT` | — | Working directory for `runs/`, `html/`, `site/`, `ledger.json`, `logs/`, `state/` | repo root |
| `LEDGER_PATH` | — | Explicit path to `ledger.json` (auto-created on first `ledger.py add`) | `<DIGEST_ROOT>/ledger.json` |
| `DIGEST_PREFERENCES_FILE` | — | Topic ranking weights JSON (see `fetch/preferences.example.json`) | `fetch/preferences.json` |
| `DIGEST_PRIVACY_PATTERNS_FILE` | — | Extra regexes checked before publishing | `site/privacy-patterns.txt` |
| `DIGEST_DEMO_PASSWORD` | — | Optional password for `publish.py --demo` (unset = public demo) | — |
| `GITHUB_TOKEN` / `GH_TOKEN` | — | Optional, raises GitHub API limits during fetch | — |

Sources live in `fetch/sources.toml`; selection rules live in `filter-notes.md`.

## Contributing & License

Pull requests are welcome (`main` is protected; see [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md)). CSS design tokens in `render_html.py` are adapted from [AIHOT](https://github.com/KKKKhazix/AIHOT) (MIT, © 2026 数字生命卡兹克; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/AIHOT-MIT.txt](licenses/AIHOT-MIT.txt)). Licensed under [MIT](LICENSE).
