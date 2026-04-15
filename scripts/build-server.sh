#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT_DIR/dist/bin/linux-amd64/server"

mkdir -p "$OUT_DIR"

build() {
  local name="$1"
  echo "Building $name for linux/amd64"
  (
    cd "$ROOT_DIR/server"
    CGO_ENABLED=0 GOOS=linux GOARCH=amd64 \
      go build -trimpath -ldflags="-s -w" -o "$OUT_DIR/$name" "./cmd/$name"
  )
}

build managementd
build signald
build relayd

echo "Server artifacts written to $OUT_DIR"
