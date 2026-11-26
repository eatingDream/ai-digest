#!/usr/bin/env bash
# Fail if any tracked or staged file matches a pattern from .github/sanitize-patterns.txt
# (generic secret/private-path shapes only). Run from the repo root.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
patterns=.github/sanitize-patterns.txt
if [ ! -f "$patterns" ]; then
  echo "sanitization guard: missing $patterns" >&2
  exit 2
fi
tmp=$(mktemp)
out=$(mktemp)
trap 'rm -f "$tmp" "$out"' EXIT
grep -vE '^[[:space:]]*(#|$)' "$patterns" > "$tmp" || true
if [ ! -s "$tmp" ]; then
  echo "sanitization guard: empty pattern list in $patterns" >&2
  exit 2
fi

found=0
scan_scope() {
  local label="$1"
  shift
  local status=0
  : > "$out"
  git grep "$@" -nEIz -f "$tmp" -- . ":(exclude)$patterns" ":(exclude)scripts/check_sanitized.sh" > "$out" || status=$?
  if [ "$status" -eq 0 ]; then
    found=1
    while IFS= read -r -d '' file && IFS= read -r -d '' line; do
      IFS= read -r _matched || true
      printf '%s:%s (%s)\n' "$file" "$line" "$label" >&2
    done < "$out"
  elif [ "$status" -ne 1 ]; then
    echo "sanitization guard: scan error in $label (git grep exited $status)" >&2
    exit "$status"
  fi
}

scan_scope "worktree"
scan_scope "cached" --cached

if [ "$found" -ne 0 ]; then
  echo "sanitization guard: forbidden pattern found (matched text redacted)" >&2
  exit 1
fi
echo "sanitization guard: clean"
