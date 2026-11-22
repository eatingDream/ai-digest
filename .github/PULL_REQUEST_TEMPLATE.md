## What and why

<!-- One focused change. Use a Conventional Commit PR title (e.g. `fix(fetch): ...`). Link the issue if there is one (`Closes #N`). -->

## Verification

<!-- How you tested this locally (e.g. `pytest -q`, `--demo` build, edge cases checked). -->

## Checklist

- [ ] `python -m py_compile *.py fetch/*.py tests/*.py`, `pytest -q`, `node --check verify_decrypt.mjs`, and `scripts/check_sanitized.sh` pass locally
- [ ] Tests added or updated for behavior changes
- [ ] No secrets, personal data, real URLs or local paths (public repo; examples and tests are synthetic)
- [ ] User-facing text is in `i18n.py` for both `en` and `zh-CN`; docs updated with the `.zh-CN.md` twin
- [ ] PR title follows Conventional Commits (used by Release Please to update `CHANGELOG.md` and bump versions automatically)
