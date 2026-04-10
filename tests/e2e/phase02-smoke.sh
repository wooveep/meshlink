#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVER_ADDR="${MESHLINK_PHASE02_ADDR:-127.0.0.1:33074}"
SERVER_LOG="$(mktemp)"
CLIENT_A_LOG="$(mktemp)"
CLIENT_B_LOG="$(mktemp)"
CLIENT_A_CONFIG="$(mktemp)"
CLIENT_B_CONFIG="$(mktemp)"

cleanup() {
  for pid_var in SERVER_PID CLIENT_A_PID CLIENT_B_PID; do
    pid="${!pid_var:-}"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  rm -f "$SERVER_LOG" "$CLIENT_A_LOG" "$CLIENT_B_LOG" "$CLIENT_A_CONFIG" "$CLIENT_B_CONFIG"
}
trap cleanup EXIT

sed "s/127.0.0.1:33073/${SERVER_ADDR}/" "$ROOT_DIR/deploy/examples/client-config.toml" >"$CLIENT_A_CONFIG"
sed "s/127.0.0.1:33073/${SERVER_ADDR}/" "$ROOT_DIR/deploy/examples/client-b-config.toml" >"$CLIENT_B_CONFIG"

(
  cd "$ROOT_DIR/server"
  go run ./cmd/managementd -listen "$SERVER_ADDR" -sync-interval 1s
) >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

sleep 2

timeout 12s cargo run \
  --manifest-path "$ROOT_DIR/client/Cargo.toml" \
  --bin meshlinkd -- \
  --config "$CLIENT_A_CONFIG" \
  >"$CLIENT_A_LOG" 2>&1 &
CLIENT_A_PID=$!

sleep 2

timeout 12s cargo run \
  --manifest-path "$ROOT_DIR/client/Cargo.toml" \
  --bin meshlinkd -- \
  --config "$CLIENT_B_CONFIG" \
  >"$CLIENT_B_LOG" 2>&1 &
CLIENT_B_PID=$!

sleep 6

grep -q "managementd listening on ${SERVER_ADDR}" "$SERVER_LOG"
grep -q "device registered" "$CLIENT_A_LOG"
grep -q "device registered" "$CLIENT_B_LOG"
grep -q "tracked_peers=1" "$CLIENT_A_LOG"
grep -q "tracked_peers=1" "$CLIENT_B_LOG"
grep -q "peer_added=1" "$CLIENT_A_LOG"
grep -q "peer_added=1" "$CLIENT_B_LOG"

echo "phase02 smoke passed"
