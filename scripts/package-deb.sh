#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/deb"
CONFIG_DIR="$ROOT_DIR/deploy/packages/nfpm"

derive_version() {
  awk '
    /^\[workspace.package\]$/ { in_workspace = 1; next }
    /^\[/ { in_workspace = 0 }
    in_workspace && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "$ROOT_DIR/client/Cargo.toml"
}

run_nfpm() {
  if command -v nfpm >/dev/null 2>&1; then
    nfpm "$@"
    return
  fi

  if ! command -v go >/dev/null 2>&1; then
    echo "nfpm is not installed and go is unavailable for fallback packaging" >&2
    exit 1
  fi

  GOFLAGS="${GOFLAGS:-}" go run github.com/goreleaser/nfpm/v2/cmd/nfpm@v2.41.2 "$@"
}

MESHLINK_PACKAGE_VERSION="${MESHLINK_PACKAGE_VERSION:-$(derive_version)}"
MESHLINK_PACKAGE_RELEASE="${MESHLINK_PACKAGE_RELEASE:-1}"

mkdir -p "$DIST_DIR"

"$ROOT_DIR/scripts/build-server.sh"
"$ROOT_DIR/scripts/build-client.sh"

export MESHLINK_PACKAGE_VERSION
export MESHLINK_PACKAGE_RELEASE

packages=(
  meshlink-managementd
  meshlink-signald
  meshlink-relayd
  meshlink-client
)

for package in "${packages[@]}"; do
  target="$DIST_DIR/${package}_${MESHLINK_PACKAGE_VERSION}-${MESHLINK_PACKAGE_RELEASE}_amd64.deb"
  echo "Packaging $package -> $target"
  run_nfpm package \
    --packager deb \
    --config "$CONFIG_DIR/${package}.yaml" \
    --target "$target"
done

echo "Debian packages written to $DIST_DIR"
