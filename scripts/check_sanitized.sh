#!/usr/bin/env bash
# Fail if any tracked file matches a pattern from .github/sanitize-patterns.txt
# (generic secret/private-path shapes only). Run from the repo root.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
patterns=.github/sanitize-patterns.txt
grep -vE '^\s*(#|$)' "$patterns" > /tmp/sanitize-patterns.$$
trap 'rm -f /tmp/sanitize-patterns.$$' EXIT
if git grep -nEI -f /tmp/sanitize-patterns.$$ -- . ":(exclude)$patterns" ":(exclude)scripts/check_sanitized.sh"; then
  echo "sanitization guard: forbidden pattern found (see matches above)" >&2
  exit 1
fi
echo "sanitization guard: clean"
