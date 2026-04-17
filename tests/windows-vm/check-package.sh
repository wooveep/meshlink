#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WINDOWS_ENV_FILE="${MESHLINK_WINDOWS_ENV:-$ROOT_DIR/tests/windows-vm/windows.env}"
PACKAGE_PATH="${1:-}"

if [[ -z "$PACKAGE_PATH" && -f "$WINDOWS_ENV_FILE" ]]; then
  # shellcheck disable=SC1090
  source "$WINDOWS_ENV_FILE"
  PACKAGE_PATH="${MESHLINK_WINDOWS_PACKAGE:-}"
fi

if [[ -z "$PACKAGE_PATH" ]]; then
  echo "usage: $0 /path/to/meshlink-client.zip" >&2
  exit 1
fi

if [[ ! -f "$PACKAGE_PATH" ]]; then
  echo "package does not exist: $PACKAGE_PATH" >&2
  exit 1
fi

command -v unzip >/dev/null 2>&1 || {
  echo "unzip is required" >&2
  exit 1
}

archive_listing="$(unzip -Z1 "$PACKAGE_PATH")"

required_files=(
  "meshlinkd.exe"
  "client.example.toml"
  "run-meshlinkd.ps1"
  "README.txt"
  "tunnel.dll"
  "wireguard.dll"
  "wintun.dll"
)

missing=0
for filename in "${required_files[@]}"; do
  if ! grep -Eq "(^|/)$filename$" <<<"$archive_listing"; then
    echo "missing from package: $filename" >&2
    missing=1
  fi
done

if (( missing )); then
  exit 1
fi

echo "package layout looks good: $PACKAGE_PATH"
