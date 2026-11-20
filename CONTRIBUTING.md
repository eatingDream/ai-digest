# Contributing

Thanks for helping out. [简体中文](CONTRIBUTING.zh-CN.md)

## Ground rules

- **Everything goes through a pull request.** `main` is protected: open a PR from a branch or a fork, wait for the required checks, then it is squash-merged. Do not push to `main` directly.
- **Required checks** (all must pass): `Python (compile + pytest)`, `Node (syntax check)`, `Secret scan (gitleaks) + sanitization guard`. The branch must be up to date with `main`.
- **Never commit secrets or personal data**: tokens, passwords, salts, `site/CONFIG`, real site URLs, real emails, machine paths, or anyone's handles. Examples and tests use synthetic data only (`example.com`, fictional names). The sanitization guard and gitleaks will fail the PR otherwise.
- The project is licensed under the [MIT License](LICENSE). By contributing you agree your work is released under the same license.

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

- One focused change per PR, with a short description of what and why.
- Add or update tests for behavior changes; keep `pytest -q` offline and fast.
- User-facing text goes through `i18n.py` and must exist in both `en` and `zh-CN`.
- Docs: English is the default. If you change a user-facing doc, update its `.zh-CN.md` twin with idiomatic Chinese (not a literal translation).
- Add a line under `Unreleased` in [CHANGELOG.md](CHANGELOG.md) for user-visible changes.
- Keep new dependencies to a minimum and update `requirements*.txt` when you add one.

## Reporting bugs and ideas

Use the [issue templates](https://github.com/eatingDream/ai-digest/issues/new/choose). For security problems, see [SECURITY.md](SECURITY.md) and do not open a public issue.

AI coding agents: read [AGENTS.md](AGENTS.md) first.
