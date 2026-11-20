#!/usr/bin/env bash
# Publish one AI digest issue (or --rebuild / --deploy-only). See publish.py.
set -euo pipefail
exec python3 "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/publish.py" "$@"
