#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UI_DIR="$ROOT_DIR/server/admin-ui"

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required to build the admin UI" >&2
  exit 1
fi

if [[ -f "$UI_DIR/package-lock.json" ]]; then
  npm --prefix "$UI_DIR" ci
else
  npm --prefix "$UI_DIR" install
fi

npm --prefix "$UI_DIR" run build
