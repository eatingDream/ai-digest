# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- Restored Python 3.11 compatibility by removing Python 3.12-only backslashes inside f-string expressions in `render_html.py` and `fetch/page.py`, and added a Python 3.11 CI check.
- Fixed `NameError` (`L` -> `tr`) in `fetch/fetch.py` (`paper_signals`) when an off-topic paper matches a major-result keyword.
- Replaced hardcoded year `2026` in `render_html.py` with dynamic year inference (from `--year`, heading, filename, or current year) and fixed cross-year weekly date ranges in both English and Chinese.

## [0.1.0] - 2026-11-20

Initial release.

### Added
- Recall layer (`fetch/`): parallel fetch of ~150 sources, de-duplication, tagging, compact triage lists, a ledger of what was already pushed or seen, and helpers for the curation agent.
- Agent prompts for the papers prefilter, the daily and the weekly issue, each with a "Run contract" (inputs, outputs, done condition).
- `render_html.py`: Markdown to a self-contained HTML page with sidebar TOC, light/dark themes and a wide-screen layout.
- `publish.py`: navigation and index pages, privacy scan, client-side AES-256-GCM encryption (PBKDF2-SHA256, 600k iterations), incremental deploy to Cloudflare Pages, decrypt verification, password rotation.
- English and Simplified Chinese everywhere (`DIGEST_LANG=en|zh-CN`, `--lang`, `LANG` in `site/CONFIG`): prompts, fetch labels, page UI, dates, samples, docs and READMEs. English is the default.
- Bilingual demo site: `publish.py --demo` builds English (`/en/`) and Chinese (`/zh-CN/`) into one deploy, sharing a salt, with a language switch on the password page, the index and every issue. Demo builds show the public demo password on the unlock page (live pipeline unchanged).
- Agent-friendly tooling: `AGENTS.md` (and `CLAUDE.md`), `--help` with examples on every CLI, `--json` output on `render_html.py`, `fetch/fetch.py`, `fetch/ledger.py` and `fetch/check_cands.py`, `--version` on every CLI, clear exit codes.
- Community files: `CONTRIBUTING.md` (and a Chinese twin), `SECURITY.md` (private reporting via GitHub security advisories), issue templates and a pull request template.
- CI (tests, secret scan, sanitization guard), tagged releases and a demo-site workflow with optional Cloudflare Pages deploy.
