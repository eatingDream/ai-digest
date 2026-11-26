<div align="center">

<img src="docs/logo.svg" alt="AI Digest logo" width="68" height="68">

# AI Digest

**An agent-driven pipeline powered by a single Rust binary (`ai-digest`) + TypeScript/CSS editorial frontend — turning ~155 open sources into a daily & weekly AI digest published as a hardened, GEO-ready static site.**

[![CI](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/eatingDream/ai-digest?color=b85432)](https://github.com/eatingDream/ai-digest/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-4f6d5a)](LICENSE)
![Rust](https://img.shields.io/badge/rust-1.88%2B-dea584?logo=rust&logoColor=1f1c18)
![TypeScript](https://img.shields.io/badge/typescript-5.x-3178c6?logo=typescript&logoColor=white)
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

- **Single self-contained Rust binary (`ai-digest`)** — zero runtime interpreter dependencies: recall (`fetch`), curation helpers (`ledger`, `cand`, `check-cands`, `page`, `weekly-late`, `xparse`), editorial rendering (`render`), static site & GEO publishing (`publish`), and native verification (`verify`) all ship in one compiled binary with embedded frontend assets (`web/dist/`).
- **Warm editorial TypeScript/CSS reader (`web/`)** — single-file HTML with dark-first obsidian-charcoal/terracotta & warm-paper light themes (zero white flash), 52 px auto-hide masthead, collapsible search drawer (`/` shortcut) with 2-row collapsible frequency-sorted tag chips, live-tracked accordion sidebar TOC + mobile dock/sheet, build-time inline LaTeX math subset rendering, keyboard navigation (`j`/`k`/`[`/`]`), copy-permalink anchors, and built-in `/favicon.svg`.
- **Native bilingual routes & static `latest`** — auto-pairs sibling `<stem>.en.md` files into symmetric `/zh/` and `/en/` site trees (or single-file `render --md-en` bilingual pages / `LANG_ALT` companion links), plus direct HTTP 200 `/daily/latest`, `/weekly/latest`, `/zh/daily/latest`, and `/en/daily/latest` pages served with `no-store`.
- **Hardened public-first static publishing & GEO (`ai-digest publish`)** — publishes a clean, password-free root site by default (`ENCRYPT=0`, `SLUG=`) with build-time SHA-256 hash-locked CSP (zero `'unsafe-inline'` scripts), HSTS/COOP/CORP/Permissions-Policy, Markdown XSS & raw `<img>` neutralization, HTML5 entity-aware privacy scanning, and Level 5/5 **GEO / Agent Readiness** (`robots.txt` Content Signals & AI bot rules, `sitemap.xml` with `hreflang`, `llms.txt`/`llms-full.txt`, Atom feeds, JSON-LD, per-page `.md` mirrors, Agent Skills discovery, and a compiled Cloudflare Pages `_worker.js` for `Accept: text/markdown` and `/.well-known/search` via `GET`/`HEAD`/`QUERY`). Opt-in client-side AES-256-GCM encryption + private random `SLUG` stays one flag away (`--init --encrypt` / `ENCRYPT=1`, disabling public GEO and keeping `Disallow: /`).
- **Wide open recall + safe optional X sources (`ai-digest fetch`)** — pulls ~155 zero-key RSS/Atom/arXiv/HF/GitHub/blog/HN/Reddit sources in parallel with SSRF-hardened DNS-pinned HTTP and `ai-digest ledger` deduplication (`★` pushed / `☆` seen). Optional X/Twitter recall works **without a Grok Bot** via free or paid safe APIs (see [Safe X recall without a Grok Bot](#safe-x-recall-without-a-grok-bot)).

## Architecture

<p align="center"><img src="docs/architecture.en.svg" alt="AI Digest architecture diagram" width="100%"></p>

1. **Wide recall & triage (`ai-digest fetch`)** — parallel async Rust collection from ~155 open sources (`fetch/sources.toml`, zero API keys, vanity metrics stripped) + `ai-digest ledger` history tracking → `triage.md`, `triage-papers.md`, and `candidates.jsonl`.
2. **Content-first LLM curation (`prompts/<lang>/`)** — platform-agnostic daily/weekly prompts verify primary sources via SSRF-hardened `ai-digest page` against `filter-notes.md` and write seven editorial sections (`Models`, `Papers`, `Tools & Projects`, `Industry & Products`, `Compute & Infrastructure`, `Safety & Policy`, `Opinions & Essays`).
3. **Single-file render & hardening (`ai-digest render`, `ai-digest publish`)** — embeds minified TypeScript/CSS assets (`web/dist/`) into self-contained pages with inline math, collapsible search, `/zh/` & `/en/` routing, static `latest`, Level 5/5 GEO + `_worker.js`, entity-aware privacy scan, and build-time SHA-256 hash-locked `_headers`.
4. **Edge deploy & dual-runtime verification** — incremental Cloudflare Pages deploy followed by `ai-digest verify` (native Rust) and optional `node web/dist/verify_decrypt.mjs` (Node WebCrypto parity) checking every published route (exact-match in public mode, AES-256-GCM + gzip decryption in encrypted mode).

## Live demo

The public demo at **[ai-digest-demo.pages.dev](https://ai-digest-demo.pages.dev)** is password-free, updated automatically on every `main` push from the synthetic issues in `examples/`, and ships both `/en/` and `/zh-CN/` archives plus static `latest` links. Reproduce it locally with one command:

```bash
ai-digest publish --demo --out dist/          # dist/en/ + dist/zh-CN/ (pass --lang en for single)
```

## Quick start

Download a prebuilt binary from [GitHub Releases](https://github.com/eatingDream/ai-digest/releases) (Linux x86_64/arm64, macOS arm64/x86_64), or build from source with Rust 1.88+ (Node 22+ is only needed when editing `web/` TypeScript/CSS or running `web/dist/verify_decrypt.mjs` / `npx wrangler`):

```bash
# Build the self-contained release binary (committed web/dist/ assets embedded automatically)
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo build --release --locked

# 1. Render a sample issue (add --lang zh-CN for Chinese, or --md-en for a single-file bilingual toggle)
./target/release/ai-digest render examples/sample-digest.en.md /tmp/sample.html
./target/release/ai-digest render examples/sample-digest.zh-CN.md /tmp/bilingual.html --md-en examples/sample-digest.en.md

# 2. Build the public demo site locally (zero secrets; set DIGEST_DEMO_PASSWORD='demo' to test encryption)
./target/release/ai-digest publish --demo --out dist/

# 3. Run the Rust & frontend test suites
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo test --locked
npm --prefix web ci && npm --prefix web run typecheck && npm --prefix web test
```

### Run your own digest

```bash
# 1. Initialize config (public root site by default; add --encrypt for password + random SLUG)
ai-digest publish --init                      # or: cp site/CONFIG.example site/CONFIG
export CLOUDFLARE_API_TOKEN=...               # scope: Account · Cloudflare Pages · Edit
# Optional password mode (only when ENCRYPT=1):
# install -m 600 /dev/stdin ~/.config/ai-digest/site-password

# 2. Fetch candidates (set DIGEST_TZ first; GITHUB_TOKEN/GH_TOKEN optional for higher API limits)
ai-digest fetch --daily                       # weekly: --weekly

# 3. Curate with an AI agent: run prompts/en/daily.md (or weekly.md; prompts/zh-CN/ for Chinese) -> digest-<P>.md

# 4. Publish (auto-discovers sibling <stem>.en.md on zh-CN sites for /zh/ & /en/ trees + static latest)
ai-digest publish --issue daily/2026-01-15 --md digest-2026-01-15.md
```

See [fetch/README.md](fetch/README.md) for channels and helper subcommands, and [site/README.md](site/README.md) for optional encryption, `/zh/` & `/en/` routing, `latest` pages, `--rebuild`, `--rerender-all`, `--rotate-password`, and `--remove`.

## Use with AI agents

[AGENTS.md](AGENTS.md) is the entry point for coding agents (repo map, pipeline, conventions, safety rules). Open the repo root in any agent runner with shell and file tools, set `DIGEST_LANG` and `DIGEST_TZ`, and run `prompts/<lang>/{papers,daily,weekly}.md` (each begins with a run contract; subcommands support `--help`, `--version`, and `--json`).

### Safe X recall without a Grok Bot

X/Twitter recall is **100% optional** and never requires a Grok Bot or personal X cookies. Full setup details and query templates live in [fetch/x-recall.md](fetch/x-recall.md):

- **Free & zero-risk paths**:
  - **Open-feeds-only mode (default)** — prompts skip X cleanly and curate from ~155 open RSS/Atom/arXiv/HF/GitHub/lab-blog/HN/Reddit feeds.
  - **Read-only web search grounding** — query `site:x.com/<handle>` over your date window using free-tier search tools (Gemini Search Grounding, Brave Search, Tavily, Exa, Perplexity) without logging into X.
  - **Official cross-posted RSS & self-hosted bridges** — subscribe to Bluesky (`https://bsky.app/profile/<handle>/rss`), Mastodon, Substack, or self-hosted [RSSHub](https://docs.rsshub.app/) public read-only routes in `fetch/sources.toml`. *(Note: X's official Free API tier does not allow general timeline/search reads.)*
- **Paid & official API paths**:
  - **xAI Grok API (`x_search` tool)** *(recommended paid)* — pay-as-you-go official API (`api.x.ai`, typically pennies/day) with handle & date filters and zero X account risk.
  - **Official X API v2 (Pay-Per-Use / Basic)** — read-only Bearer Token (`GET /2/tweets/search/recent`), whose JSON plugs directly into `ai-digest xparse`.
  - **Third-party read-only search/X APIs & MCPs** — Exa, Tavily, Perplexity Sonar, TwitterAPI.io, or Apify (no personal X credentials).
- **Red lines**: never pass personal `auth_token`/`ct0` cookies to scrapers, keep all automation strictly read-only, store API keys outside the repo (`chmod 600`), and never leak personal watch lists into published issues (unreplaced `<account_1>` / `<zh_account_1>` placeholders in `fetch/x-recall.md` and `x-sources.md` are skipped automatically).

## Configuration

Environment variables take precedence over `site/CONFIG`.

| Variable | `CONFIG` key | Description | Default |
|---|---|---|---|
| `SITE_DOMAIN` | `DOMAIN` | Site domain | `digest.example.com` |
| `SITE_SLUG` | `SLUG` | Optional path prefix (empty = root `/`; non-empty requires `>= 8` URL-safe chars) | empty (`--init --encrypt` generates random) |
| `DIGEST_ENCRYPT` | `ENCRYPT` | Opt-in client-side AES-256-GCM password encryption (`1`/`true`/`yes`/`on`; disables public GEO) | `0` (public) |
| `DIGEST_GEO_AI_TRAIN` | `GEO_AI_TRAIN` | Public `Content-Signal` & training-crawler policy (`yes` or `no`; `ENCRYPT=0` only) | `yes` |
| `DIGEST_GEO_WORKER` | `GEO_WORKER` | Emit Pages `_worker.js` for `Accept: text/markdown` & `/.well-known/search` (`1`/`0`; `ENCRYPT=0` only) | `1` (on) |
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
| `LEDGER_PATH` | — | Explicit path to `ledger.json` (auto-created on first `ai-digest ledger add`) | `<DIGEST_ROOT>/ledger.json` |
| `DIGEST_PREFERENCES_FILE` | — | Topic ranking weights JSON (see `fetch/preferences.example.json`) | `fetch/preferences.json` |
| `DIGEST_PRIVACY_PATTERNS_FILE` | — | Extra regexes checked before publishing | `site/privacy-patterns.txt` |
| `DIGEST_DEMO_PASSWORD` | — | Optional password for `ai-digest publish --demo` (unset = public demo) | — |
| `GITHUB_TOKEN` / `GH_TOKEN` | — | Optional, raises GitHub API limits during fetch | — |

Sources live in `fetch/sources.toml`; selection rules live in `filter-notes.md`.

## AI Agent Integration

- **Install the portable Agent Skill**: [`skills/ai-digest/SKILL.md`](skills/ai-digest/SKILL.md) (or `npx skills add eatingDream/ai-digest`) teaches Claude Code, Codex, Cursor, OpenCode, and Antigravity how to query published sites and run the local pipeline.
- **Machine-readable repo & site indexes**: Read [`llms.txt`](llms.txt) and [`AGENTS.md`](AGENTS.md) in this repo; every public (`ENCRYPT=0`) deployed site also publishes `/llms.txt`, `/llms-full.txt`, `/sitemap.xml`, `/feed.xml`, `/.well-known/agent-skills/index.json`, `Accept: text/markdown` content negotiation, and `GET`/`HEAD`/`QUERY` `/.well-known/search?q=<query>`.

## Contributing & License

Pull requests are welcome (`main` is protected; see [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md)). CSS design tokens in `web/src/css/tokens.css` are adapted from [AIHOT](https://github.com/KKKKhazix/AIHOT) (MIT, © 2026 数字生命卡兹克; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/AIHOT-MIT.txt](licenses/AIHOT-MIT.txt)). Licensed under [MIT](LICENSE).
