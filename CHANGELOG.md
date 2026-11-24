# Changelog

All notable changes to this project are documented here.
The project uses [Semantic Versioning](https://semver.org/) and updates this file automatically via [Release Please](https://github.com/googleapis/release-please) from Conventional Commits.

## [0.3.0](https://github.com/eatingDream/ai-digest/compare/v0.2.0...v0.3.0) (2026-11-24)


### Features

* add single-page bilingual rendering and full sidebar TOC titles ([#14](https://github.com/eatingDream/ai-digest/issues/14)) ([3ac3e09](https://github.com/eatingDream/ai-digest/commit/3ac3e095b758d9dc85ff5f3249677529145ab2b3))

## [0.2.0](https://github.com/eatingDream/ai-digest/compare/v0.1.1...v0.2.0) (2026-11-24)


### Features

* harden first-run config/ledger flows, reader UX, and bilingual site support ([#13](https://github.com/eatingDream/ai-digest/issues/13)) ([35f3951](https://github.com/eatingDream/ai-digest/commit/35f39518b9647b649c565b508892ad14c06bb7a8))


### Bug Fixes

* **security:** harden HTML/URL sanitization, CSP headers, SSRF guard, and CI pinning ([#11](https://github.com/eatingDream/ai-digest/issues/11)) ([c15f559](https://github.com/eatingDream/ai-digest/commit/c15f55924c3bf88c49dcf577358ce9d65000c7d7))

## [0.1.1](https://github.com/eatingDream/ai-digest/compare/v0.1.0...v0.1.1) (2026-11-22)


### Bug Fixes

* **fetch:** harden helper CLIs (atomic ledger, weekly_late --chars, in-process page.py, i18n) ([#7](https://github.com/eatingDream/ai-digest/issues/7)) ([b33144e](https://github.com/eatingDream/ai-digest/commit/b33144ee013155a3c38748bd4b18b3eb4b30a245))
* **fetch:** replace undefined L() with tr() in paper_signals ([#3](https://github.com/eatingDream/ai-digest/issues/3)) ([16cc1e3](https://github.com/eatingDream/ai-digest/commit/16cc1e3e56fd047fde1dd6e856043d8aabfc9b2b))
* **render:** infer digest year dynamically, handle cross-year weeklies, drop dead code ([#5](https://github.com/eatingDream/ai-digest/issues/5)) ([2bff7d0](https://github.com/eatingDream/ai-digest/commit/2bff7d084e4a88ccd95708d5f3d5005d695f7803))
* restore Python 3.11 compatibility and add 3.11 CI check ([#2](https://github.com/eatingDream/ai-digest/issues/2)) ([12383ba](https://github.com/eatingDream/ai-digest/commit/12383ba6efa23ef390b64b044ae2b5e9b9b38a31))


### Performance Improvements

* **fetch:** thread-safe sessions and state, UTF-8 I/O, reuse --json, and source dedup ([#6](https://github.com/eatingDream/ai-digest/issues/6)) ([00782d2](https://github.com/eatingDream/ai-digest/commit/00782d29f42580d8671b3fd53a146f2f1bbd8c54))


### Code Refactoring

* **i18n:** fix cross-year zh-CN date range, add fetch keys, drop dead regex ([#4](https://github.com/eatingDream/ai-digest/issues/4)) ([158d619](https://github.com/eatingDream/ai-digest/commit/158d6192c8e30d94b1e55cc4f67fc08795328d29))
* **publish:** harden encryption cache, timezone, state restoration, dry-run --issue, and verify_decrypt ([#8](https://github.com/eatingDream/ai-digest/issues/8)) ([abe47aa](https://github.com/eatingDream/ai-digest/commit/abe47aa5d2d4dd86b1284887e48bb9ead7472578))


### Documentation

* formalize PR-based development and release workflow ([#1](https://github.com/eatingDream/ai-digest/issues/1)) ([f248423](https://github.com/eatingDream/ai-digest/commit/f2484237c546581825d5c419b70eda9aacc34af9))

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
