#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT_DIR/dist/bin/linux-amd64/client"

mkdir -p "$OUT_DIR"

echo "Building meshlinkd release binary"
cargo build --manifest-path "$ROOT_DIR/client/Cargo.toml" --release --bin meshlinkd
install -m 0755 "$ROOT_DIR/client/target/release/meshlinkd" "$OUT_DIR/meshlinkd"

echo "Client artifact written to $OUT_DIR"
