#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WINDOWS_ENV_FILE="${MESHLINK_WINDOWS_ENV:-$ROOT_DIR/tests/windows-vm/windows.env}"

load_windows_env() {
  if [[ -f "$WINDOWS_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$WINDOWS_ENV_FILE"
  fi
}

require_commands() {
  local missing=0
  for cmd in "$@"; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      echo "missing required command: $cmd" >&2
      missing=1
    fi
  done
  if (( missing )); then
    exit 1
  fi
}

ensure_iso_with_package() {
  local package_path="$1"
  local output_iso="$2"
  local staging_dir

  if [[ ! -f "$package_path" ]]; then
    echo "package file does not exist: $package_path" >&2
    exit 1
  fi

  staging_dir="$(mktemp -d)"

  cp "$package_path" "$staging_dir/MESHLINK.ZIP"
  xorriso -as mkisofs -quiet -o "$output_iso" "$staging_dir"
  rm -rf "$staging_dir"
}

load_windows_env
require_commands virsh xorriso

MESHLINK_WINDOWS_VM_NAME="${MESHLINK_WINDOWS_VM_NAME:-meshlink-win-a}"
MESHLINK_WINDOWS_PACKAGE="${MESHLINK_WINDOWS_PACKAGE:-}"

if [[ -z "$MESHLINK_WINDOWS_PACKAGE" ]]; then
  echo "MESHLINK_WINDOWS_PACKAGE is required in $WINDOWS_ENV_FILE" >&2
  exit 1
fi

xml="$(virsh dumpxml "$MESHLINK_WINDOWS_VM_NAME")"
iso_path="$(perl -0ne "print \$1 if /<disk[^>]*>.*?<source file='([^']*meshlink-package\\.iso)'.*?<\/disk>/s" <<<"$xml")"
if [[ -z "$iso_path" ]]; then
  echo "could not find meshlink-package.iso attached to $MESHLINK_WINDOWS_VM_NAME" >&2
  exit 1
fi

target_dev="$(perl -0ne "print \$1 if /<disk[^>]*>.*?<source file='[^']*meshlink-package\\.iso'.*?<target dev='([^']+)'.*?<\/disk>/s" <<<"$xml")"
if [[ -z "$target_dev" ]]; then
  echo "could not determine attached CD-ROM target device for meshlink-package.iso" >&2
  exit 1
fi

ensure_iso_with_package "$MESHLINK_WINDOWS_PACKAGE" "$iso_path"
virsh change-media "$MESHLINK_WINDOWS_VM_NAME" "$target_dev" --eject --live >/dev/null 2>&1 || true
virsh change-media "$MESHLINK_WINDOWS_VM_NAME" "$target_dev" "$iso_path" --insert --live >/dev/null

echo "refreshed package ISO for $MESHLINK_WINDOWS_VM_NAME ($target_dev) -> $iso_path"
