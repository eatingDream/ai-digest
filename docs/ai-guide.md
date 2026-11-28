# AI Digest — Agent & Operations Guide

Complete configuration reference, operational workflows, and safe optional X recall instructions for AI agents and operators running `ai-digest`.

## Pipeline Commands

Run with the compiled `ai-digest` binary (`target/release/ai-digest` or `ai-digest` on `PATH`; all subcommands support `--help`, `--version`, and `--json`):

| Step | Command | Purpose |
| --- | --- | --- |
| Initialize config | `ai-digest publish --init` (`--encrypt` for opt-in AES-256-GCM + random `SLUG`) | Creates `site/CONFIG` (or copy `site/CONFIG.example`) |
| Fetch candidates | `ai-digest fetch --daily --day YYYY-MM-DD --json` (`--weekly` for weekly) | Pulls ~155 zero-key RSS/Atom/arXiv/HF/GitHub/blog/HN/Reddit sources into `runs/<P>/fetch/` |
| Ledger dedup | `ai-digest ledger recent --json` / `ai-digest ledger find` / `ai-digest ledger add --day YYYY-MM-DD` | Tracks pushed (`★`) vs. seen (`☆`) items across issues |
| Curation helpers | `ai-digest cand`, `ai-digest check-cands`, `ai-digest page`, `ai-digest weekly-late`, `ai-digest xparse` | Inspect candidates, verify primary sources via SSRF-hardened DNS-pinned HTTP |
| Curate Markdown | Follow `prompts/<en\|zh-CN>/{papers,daily,weekly}.md` against `filter-notes*.md` | Produces 7 editorial sections (`digest-<P>.md`, optional sibling `<stem>.en.md`) |
| Render HTML | `ai-digest render digest-YYYY-MM-DD.md html/daily-YYYY-MM-DD.html [--lang zh-CN\|en] [--md-en ...]` | Embeds `web/dist/` assets, inline LaTeX math, wide elevated reader UI |
| Publish site | `ai-digest publish --issue daily/YYYY-MM-DD --md digest-YYYY-MM-DD.md [--prepare-only]` | Builds `/zh/` & `/en/` routes, static `latest`, Level 5/5 GEO + `_worker.js`, hash-locked CSP `_headers`, privacy scan, and deploys to Cloudflare Pages |
| Maintenance | `ai-digest publish --rebuild \| --rerender-all \| --rotate-password \| --remove <issue>` | Rebuilds archive, re-renders all stored issues, rotates encryption key, or removes an issue |
| Verify output | `ai-digest verify --local` (or `node web/dist/verify_decrypt.mjs --local`) | Exact-match verification in public mode; AES-256-GCM + gzip decrypt parity in encrypted mode |
| Offline demo | `ai-digest publish --demo --out dist/` | Builds synthetic bilingual demo site with zero secrets or network |

## Configuration Reference

Environment variables take precedence over `site/CONFIG`.

| Variable | `CONFIG` key | Description | Default |
| --- | --- | --- | --- |
| `SITE_DOMAIN` | `DOMAIN` | Site domain | `digest.example.com` |
| `SITE_SLUG` | `SLUG` | Optional path prefix (empty = root `/`; non-empty requires `>= 8` URL-safe chars) | empty (`--init --encrypt` generates random) |
| `DIGEST_ENCRYPT` | `ENCRYPT` | Opt-in client-side AES-256-GCM password encryption (`1`/`true`/`yes`/`on`; disables public GEO) | `0` (public) |
| `DIGEST_GEO_AI_TRAIN` | `GEO_AI_TRAIN` | Public `Content-Signal` & training-crawler policy (`yes` or `no`; `ENCRYPT=0` only) | `yes` |
| `DIGEST_GEO_WORKER` | `GEO_WORKER` | Emit Pages `_worker.js` for `Accept: text/markdown` & `/.well-known/search` (`1`/`0`; `ENCRYPT=0` only) | `1` (on) |
| `DIGEST_INDEXNOW_KEY` | `INDEXNOW_KEY` | Optional IndexNow key (`^[A-Za-z0-9-]{8,128}$`) emitting `/<key>.txt` and pushing canonical URLs post-deploy (`ENCRYPT=0` only; set `DOMAIN` to the public hostname serving `/<key>.txt`) | empty (disabled) |
| `DIGEST_INDEXNOW_ENDPOINT` | — | Optional IndexNow API endpoint override | `https://api.indexnow.org/indexnow` |
| `CF_ACCOUNT_ID` | `ACCOUNT_ID` | Cloudflare account ID | — |
| `CF_ZONE_ID` | `ZONE_ID` | Zone of the domain (optional) | — |
| `CF_PAGES_PROJECT` | `PROJECT` | Cloudflare Pages project name | `ai-digest` |
| `CLOUDFLARE_API_TOKEN` / `_FILE` | — | Deploy token (scope: Account · Cloudflare Pages · Edit) | — |
| `DIGEST_SITE_PASSWORD_FILE` | — | Site password file (mode `0600`; used only when `ENCRYPT=1`) | `~/.config/ai-digest/site-password` |
| `DIGEST_SITE_SALT_FILE` | — | PBKDF2 salt file (auto-generated when `ENCRYPT=1`) | `~/.config/ai-digest/site-salt` |
| `DIGEST_LANG` | `LANG` | Digest & site UI language: `en` or `zh-CN` (`--lang` overrides) | `en` |
| `DIGEST_LANG_ALT` | `LANG_ALT` | Optional companion site URL/path in the other language (e.g. `zh-CN:/zh-CN/`) | — |
| `DIGEST_ROOT_LATEST` | `ROOT_LATEST` | When `SLUG` is set, opt-in root `/latest`, `/daily/latest`, `/weekly/latest` static pages (`1`/`0`) | `0` (off) |
| `DIGEST_TZ` | — | IANA time zone defining a natural calendar day | `UTC` |
| `DIGEST_ROOT` | — | Working directory for `runs/`, `html/`, `site/`, `ledger.json`, `logs/`, `state/` | repo root |
| `LEDGER_PATH` | — | Explicit path to `ledger.json` (auto-created on first `ai-digest ledger add`) | `<DIGEST_ROOT>/ledger.json` |
| `DIGEST_PREFERENCES_FILE` | — | Topic ranking weights JSON (see `fetch/preferences.example.json`) | `fetch/preferences.json` |
| `DIGEST_PRIVACY_PATTERNS_FILE` | — | Extra regexes checked before publishing | `site/privacy-patterns.txt` |
| `DIGEST_DEMO_PASSWORD` | — | Optional password for `ai-digest publish --demo` (unset = public demo) | — |
| `GITHUB_TOKEN` / `GH_TOKEN` | — | Optional, raises GitHub API limits during fetch | — |

Sources live in [fetch/sources.toml](../fetch/sources.toml); editorial selection rules live in [filter-notes.md](../filter-notes.md) / [filter-notes.zh-CN.md](../filter-notes.zh-CN.md).

## Safe Optional X Recall (No Grok Bot Required)

X/Twitter recall is **100% optional** and never requires a Grok Bot or personal X cookies. Full templates live in [fetch/x-recall.md](../fetch/x-recall.md) ([中文](../fetch/x-recall.zh-CN.md)):

- **Free & zero-risk paths**:
  - **Open-feeds-only mode (default)** — prompts skip X cleanly and curate from ~155 open RSS/Atom/arXiv/HF/GitHub/lab-blog/HN/Reddit feeds.
  - **Read-only web search grounding** — query `site:x.com/<handle>` over your date window using search grounding (Gemini, Brave, Tavily, Exa, Perplexity) without logging into X.
  - **Official cross-posted RSS & self-hosted bridges** — subscribe to Bluesky (`https://bsky.app/profile/<handle>/rss`), Mastodon, Substack, or self-hosted RSSHub public read-only routes in `fetch/sources.toml`.
- **Paid & official API paths**:
  - **xAI Grok API (`x_search`)** *(recommended paid)* — pay-as-you-go official API (`api.x.ai`) with handle & date filters and zero X account risk.
  - **Official X API v2** — read-only Bearer Token (`GET /2/tweets/search/recent`), feeding JSON directly into `ai-digest xparse`.
  - **Third-party read-only search/X APIs & MCPs** — Exa, Tavily, Perplexity Sonar, TwitterAPI.io, or Apify.
- **Red lines**: never pass personal `auth_token`/`ct0` cookies to scrapers, keep automation strictly read-only, store API keys outside the repo (`0600`), and never leak personal watch lists into published issues (unreplaced `<account_1>` / `<zh_account_1>` placeholders are skipped automatically).

## Consuming a Published Site (GEO / Agent Endpoints)

Every public (`ENCRYPT=0`) site exposes:
- `GET /llms.txt` and `GET /llms-full.txt`
- `GET /.well-known/search?q=<query>`, `HEAD /.well-known/search`, and RFC 10008 `QUERY /.well-known/search` (`text/plain` or `{"q":"...","lang":"en","limit":10}`)
- `Accept: text/markdown` content negotiation on every page, plus direct `.md` mirrors (e.g. `/zh/daily/latest.md`, `/en/daily/latest.md`)
- `GET /.well-known/agent-skills/index.json`, `GET /sitemap.xml`, and `GET /feed.xml`
