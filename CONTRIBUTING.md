# Contributing

Thanks for helping out. [简体中文](CONTRIBUTING.zh-CN.md)

## Ground rules

- **Everything goes through a pull request.** `main` is protected for everyone (including maintainers, `enforce_admins` enabled): open a PR from a short-lived branch or fork, wait for all required checks to pass, resolve review threads, then squash-merge. Never push to `main` directly or force-push published history.
- **Required checks** (all must pass, branch up to date with `main`): `Python (compile + pytest)`, `Node (syntax check)`, `Secret scan (gitleaks) + sanitization guard`.
- **This is a public repository — never commit secrets or personal data**: tokens, passwords, salts, `site/CONFIG`, real site URLs, real emails, machine paths, or anyone's handles. Examples and tests use synthetic data only (`example.com`, fictional names). GitHub push protection, gitleaks, and `scripts/check_sanitized.sh` enforce this.
- The project is licensed under the [MIT License](LICENSE). By contributing you agree your work is released under the same license.

## Development workflow

We follow a single-trunk GitHub Flow with linear history on `main`:

1. **Scope first**: open or reference an issue for non-trivial changes. Keep each PR focused on one change.
2. **Branch from latest `main`**: `git checkout main && git pull --ff-only && git checkout -b <type>/<short-slug>` using a standard prefix: `feat/`, `fix/`, `docs/`, `refactor/`, `test/`, `chore/`, or `ci/`.
3. **Commit and PR title convention**: use [Conventional Commits](https://www.conventionalcommits.org/) for PR titles (and commits), e.g. `fix(fetch): ...`, `feat(render): ...`, `docs: ...`. Because `main` uses squash-only merge with the PR title, each merged PR becomes one clean commit in history.
4. **Verify locally, then open a PR**: run the checks below before pushing, fill in the PR template, and link any related issue (`Closes #N`). Wait for CI green and review before squash-merging; merged branches are deleted automatically.

## Releases

Releases are automated with [Release Please](https://github.com/googleapis/release-please) (`.github/workflows/release-please.yml`):

1. Merge normal PRs to `main` using Conventional Commit titles (`fix:` / `perf:` / `refactor:` → patch, `feat:` → minor, `!:` or `BREAKING CHANGE:` → minor pre-1.0 / major ≥1.0). Contributors do **not** need to edit `version.py` or `CHANGELOG.md` manually.
2. On every push to `main`, Release Please automatically opens or updates a standing Release PR (`chore(main): release X.Y.Z`) that bumps [version.py](version.py), [.release-please-manifest.json](.release-please-manifest.json), and [CHANGELOG.md](CHANGELOG.md).
3. When ready to ship, ensure CI is green on the Release PR and squash-merge it. Release Please automatically creates the `vX.Y.Z` tag and GitHub Release, then invokes `.github/workflows/release.yml` to run full checks and attach the source tarball, demo zip, and `SHA256SUMS.txt`.
4. Pre-releases (e.g. `v0.2.0-rc.1`) can still be tagged manually on `main` to trigger `.github/workflows/release.yml` directly.

## Set up and test

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt
python -m py_compile *.py fetch/*.py tests/*.py
pytest -q                          # offline, a few seconds
node --check verify_decrypt.mjs
scripts/check_sanitized.sh         # must print "clean"
```

Preview the demo site without any secrets: `DIGEST_DEMO_PASSWORD=demo python3 publish.py --demo --out dist/`.

## Pull request checklist

- One focused change per PR, with a Conventional Commit title and a short description of what and why (Release Please generates [CHANGELOG.md](CHANGELOG.md) directly from PR titles).
- Add or update tests for behavior changes; keep `pytest -q` offline and fast.
- User-facing text goes through `i18n.py` and must exist in both `en` and `zh-CN`.
- Docs: English is the default. If you change a user-facing doc, update its `.zh-CN.md` twin with idiomatic Chinese (not a literal translation).
- Keep new dependencies to a minimum and update `requirements*.txt` when you add one.

## Reporting bugs and ideas

Use the [issue templates](https://github.com/eatingDream/ai-digest/issues/new/choose). For security problems, see [SECURITY.md](SECURITY.md) and do not open a public issue.

AI coding agents: read [AGENTS.md](AGENTS.md) first.
