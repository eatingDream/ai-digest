<div align="center">

# AI DIGEST

**An agent-driven pipeline that turns ~150 sources into a daily and weekly AI digest — rendered to a single-file HTML and published as an encrypted static site.**

[![CI](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream/ai-digest/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Python](https://img.shields.io/badge/python-3.11%2B-3776AB?logo=python&logoColor=white)
![Cloudflare Pages](https://img.shields.io/badge/deploy-Cloudflare%20Pages-F38020?logo=cloudflare&logoColor=white)

English | [简体中文](README.zh-CN.md)

[Live demo](https://ai-digest-demo.pages.dev) — password `demo` · [English](https://ai-digest-demo.pages.dev/en/) · [简体中文](https://ai-digest-demo.pages.dev/zh-CN/)

<img src="docs/screenshot.en.png" alt="Rendered sample issue (synthetic data)" width="860">

<sub>A rendered issue built from the synthetic data in <code>examples/</code>. English and Simplified Chinese are both supported (<code>DIGEST_LANG</code>).</sub>

</div>

## Features

- **Wide recall, no API keys** — `fetch/fetch.py` pulls ~150 feeds in parallel (RSS, HTML, arXiv, Hugging Face, GitHub releases, Hacker News, Reddit), de-duplicates, tags and ranks candidates into compact triage lists.
- **Layered, content-first curation** — platform-agnostic agent prompts in `prompts/`. Selection rests on substance, never popularity counts; scope is the whole AI field.
- **Never repeat yourself** — `fetch/ledger.py` records what was already pushed (★) or only seen (☆), so repeats appear only with real new developments.
- **Daily and weekly issues** — seven sections: Models, Papers, Tools & Projects, Industry & Products, Compute & Infrastructure, Safety & Policy, Opinion & Essays.
- **English and Chinese** — one switch (`DIGEST_LANG=en|zh-CN`) sets the language of the prompts, the fetch labels and the whole reader UI; `render_html.py --md-en` / `publish.py --md-en` (auto-discovered as `<stem>.en.md` on `zh-CN` sites) bundles a single-page bilingual toggle, and `LANG_ALT` optionally links separate companion sites. English is the default.
- **Polished reader** — `render_html.py` emits self-contained HTML with a full-title sidebar TOC, instant client-side keyword & tag filtering (scoped per language pane), `#fragment` deep-link scroll after decryption, light/dark themes and a wide-screen layout.
- **Encrypted static hosting** — `publish.py` encrypts every page client-side (AES-256-GCM, PBKDF2-SHA256, 600k iterations), deploys incrementally to Cloudflare Pages and decrypt-verifies each page.
- **CI/CD included** — tests, secret scanning, tagged releases and a demo-site workflow (optional Cloudflare Pages deploy).

## Architecture

<p align="center"><img src="docs/architecture.en.svg" alt="ai-digest architecture" width="100%"></p>

1. **Sources** are read-only: ~153 open feeds, research and code hubs, and communities (`fetch/sources.toml`), plus an optional capability-gated X/Twitter search (`fetch/x-recall.md`) when your agent runner provides read-only X tools.
2. **Fetch & triage** (`fetch/`) runs as a plain script and writes `triage.md`, `triage-papers.md` and `candidates.jsonl`. A cheap papers prefilter agent narrows the paper list.
3. **LLM curation** (`prompts/<lang>/`, `filter-notes.md`, `x-sources.md`) checks primary sources and writes `digest-<P>.md`.
4. **Render & encrypt** turns Markdown into HTML, injects navigation and TOC pages, encrypts every page and verifies the round trip.
5. **Deploy & read**: `wrangler pages deploy` publishes ciphertext only; readers unlock pages in the browser with Web Crypto.

A scheduler (any agent runner with shell, file and web tools) triggers the daily and weekly runs; GitHub Actions covers CI, releases and the demo site.

## Quick start

Requirements: Python 3.11+ (CI uses 3.12), Node 20 + `npx` (only for deploying with wrangler).

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt          # requests, lxml, markdown, cryptography, pytest

# 1. Render the sample issue (or bundle zh-CN + en into one toggleable page with --md-en)
python3 render_html.py examples/sample-digest.en.md /tmp/sample.html      # add --lang zh-CN for Chinese
python3 render_html.py examples/sample-digest.zh-CN.md /tmp/bilingual.html --md-en examples/sample-digest.en.md

# 2. Build the encrypted demo site (no deploy, no secrets)
DIGEST_DEMO_PASSWORD='demo' python3 publish.py --demo --out dist/
# English at dist/en/, Chinese at dist/zh-CN/ — every page is decrypt-verified during the build
# add --lang en (or zh-CN) to build a single language at dist/demo/

# 3. Run the tests
pytest -q
```

### Run your own digest

```bash
# 1. Configure the site (generates site/CONFIG with a random 22-char SLUG; set LANG=en or zh-CN)
python3 publish.py --init                                # or: cp site/CONFIG.example site/CONFIG (replace <...> placeholders)
install -m 600 /dev/stdin ~/.config/ai-digest/site-password   # paste a password of 16+ chars, then Ctrl-D
export CLOUDFLARE_API_TOKEN=...                          # scope: Account · Cloudflare Pages · Edit

# 2. Fetch candidates for yesterday (set DIGEST_TZ first; GITHUB_TOKEN/GH_TOKEN optional for higher API limits)
python3 fetch/fetch.py --daily                           # weekly: --weekly (labels follow DIGEST_LANG / --lang)

# 3. Let an agent curate: run prompts/en/daily.md (or weekly.md; prompts/zh-CN/ for Chinese) in your agent runner → digest-<P>.md

# 4. Publish (ledger.json is initialized automatically on first write)
python3 publish.py --issue daily/2026-01-15 --md digest-2026-01-15.md
```

More commands (`--rebuild`, `--rerender-all`, `--rotate-password`, `--remove`) are listed in [site/README.md](site/README.md); fetch options and output files in [fetch/README.md](fetch/README.md).

## Use with AI agents

[AGENTS.md](AGENTS.md) is the entry point for coding agents (Cursor, Claude Code, Codex, …): repo map, commands, pipeline, conventions and safety rules. To produce a digest with an agent, open the repo root in your agent runner with shell and file read/write tools, set `DIGEST_LANG` and `DIGEST_TZ`, and give it `prompts/en/daily.md` (or `weekly.md`; `prompts/zh-CN/` for Chinese). Read-only X/Twitter search tools (`search_posts_all`, `get_users_posts`, etc.) are **optional**: if your runner lacks them, the prompt cleanly skips X recall and curates from the ~153 open feeds; if available, you can optionally customize personal watch-circle handles (`<account_1>`, `<zh_account_1>`) in `fetch/x-recall.md` and `x-sources.md` (queries with unreplaced placeholders are automatically skipped). Each prompt starts with a run contract listing inputs, outputs and the done condition. Every CLI has `--help` with examples; several have `--json`.

## Configuration

Environment variables take precedence over `site/CONFIG`.

| Variable | `CONFIG` key | Description | Default |
|---|---|---|---|
| `SITE_DOMAIN` | `DOMAIN` | Site domain | `digest.example.com` |
| `SITE_SLUG` | `SLUG` | Unguessable path prefix (`>= 8` URL-safe chars; never change after publishing) | random (`--init`) |
| `CF_ACCOUNT_ID` | `ACCOUNT_ID` | Cloudflare account ID | — |
| `CF_ZONE_ID` | `ZONE_ID` | Zone of the domain (optional) | — |
| `CF_PAGES_PROJECT` | `PROJECT` | Cloudflare Pages project name | `ai-digest` |
| `CLOUDFLARE_API_TOKEN` / `_FILE` | — | Deploy token (Pages · Edit only) | — |
| `DIGEST_SITE_PASSWORD_FILE` | — | File holding the site password (mode 600) | `~/.config/ai-digest/site-password` |
| `DIGEST_SITE_SALT_FILE` | — | PBKDF2 salt file (auto-generated) | `~/.config/ai-digest/site-salt` |
| `DIGEST_LANG` | `LANG` | Language of the digest and the site UI: `en` or `zh-CN` (`--lang` overrides) | `en` |
| `DIGEST_LANG_ALT` | `LANG_ALT` | Optional companion site URL/path in the other language (e.g. `zh-CN:/zh-CN/`) | — |
| `DIGEST_ROOT_LATEST` | `ROOT_LATEST` | Opt-in root `/latest`, `/daily/latest`, `/weekly/latest` static 302 redirects (`1`/`true`/`yes`/`on`; slug-scoped `/<slug>/...latest` aliases are always generated in `_redirects`) | `0` (off) |
| `DIGEST_TZ` | — | IANA time zone that defines a "day" | `UTC` |
| `DIGEST_ROOT` | — | Working directory for `runs/`, `html/`, `site/`, `ledger.json`, `logs/`, and `state/` | repo root |
| `LEDGER_PATH` | — | Explicit path to `ledger.json` (auto-created on first `ledger.py add`) | `<DIGEST_ROOT>/ledger.json` |
| `DIGEST_PREFERENCES_FILE` | — | Topic weights JSON (see `fetch/preferences.example.json`) | `fetch/preferences.json` |
| `DIGEST_PRIVACY_PATTERNS_FILE` | — | Extra regexes checked before publishing | `site/privacy-patterns.txt` |
| `DIGEST_DEMO_PASSWORD` | — | Password for `publish.py --demo` | — |
| `GITHUB_TOKEN` / `GH_TOKEN` | — | Optional, raises GitHub API limits during fetch | — |

Sources live in `fetch/sources.toml`; the selection rules are in `filter-notes.md`.

## Project structure

```text
ai-digest/
├── fetch/              # recall: fetch.py, ledger.py, sources.toml, X recall protocol
├── prompts/            # agent prompts per language: en/, zh-CN/ (daily, weekly, papers)
├── i18n.py             # UI strings, date formats and label vocabulary (en / zh-CN)
├── render_html.py      # Markdown → self-contained HTML
├── publish.py          # build, encrypt, deploy, rotate password, --demo
├── verify_decrypt.mjs  # headless decrypt verification
├── site/               # encryption shell, _headers, CONFIG.example, docs
├── examples/           # synthetic sample issues (en, zh-CN)
├── tests/              # pytest suite
├── AGENTS.md           # guide for AI coding agents (CLAUDE.md points to it)
├── CHANGELOG.md
├── scripts/            # check_sanitized.sh
├── docs/               # architecture diagrams, screenshots
├── filter-notes.md     # selection rules (also .zh-CN.md)
├── x-sources.md        # reader-preference profile, template (also .zh-CN.md)
└── licenses/           # third-party license texts
```

## CI/CD

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push / PR to `main` | `py_compile`, `pytest`, `node --check`, gitleaks over full history, `scripts/check_sanitized.sh` |
| `release.yml` | tag `v*` | runs tests, then creates a GitHub Release with a source tarball and the demo `dist` zip |
| `demo.yml` | push to `main` (renderer / publisher / examples / site) or manual | builds the demo site, uploads it as an artifact and deploys it to Cloudflare Pages |

The demo deploy is skipped while its secrets are unset. To enable it, add in **Settings → Secrets and variables → Actions**:

- Secret `CLOUDFLARE_API_TOKEN` — scope **Account · Cloudflare Pages · Edit** only
- Secret `CLOUDFLARE_ACCOUNT_ID`
- Optional variables `CF_PAGES_PROJECT` (default `ai-digest-demo`) and `DIGEST_DEMO_PASSWORD` (default `demo`)

Dependabot updates GitHub Actions and pip dependencies weekly.

## Contributing

Pull requests are welcome — `main` is protected, so changes go through a PR with green CI. See [CONTRIBUTING.md](CONTRIBUTING.md); report vulnerabilities privately per [SECURITY.md](SECURITY.md).

## Acknowledgements

The CSS design tokens embedded in `render_html.py` are adapted from [AIHOT](https://github.com/KKKKhazix/AIHOT) (MIT, © 2026 数字生命卡兹克). See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/AIHOT-MIT.txt](licenses/AIHOT-MIT.txt).

## License

[MIT](LICENSE) — free to use, modify, and distribute, including commercially. Third-party material keeps its own license.
