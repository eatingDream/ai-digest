# AGENTS.md

Guidance for AI coding agents (Cursor, Claude Code, Codex, Antigravity, …) working in this repository. Humans: see [README.md](README.md).

## What this is

`ai-digest` is a self-contained **Rust binary (`ai-digest`) + TypeScript/CSS editorial frontend (`web/`)** that turns ~155 open sources into daily and weekly AI digests: `ai-digest fetch` collects and triages candidates, an LLM agent curates Markdown, `ai-digest render` renders single-file HTML with embedded frontend assets (`web/dist/`), and `ai-digest publish` builds and deploys a root-mounted public static site by default (`ENCRYPT=0`, `SLUG=`, symmetric `/zh/` & `/en/` paths, Level 5/5 GEO + `_worker.js`; optional client-side AES-256-GCM encryption via `--init --encrypt` or `ENCRYPT=1`) to Cloudflare Pages (`DIGEST_LANG` / `--lang`: `en` default, `zh-CN`). Rust 1.88+ builds the binary with zero runtime interpreter dependencies; Node 24+ is used only for building/testing `web/`, running `web/dist/verify_decrypt.mjs`, or optional `npx wrangler` deploys.

## Setup, test, lint (run before every PR)

```bash
# Frontend (Node 24+)
npm --prefix web ci
npm --prefix web run typecheck
npm --prefix web run build && git diff --exit-code web/dist/
npm --prefix web test

# Rust (1.88+)
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo fmt --check
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo clippy --all-targets -- -D warnings
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo test --locked

# Release hygiene & sanitization guard
RUSTFLAGS="--remap-path-prefix=$HOME=/build --remap-path-prefix=$PWD=/src" \
  AI_DIGEST_REQUIRE_WEB_DIST=1 cargo build --release --locked
scripts/check_sanitized.sh                           # must print "clean"
```

CI (`.github/workflows/ci.yml`) also checks `strings target/release/ai-digest` for zero local paths, verifies Rust+Node E2E parity on public and encrypted sites, and runs gitleaks over full history. Required checks on `main`: `Frontend (TS + CSS + Vitest)`, `Rust (fmt + clippy + test)`, `E2E parity & release binary hygiene`, `Secret scan (gitleaks) + sanitization guard`. Local demo build (no secrets/network): `./target/release/ai-digest publish --demo --out dist/` (set `DIGEST_DEMO_PASSWORD=demo` for encrypted mode).

## Repo map

| Path | Purpose |
|---|---|
| `Cargo.toml`, `build.rs`, `src/` | Single `ai-digest` Rust crate (strict lower-triangular module DAG; embeds `web/dist/*` at compile time) |
| `src/fetch/` | Recall (`fetch`) & curation helpers (`cand`, `check-cands`, `page`, `weekly-late`, `xparse`), plus `src/ledger.rs` |
| `src/render.rs`, `src/math.rs`, `src/parser.rs` | Markdown → single-file HTML (Opus editorial UI, build-time LaTeX inline math, `--md-en` bilingual toggle) & dual-era HTML5 parser |
| `src/geo.rs` | Public Level 5/5 GEO & Agent Readiness (`ENCRYPT=0` only): `robots.txt` Content Signals, `sitemap.xml`, `llms.txt`/`llms-full.txt`, Atom feeds, JSON-LD, `.md` mirrors, `/.well-known/agent-skills/`, `search-index.json`, and `_worker.js` config injection |
| `src/crypto.rs`, `src/net.rs`, `src/verify.rs` | Privacy scan, build-time SHA-256 CSP header finalizer, opt-in AES-256-GCM+gzip encryption (`ENCRYPT=1`), SSRF-hardened HTTP client, native verifier |
| `src/publish.rs`, `src/i18n.rs`, `src/model.rs`, `src/assets.rs`, `src/version.rs` | Publish orchestrator (`--init`, `--issue`, `--remove`, `--rebuild`, `--rerender-all`, `--rotate-password`, `--demo`), bilingual i18n, shared models |
| `web/ui-theme.json`, `web/src/{css,ts}/`, `web/build.ts`, `web/dist/`, `web/tests/` | Declarative Lucid Indigo palette + motion config, strict TypeScript + CSS frontend, deterministic `esbuild` pipeline (committed `web/dist/*` + `verify_decrypt.mjs`), Vitest suite (budgets, CSS hygiene, Worker, WebCrypto parity) |
| `fetch/` | Source registry (`sources.toml`), `preferences.example.json`, optional X recall protocol (`x-recall*.md`), arXiv troubleshooting |
| `prompts/<en\|zh-CN>/{papers,daily,weekly}.md` | Self-contained agent prompts; each starts with a "Run contract" table |
| `filter-notes.md`, `x-sources.md` | Selection rules and reader-preference profile template (`.zh-CN.md` twins) |
| `skills/ai-digest/SKILL.md`, `llms.txt` | Portable Agent Skill (`agentskills.io`) and repo-level `llms.txt` directory for AI agents |
| `site/` | `CONFIG.example`, static `public/{404.html,robots.txt,_headers}`, site docs (`site/CONFIG` is local and gitignored) |
| `examples/` | Synthetic sample issues (`sample-{digest,weekly}.{en,zh-CN}.md`) for tests, demo, and docs |
| `tests/` | Offline Rust integration/contract test suite (`core_tests.rs`, `cli_tests.rs`, `fetch_tests.rs`, `publish_geo_tests.rs`) |
| `publish.sh` | Thin POSIX wrapper resolving `ai-digest` binary and forwarding to `ai-digest publish "$@"` |

Gitignored working data: `runs/`, `logs/`, `html/`, `ledger.json`, `digest-*.md`, `weekly-*.md`, `site/plain|sources|public/*`, `/dist/`, `/target`, `web/node_modules`. (`web/dist/` IS committed so Rust builds without Node.)

## How the pipeline runs

Times are local (`DIGEST_TZ`, default UTC); natural-day windows are half-open `[P 00:00, P+1 00:00)`. Run prompts from the repo root with shell + file tools (optional read-only X search); dry-run rendering via `ai-digest render digest-P.md /tmp/p.html`.

| Step | When | Command / prompt | Output |
|---|---|---|---|
| 1. Papers prefilter | 00:10 | `prompts/<lang>/papers.md` (runs `ai-digest fetch --daily --reuse`) | `runs/P/fetch/`, `runs/P/paper-cands.md` |
| 2. Daily curation | 00:30 | `prompts/<lang>/daily.md` | `digest-P.md`, `runs/P/filter-log.md`, ledger updates |
| 3. Publish | end of step 2 | `ai-digest publish --issue daily/P --md digest-P.md` | published `daily/P` (plus `/zh/` & `/en/` when `*.en.md` exists), rebuilt index/nav, deploy |
| 4. Weekly | Monday 01:30 | `prompts/<lang>/weekly.md` (uses `ai-digest weekly-late`) | `weekly-L.md` (`L` = `YYYY-MM-DD_MM-DD`, Monday_Sunday), published `weekly/L` |

File names: daily `digest-YYYY-MM-DD.md`, weekly `weekly-YYYY-MM-DD_MM-DD.md`; issue ids `daily/YYYY-MM-DD`, `weekly/YYYY-MM-DD_MM-DD`.

Markdown contract (parsed by `src/parser.rs` in either language): title `**AI Digest · Daily · M/D**` or `**AI Digest · Weekly · MM/DD–MM/DD**` (zh: `**AI Digest · 日报 · M/D**`); seven `##` sections (Models, Papers, Tools & Projects, Industry & Products, Compute & Infrastructure, Safety & Policy, Opinions & Essays); numbered `###` items with a metadata line and fields In brief / Why it matters / Details (/ Follow-up in weeklies). Follow `examples/sample-digest.en.md`.

## Conventions

- Rust & TypeScript: keep pure-Rust dependencies lean, respect the module DAG enforced by `tests/core_tests.rs`, keep `web/src/` strict TS with CSS/JS budgets enforced by `web/build.ts`, and always commit rebuilt `web/dist/` when changing `web/src/`.
- i18n & docs: all user-facing strings go through `src/i18n.rs` in both `en` and `zh-CN` (`tests/core_tests.rs` enforces parity); keep `.zh-CN.md` doc twins idiomatic and READMEs lean.
- CLIs: pure English `--help` with examples, `--version` printing `ai-digest <ver>`, non-interactive by default, stderr + exit `1`/`2` on errors, `--json` for summaries.
- Releases & community files: Release Please (`.github/workflows/release-please.yml`) manages `Cargo.toml`, `Cargo.lock`, `web/package.json`, `web/package-lock.json`, `CHANGELOG.md`, and tagged releases from Conventional Commit PR titles — don't edit them in normal PRs. Keep `CONTRIBUTING*.md` and `.github/` templates in sync with rules here.

## Safety rules & do / don't

- Never commit secrets, tokens, passwords, salts, personal data, real private URLs, or handles (`site/CONFIG`, password/salt files, and `ledger.json` stay local; never log tokens/passwords). Examples and tests use synthetic data (`example.com`, fictional names).
- `main` is protected (`enforce_admins`, squash-only linear history): branch with `feat/`, `fix/`, `docs/`, `refactor/`, `test/`, `chore/`, or `ci/`, use a Conventional Commit PR title, wait for required checks, then squash-merge; never push to `main` or force-push published history.
- Run live `ai-digest publish` deploys, `--rotate-password`, or `--remove` only when explicitly requested (`--demo` and `--prepare-only` are always safe). X/Twitter access is strictly read-only.
- Read the relevant `prompts/` spec before changing pipeline behavior; use helpers (`ai-digest ledger`, `cand`, `page`) instead of loading `ledger.json`, `candidates.jsonl`, or full web pages into context.
- Keep separate single-lang pages under `/zh/` and `/en/` when `LANG=zh-CN` has companion `*.en.md` files (with `LANG_ALT` for external companion sites and `ai-digest render --md-en` for standalone single-file bilingual output); never hand-edit or commit `/dist/`, `site/public/*`, or `runs/`.
- Never alter encryption parameters (`PBKDF2-SHA256` 600,000 iters, `AES-256-GCM`, gzip `v:2`) or unlock DOM contracts (`id="pw"`, `id="rm"`, `id="c"`) without updating both `src/crypto.rs`/`src/verify.rs` and `web/src/ts/{unlock,verify_decrypt}.ts` + tests.
