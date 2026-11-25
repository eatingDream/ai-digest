# AGENTS.md

Guidance for AI coding agents (Cursor, Claude Code, Codex, …) working in this repository. Humans: see [README.md](README.md).

## What this is

`ai-digest` turns ~155 open sources into daily and weekly AI digests: a script fetches and triages candidates, an LLM agent curates Markdown, `render_html.py` renders single-file HTML, and `publish.py` encrypts every page client-side and deploys to Cloudflare Pages (`DIGEST_LANG` / `--lang`: `en` default, `zh-CN`). Python 3.11+ (`requests`, `lxml`, `markdown`, `cryptography`); Node is only used for `verify_decrypt.mjs` and `npx wrangler`.

## Setup, test, lint (run before every PR)

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt
python -m py_compile *.py fetch/*.py tests/*.py      # CI "compile" step
pytest -q                                            # all tests, offline, ~5 s
node --check verify_decrypt.mjs
scripts/check_sanitized.sh                           # forbidden-pattern guard (must print "clean")
```

CI (`.github/workflows/ci.yml`) also runs gitleaks over full history. Required checks on `main`: `Python (compile + pytest)`, `Node (syntax check)`, `Secret scan (gitleaks) + sanitization guard`. Local demo build (no secrets/network): `DIGEST_DEMO_PASSWORD=demo python3 publish.py --demo --out dist/` → `/en/` and `/zh-CN/` (`--lang en` for single `/demo/`).

## Repo map

| Path | Purpose |
|---|---|
| `fetch/fetch.py` | Recall: parallel fetch of `fetch/sources.toml` → `runs/<P>/fetch/{candidates.jsonl,triage.md,triage-papers.md,report.json,report.md}` |
| `fetch/cand.py`, `page.py`, `xparse.py`, `weekly_late.py`, `check_cands.py`, `ledger.py` | Curation helpers (candidate lookup, slim page fetch, X compaction, weekly "late signal" list, papers validation, ledger queries/writes) |
| `fetch/x-recall.md` | Optional read-only X/Twitter search protocol (`.zh-CN.md` twin) |
| `prompts/<en\|zh-CN>/{papers,daily,weekly}.md` | Self-contained agent prompts; each starts with a "Run contract" table |
| `filter-notes.md`, `x-sources.md` | Selection rules and reader-preference profile template (`.zh-CN.md` twins) |
| `render_html.py` | Markdown → single-file HTML (embedded CSS, full-title sidebar TOC, optional `--md-en` single-page bilingual toggle; parses EN & ZH labels) |
| `publish.py` | Dual-pane localized nav, index, static encrypted `latest` pages (`/<slug>/{daily,weekly,}/latest` + opt-in `ROOT_LATEST`, HTTP 200), privacy scan, AES-256-GCM encryption, deploy, `--demo`, `--md-en` / auto-discovered `*.en.md` |
| `i18n.py` | All UI strings, date formats, fetch labels, and bilingual Markdown vocabulary (`en`, `zh-CN`) |
| `site/` | Password page template, `CONFIG.example`, site docs (`site/CONFIG` is local and gitignored) |
| `examples/` | Synthetic sample issues (`sample-{digest,weekly}.{en,zh-CN}.md`) for tests, demo, and docs |
| `tests/` | Offline pytest suite |
| `version.py` | `__version__` (also `--version` on `publish.py`, `render_html.py`, `fetch/fetch.py`) |

Gitignored working data: `runs/`, `logs/`, `html/`, `ledger.json`, `digest-*.md`, `weekly-*.md`, `site/plain|sources|public`, `dist/`.

## How the pipeline runs

Times are local (`DIGEST_TZ`, default UTC); natural-day windows are half-open `[P 00:00, P+1 00:00)`. Run prompts from the repo root with shell + file tools (optional read-only X search); dry-run rendering via `python3 render_html.py digest-P.md /tmp/p.html`.

| Step | When | Command / prompt | Output |
|---|---|---|---|
| 1. Papers prefilter | 00:10 | `prompts/<lang>/papers.md` (runs `fetch/fetch.py --daily --reuse`) | `runs/P/fetch/`, `runs/P/paper-cands.md` |
| 2. Daily curation | 00:30 | `prompts/<lang>/daily.md` | `digest-P.md`, `runs/P/filter-log.md`, ledger updates |
| 3. Publish | end of step 2 | `python3 publish.py --issue daily/P --md digest-P.md` | encrypted page `daily/P`, rebuilt index/nav, deploy |
| 4. Weekly | Monday 01:30 | `prompts/<lang>/weekly.md` (uses `fetch/weekly_late.py`) | `weekly-L.md` (`L` = `YYYY-MM-DD_MM-DD`, Monday_Sunday), published `weekly/L` |

File names: daily `digest-YYYY-MM-DD.md`, weekly `weekly-YYYY-MM-DD_MM-DD.md`; issue ids `daily/YYYY-MM-DD`, `weekly/YYYY-MM-DD_MM-DD`.

Markdown contract (parsed by `render_html.py` in either language): title `**AI Daily · M/D**` or `**AI Weekly · MM/DD–MM/DD**` (zh: `**AI精选日报 · M/D**`); seven `##` sections (Models, Papers, Tools & Projects, Industry & Products, Compute & Infrastructure, Safety & Policy, Opinions & Essays); numbered `###` items with a metadata line and fields In brief / Why it matters / Details (/ Follow-up in weeklies). Follow `examples/sample-digest.en.md`.

## Conventions

- Python: stdlib first, light type hints, minimize new deps (update `requirements*.txt`), add offline tests for behavior changes.
- i18n & docs: all user-facing strings go through `i18n.py` (`t()` / `ft()`) in both `en` and `zh-CN` (`tests/test_i18n.py` enforces parity); keep `.zh-CN.md` doc twins idiomatic and READMEs lean.
- CLIs: `--help` with examples, non-interactive by default, stderr + non-zero exit on errors, `--json` for summaries, backward compatible.
- Releases & community files: Release Please (`.github/workflows/release-please.yml`) manages `version.py`, `CHANGELOG.md`, and tagged releases from Conventional Commit PR titles — don't edit them in normal PRs. Keep `CONTRIBUTING*.md` and `.github/` templates in sync with rules here.

## Safety rules & do / don't

- Never commit secrets, tokens, passwords, salts, personal data, real private URLs, or handles (`site/CONFIG`, password/salt files, and `ledger.json` stay local; never log tokens/passwords). Examples and tests use synthetic data (`example.com`, fictional names).
- `main` is protected (`enforce_admins`, squash-only linear history): branch with `feat/`, `fix/`, `docs/`, `refactor/`, `test/`, `chore/`, or `ci/`, use a Conventional Commit PR title, wait for required checks, then squash-merge; never push to `main` or force-push published history.
- Run `publish.py` live deploys, `--rotate-password`, or `--remove` only when explicitly requested (`--demo` is always safe). X/Twitter access is strictly read-only.
- Read the relevant `prompts/` spec before changing pipeline behavior; use helpers (`ledger.py`, `cand.py`, `page.py`) instead of loading `ledger.json`, `candidates.jsonl`, or full web pages into context.
- Keep one primary language per site directory (`LANG_ALT` for companion sites; `--md-en` / sibling `<stem>.en.md` for single-page bilingual issues); never hand-edit or commit `dist/`, `site/public`, or `runs/`.
- Keep `{{demo_hint}}` in `site/password_template.html` demo-only, and never alter encryption parameters or password-page DOM contracts (`id="pw"`, `id="rm"`, `__ENCRYPT_CFG_JSON__`) without updating `verify_decrypt.mjs` and tests.
