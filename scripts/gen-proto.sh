#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROTO_DIR="$ROOT_DIR/proto"
OUT_DIR="$ROOT_DIR/server/pkg/pb"

mkdir -p "$OUT_DIR"

export PATH="$PATH:$(go env GOPATH)/bin"

protoc \
  -I "$PROTO_DIR" \
  --go_out="$OUT_DIR" \
  --go_opt=paths=source_relative \
  --go-grpc_out="$OUT_DIR" \
  --go-grpc_opt=paths=source_relative \
  "$PROTO_DIR/common.proto" \
  "$PROTO_DIR/management.proto" \
  "$PROTO_DIR/relay.proto" \
  "$PROTO_DIR/signal.proto"

echo "Generated Go protobuf stubs into $OUT_DIR"
