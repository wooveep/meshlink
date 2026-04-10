#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVER_ADDR="${MESHLINK_PHASE01_ADDR:-127.0.0.1:33073}"
SERVER_LOG="$(mktemp)"
CLIENT_LOG="$(mktemp)"
CLIENT_CONFIG="$(mktemp)"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -f "$SERVER_LOG" "$CLIENT_LOG" "$CLIENT_CONFIG"
}
trap cleanup EXIT

sed "s/127.0.0.1:33073/${SERVER_ADDR}/" "$ROOT_DIR/deploy/examples/client-config.toml" >"$CLIENT_CONFIG"

(
  cd "$ROOT_DIR/server"
  go run ./cmd/managementd -listen "$SERVER_ADDR"
) >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

sleep 2

timeout 5s cargo run \
  --manifest-path "$ROOT_DIR/client/Cargo.toml" \
  --bin meshlinkd -- \
  --config "$CLIENT_CONFIG" \
  >"$CLIENT_LOG" 2>&1 || true

grep -q "managementd listening on ${SERVER_ADDR}" "$SERVER_LOG"
grep -q "device registered" "$CLIENT_LOG"
grep -q "received config event" "$CLIENT_LOG"

echo "phase01 smoke passed"
