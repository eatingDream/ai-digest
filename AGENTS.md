# AGENTS.md

Guidance for AI coding agents (Cursor, Claude Code, Codex, …) working in this repository. Humans: see [README.md](README.md).

## What this is

`ai-digest` turns ~150 sources into a daily and a weekly AI digest: a script fetches and triages candidates, an LLM agent curates them into Markdown, `render_html.py` renders one-file HTML, and `publish.py` encrypts every page client-side and deploys a static site to Cloudflare Pages. Output language is `en` (default) or `zh-CN` (`DIGEST_LANG` / `--lang`).

Python 3.11+, no framework. Runtime deps: `requests`, `lxml`, `markdown`, `cryptography`. Node is only used for `verify_decrypt.mjs` and `npx wrangler`.

## Setup, test, lint (run before every PR)

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt
python -m py_compile *.py fetch/*.py tests/*.py      # CI "compile" step
pytest -q                                            # all tests, offline, ~5 s
node --check verify_decrypt.mjs
scripts/check_sanitized.sh                           # forbidden-pattern guard (must print "clean")
```

CI (`.github/workflows/ci.yml`) also runs gitleaks over the full history. Required checks on `main`: `Python (compile + pytest)`, `Node (syntax check)`, `Secret scan (gitleaks) + sanitization guard`.

Build the demo site locally (no secrets, no network): `DIGEST_DEMO_PASSWORD=demo python3 publish.py --demo --out dist/` → `/en/` and `/zh-CN/` (add `--lang en` for a single `/demo/`).

## Repo map

| Path | Purpose |
|---|---|
| `fetch/fetch.py` | Recall: parallel fetch of `fetch/sources.toml` → `runs/<P>/fetch/{candidates.jsonl,triage.md,triage-papers.md,report.json,report.md}` |
| `fetch/cand.py`, `page.py`, `xparse.py`, `weekly_late.py`, `check_cands.py`, `ledger.py` | Small helpers the curation agent calls (candidate lookup, slim page fetch, X result compaction, weekly "late signal" list, papers-list validation, ledger queries/writes) |
| `fetch/x-recall.md` | X (Twitter) search protocol the agent follows (read-only) |
| `prompts/<en|zh-CN>/{papers,daily,weekly}.md` | The agent prompts. Self-contained; start each with a "Run contract" table |
| `filter-notes.md`, `x-sources.md` | Selection rules and a reader-preference profile (templates); `.zh-CN.md` variants |
| `render_html.py` | Markdown → single-file HTML (CSS embedded). Parses English and Chinese label vocabularies |
| `publish.py` | Nav, index, privacy scan, AES-256-GCM encryption, deploy, `--demo` |
| `i18n.py` | All UI strings, date formats, fetch labels and the bilingual Markdown vocabulary (`en`, `zh-CN`) |
| `site/` | Password page template, `CONFIG.example`, site docs. `site/CONFIG` is local and gitignored |
| `examples/` | Synthetic sample issues (`sample-{digest,weekly}.{en,zh-CN}.md`); used by tests, demo and docs |
| `tests/` | pytest suite (offline) |
| `version.py` | `__version__` (also `--version` on `publish.py`, `render_html.py`, `fetch/fetch.py`) |

Working data is gitignored: `runs/`, `logs/`, `html/`, `ledger.json`, `digest-*.md`, `weekly-*.md`, `site/plain|sources|public`, `dist/`.

## How the pipeline runs

Times are local (`DIGEST_TZ`, default UTC). A "day" `P` is a natural calendar day, windows are half-open `[P 00:00, P+1 00:00)`.

| Step | When | Command / prompt | Output |
|---|---|---|---|
| 1. Papers prefilter | 00:10 | `prompts/<lang>/papers.md` (runs `fetch/fetch.py --daily --reuse`) | `runs/P/fetch/`, `runs/P/paper-cands.md` |
| 2. Daily curation | 00:30 | `prompts/<lang>/daily.md` | `digest-P.md`, `runs/P/filter-log.md`, ledger updates |
| 3. Publish | end of step 2 | `python3 publish.py --issue daily/P --md digest-P.md` | encrypted page `daily/P`, rebuilt index/nav, deploy |
| 4. Weekly | Monday 01:30 | `prompts/<lang>/weekly.md` (uses `fetch/weekly_late.py`) | `weekly-L.md` (`L` = `YYYY-MM-DD_MM-DD`, Monday_Sunday), published `weekly/L` |

File names: daily `digest-YYYY-MM-DD.md`, weekly `weekly-YYYY-MM-DD_MM-DD.md`; issue ids `daily/YYYY-MM-DD`, `weekly/YYYY-MM-DD_MM-DD`.

Markdown contract (what `render_html.py` parses; either language's labels work): title `**AI Daily · M/D**` or `**AI Weekly · MM/DD–MM/DD**` (zh: `**AI精选日报 · M/D**`); seven `##` sections (Models, Papers, Tools & Projects, Industry & Products, Compute & Infrastructure, Safety & Policy, Opinions & Essays); numbered `###` items with a metadata line and the fields In brief / Why it matters / Details (/ Follow-up in weeklies). Copy the shape of `examples/sample-digest.en.md`.

## Run a digest end to end with an agent

1. Configure: `python3 publish.py --init` (or `cp site/CONFIG.example site/CONFIG` and replace `<...>` placeholders), put a 16+ char password in `~/.config/ai-digest/site-password` (mode 600), export `CLOUDFLARE_API_TOKEN` (scope: Account · Cloudflare Pages · Edit). Set `DIGEST_LANG` and `DIGEST_TZ`.
2. Give the agent (shell, file read/write; optional read-only X search) the prompt file `prompts/en/daily.md` (or `prompts/zh-CN/daily.md`, `weekly.md`, `papers.md`) with the repo root as working directory. The prompt's "Run contract" lists inputs, outputs and the done condition.
3. Dry run without deploying: `python3 render_html.py digest-P.md /tmp/p.html`; for the whole build use the demo command above.

## Conventions

- Python: standard library first, type hints where cheap, no new dependency without a strong reason (update `requirements*.txt`).
- User-facing text goes through `i18n.py` (`t()` / `ft()`); never hard-code English or Chinese in `render_html.py`, `publish.py` or `fetch/`. Every key must exist in both languages (`tests/test_i18n.py` enforces it).
- CLIs: `--help` with examples, non-interactive by default, errors on stderr with a non-zero exit code, `--json` where output is a summary. Keep behavior backward compatible.
- Docs: English is the default; add the idiomatic Chinese in the `.zh-CN.md` / `README.zh-CN.md` twin when you change user-facing docs. Keep READMEs lean.
- Add or update tests with every behavior change; keep `pytest -q` offline and fast.
- Changelog & releases: managed automatically by Release Please (`.github/workflows/release-please.yml`, `release-please-config.json`, `.release-please-manifest.json`) from Conventional Commit PR titles; normal PRs do not need to edit `version.py` or `CHANGELOG.md`.
- Community files: `CONTRIBUTING.md` (+ `.zh-CN.md`), `SECURITY.md`, `.github/ISSUE_TEMPLATE/` and `.github/PULL_REQUEST_TEMPLATE.md` describe the human contribution flow; keep them in sync with the rules here (required checks, sanitization).

## Safety rules

- Never commit secrets, tokens, passwords, salts, personal data, real private URLs or the reader's handles. `site/CONFIG`, password/salt files and `ledger.json` stay local. Never print a token or password in logs or output.
- Run `scripts/check_sanitized.sh` and `pytest -q` before pushing. Digest content in examples/tests must be synthetic (`example.com` links, fictional names).
- `main` is protected for everyone (including admins, linear history, squash-only): branch from `main` using `feat/`, `fix/`, `docs/`, `refactor/`, `test/`, `chore/`, or `ci/`, use a Conventional Commit PR title, wait for all required checks and thread resolution, then squash-merge. Never push directly to `main` or force-push published history; releases are automated by Release Please (merging its standing `chore(main): release X.Y.Z` PR automatically tags `vX.Y.Z`, creates the GitHub Release, and invokes `release.yml` to attach verified assets).
- `publish.py` deploys to a real site and `--rotate-password` / `--remove` change it: run them only when the task says so. The demo build (`--demo`) is always safe.
- X/Twitter access is read-only. Do not post, like, follow or message.

## Do / don't

- Do read the relevant prompt in `prompts/` before changing pipeline behavior; they are the spec for the curation step.
- Do use the helpers (`fetch/ledger.py find`, `fetch/cand.py`, `fetch/page.py`) instead of reading `ledger.json`, `candidates.jsonl` or whole pages into context.
- Do keep one language per site directory; link companion sites via `LANG_ALT` (the bilingual demo builds `/en/` and `/zh-CN/` sharing one salt).
- Don't edit generated output (`dist/`, `site/public`, `runs/`) by hand or commit it.
- Demo builds (`publish.py --demo`) show the public demo password on the unlock page (`{{demo_hint}}` in `site/password_template.html`); the live pipeline never does. Keep it that way.
- Don't change the encryption parameters or the password-page contract (`id="pw"`, `id="rm"`, `__ENCRYPT_CFG_JSON__`) without updating `verify_decrypt.mjs` and the tests.
