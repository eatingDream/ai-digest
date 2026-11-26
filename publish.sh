#!/usr/bin/env sh
# Thin POSIX wrapper resolving the ai-digest binary and forwarding to `ai-digest publish`.
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

if [ -n "${AI_DIGEST_BIN:-}" ]; then
  BIN="$AI_DIGEST_BIN"
elif [ -x "$SCRIPT_DIR/bin/ai-digest" ]; then
  BIN="$SCRIPT_DIR/bin/ai-digest"
elif [ -x "$SCRIPT_DIR/ai-digest" ]; then
  BIN="$SCRIPT_DIR/ai-digest"
elif [ -x "$SCRIPT_DIR/target/release/ai-digest" ]; then
  BIN="$SCRIPT_DIR/target/release/ai-digest"
elif command -v ai-digest >/dev/null 2>&1; then
  BIN=$(command -v ai-digest)
else
  echo "error: ai-digest binary not found (set AI_DIGEST_BIN, place bin/ai-digest or ai-digest beside publish.sh, build target/release/ai-digest, or install ai-digest on PATH)" >&2
  exit 127
fi

exec "$BIN" publish "$@"
