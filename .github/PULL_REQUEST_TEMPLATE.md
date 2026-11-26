## What and why

<!-- One focused change. Use a Conventional Commit PR title (e.g. `fix(fetch): ...`). Link the issue if there is one (`Closes #N`). -->

## Verification

<!-- How you tested this locally (e.g. `cargo test --locked`, `npm --prefix web test`, `--demo` build, edge cases checked). -->

## Checklist

- [ ] `npm --prefix web run typecheck && npm --prefix web test`, `AI_DIGEST_REQUIRE_WEB_DIST=1 cargo fmt --check`, `AI_DIGEST_REQUIRE_WEB_DIST=1 cargo clippy --all-targets -- -D warnings`, `AI_DIGEST_REQUIRE_WEB_DIST=1 cargo test --locked`, and `scripts/check_sanitized.sh` pass locally
- [ ] Rebuilt and committed `web/dist/` if `web/src/**` changed
- [ ] Tests added or updated for behavior changes
- [ ] No secrets, personal data, real URLs or local paths (public repo; examples and tests are synthetic)
- [ ] User-facing text is in `src/i18n.rs` for both `en` and `zh-CN`; docs updated with the `.zh-CN.md` twin
- [ ] PR title follows Conventional Commits (used by Release Please to update `CHANGELOG.md` and bump versions automatically)
