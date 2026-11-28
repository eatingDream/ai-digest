# Contributing

Thanks for helping out. [简体中文](CONTRIBUTING.zh-CN.md)

## Workflow & rules

- **PRs only**: `main` is protected for everyone (`enforce_admins`, linear squash-only history). Branch from latest `main` (`feat/`, `fix/`, `docs/`, `refactor/`, `test/`, `chore/`, `ci/`), keep one focused change per PR, and never push directly to `main` or force-push published history.
- **Conventional Commits & releases**: use [Conventional Commits](https://www.conventionalcommits.org/) for PR titles (e.g. `fix(fetch): ...`, `feat(render): ...`, `docs: ...`). [Release Please](https://github.com/googleapis/release-please) automatically bumps `Cargo.toml`, `Cargo.lock`, `web/package.json`, `web/package-lock.json`, and [CHANGELOG.md](CHANGELOG.md), and cuts tagged GitHub Releases (`release.yml`) from merged PR titles — do not edit version or changelog files in normal PRs.
- **Public repo hygiene**: never commit secrets, tokens, passwords, salts, `site/CONFIG`, real URLs/emails, machine paths, or personal handles. Examples and tests must use synthetic data only (`example.com`, fictional names).
- **Required checks** (branch up to date with `main`): `Frontend (TS + CSS + Vitest)`, `Rust (fmt + clippy + test)`, `E2E parity & release binary hygiene`, and `Secret scan (gitleaks) + sanitization guard`. Merged branches are deleted automatically.
- **License**: contributions are released under the [MIT License](LICENSE).

## Local checks

```bash
# Frontend (Node 24+)
npm --prefix web ci
npm --prefix web run typecheck
npm --prefix web run build
git diff --exit-code web/dist/
npm --prefix web test

# Rust (1.88+)
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo fmt --check
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo clippy --all-targets -- -D warnings
AI_DIGEST_REQUIRE_WEB_DIST=1 cargo test --locked

# Sanitization guard (must print "clean")
scripts/check_sanitized.sh
```

Preview the demo site without secrets: `cargo run -- publish --demo --out dist/` (set `DIGEST_DEMO_PASSWORD=demo` to preview encrypted mode).

## PR expectations

- Add or update fast offline Rust (`tests/*.rs`) or Vitest (`web/tests/*.test.ts`) tests for behavior changes; keep new dependencies minimal and pure-Rust where possible.
- Rebuild and commit `web/dist/` whenever `web/src/**` changes (CI verifies clean rebuild parity).
- Put all user-facing strings in `src/i18n.rs` for both `en` and `zh-CN`.
- English is the default doc language; when changing user-facing docs, update the `.zh-CN.md` twin with idiomatic Chinese.

## Issues, security & agents

Use the [issue templates](https://github.com/eatingDream/ai-digest/issues/new/choose); report vulnerabilities privately per [SECURITY.md](SECURITY.md). AI coding agents: read [AGENTS.md](AGENTS.md) first.
