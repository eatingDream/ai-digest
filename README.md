<div align="center">

# AI DIGEST

**An agent-driven pipeline that turns ~150 sources into a daily and weekly AI digest — rendered to a single-file HTML and published as a static site (with optional client-side encryption).**

[![CI](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Python](https://img.shields.io/badge/python-3.11%2B-3776AB?logo=python&logoColor=white)
![Cloudflare Pages](https://img.shields.io/badge/deploy-Cloudflare%20Pages-F38020?logo=cloudflare&logoColor=white)

English | [简体中文](README.zh-CN.md)

[Live demo](https://ai-digest-demo.pages.dev) · [English](https://ai-digest-demo.pages.dev/en/) · [简体中文](https://ai-digest-demo.pages.dev/zh-CN/)

<img src="docs/screenshot.en.png" alt="Rendered sample issue (synthetic data)" width="860">

<sub>A rendered issue built from the synthetic data in <code>examples/</code>. English and Simplified Chinese are both supported (<code>DIGEST_LANG</code>).</sub>

</div>

## How it works

<p align="center"><img src="docs/architecture.en.svg" alt="ai-digest architecture" width="100%"></p>

1. **Wide recall & triage (`fetch/`)** — pulls ~155 read-only open sources in parallel (`fetch/sources.toml`, no API keys), deduplicates, tracks pushed (★) / seen (☆) history in `fetch/ledger.py` so items repeat only on substantive new developments, and writes `triage.md`, `triage-papers.md`, and `candidates.jsonl` (plus optional capability-gated X/Twitter search via `fetch/x-recall.md`).
2. **Content-first LLM curation (`prompts/<lang>/`)** — platform-agnostic daily/weekly prompts (with a lightweight papers prefilter) verify primary sources against `filter-notes.md` and write seven sections (`Models`, `Papers`, `Tools & Projects`, `Industry & Products`, `Compute & Infrastructure`, `Safety & Policy`, `Opinion & Essays`) — selected by substance, never popularity counts.
3. **Single-file HTML reader (`render_html.py`)** — emits self-contained HTML with a full-title sidebar TOC, instant client-side keyword/tag filtering, `#fragment` deep links, warm paper/charcoal light & dark themes, and bilingual support (`DIGEST_LANG=en|zh-CN`, symmetric `/zh/` & `/en/` site paths, standalone `--md-en`, or `LANG_ALT` companion links).
4. **Static hosting (`publish.py`)** — publishes a clean, root-mounted public static site by default (`ENCRYPT=0`, `SLUG=`), generates symmetric `/zh/` & `/en/` trees whenever `*.en.md` companions exist, serves static `/{daily,weekly,}/latest` pages directly (HTTP 200, URL stays on `latest`, `no-store`), runs a pre-publish privacy scan, deploys incrementally to Cloudflare Pages, and verifies every page via `verify_decrypt.mjs`. Optional client-side AES-256-GCM encryption + private random `SLUG` can be enabled with `--init --encrypt` or `ENCRYPT=1`.

## Quick start

Requirements: Python 3.11+ (CI uses 3.12), Node 20 + `npx` (only for `wrangler` deploys and `verify_decrypt.mjs`).

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt          # requests, lxml, markdown, cryptography, pytest

# 1. Render a sample issue (use --md-en to bundle zh-CN + en into one toggleable page)
python3 render_html.py examples/sample-digest.en.md /tmp/sample.html      # add --lang zh-CN for Chinese
python3 render_html.py examples/sample-digest.zh-CN.md /tmp/bilingual.html --md-en examples/sample-digest.en.md

# 2. Build the public demo site (no deploy, no secrets; set DIGEST_DEMO_PASSWORD='demo' for encrypted mode)
python3 publish.py --demo --out dist/                    # dist/en/ + dist/zh-CN/ (--lang for single)

# 3. Run the tests
pytest -q
```

### Run your own digest

```bash
# 1. Configure site (public root-mounted by default; pass --encrypt for private SLUG + password)
python3 publish.py --init                                # or: cp site/CONFIG.example site/CONFIG
export CLOUDFLARE_API_TOKEN=...                          # scope: Account · Cloudflare Pages · Edit
# Optional password mode (only when ENCRYPT=1):
# install -m 600 /dev/stdin ~/.config/ai-digest/site-password

# 2. Fetch candidates (set DIGEST_TZ first; GITHUB_TOKEN/GH_TOKEN optional for higher API limits)
python3 fetch/fetch.py --daily                           # weekly: --weekly

# 3. Curate with an agent: run prompts/en/daily.md (or weekly.md; prompts/zh-CN/ for Chinese) → digest-<P>.md

# 4. Publish (auto-discovers sibling <stem>.en.md on zh-CN sites to generate /zh/ & /en/ trees + static latest pages)
python3 publish.py --issue daily/2026-01-15 --md digest-2026-01-15.md
```

See [fetch/README.md](fetch/README.md) for fetch options, channels, and helper CLIs, and [site/README.md](site/README.md) for optional encryption, `/zh/` & `/en/` routing, `latest` pages, `--rebuild`, `--rerender-all`, `--rotate-password`, and `--remove`.

## Use with AI agents

[AGENTS.md](AGENTS.md) is the entry point for coding agents (repo map, pipeline, conventions, safety rules). To run a digest, open the repo root in any agent runner with shell and file tools, set `DIGEST_LANG` and `DIGEST_TZ`, and run `prompts/<lang>/{papers,daily,weekly}.md` (each starts with a run contract; CLIs expose `--help` and `--json`). Read-only X/Twitter tools (`search_posts_all`, `get_users_posts`, etc.) are **optional**: without them, prompts cleanly skip X recall and curate from the open feeds; if available, customize `<account_1>` / `<zh_account_1>` in `fetch/x-recall.md` and `x-sources.md` (unreplaced placeholders are skipped automatically).

## Configuration

Environment variables take precedence over `site/CONFIG`.

| Variable | `CONFIG` key | Description | Default |
|---|---|---|---|
| `SITE_DOMAIN` | `DOMAIN` | Site domain | `digest.example.com` |
| `SITE_SLUG` | `SLUG` | Optional path prefix (empty = root-mounted `/`; non-empty requires `>= 8` URL-safe chars) | empty (`--init --encrypt` generates random) |
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
