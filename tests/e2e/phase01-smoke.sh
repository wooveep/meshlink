#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVER_ADDR="${MESHLINK_PHASE01_ADDR:-127.0.0.1:33073}"
SERVER_LOG="$(mktemp)"
CLIENT_LOG="$(mktemp)"
WORK_DIR="$(mktemp -d)"
CLIENT_CONFIG="$WORK_DIR/client.toml"
STATE_DB="$WORK_DIR/management.db"

cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -f "$SERVER_LOG" "$CLIENT_LOG"
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

wait_for_log() {
  local file="$1"
  local pattern="$2"
  local attempts="${3:-20}"
  local attempt=1

  while (( attempt <= attempts )); do
    if grep -q "$pattern" "$file"; then
      return 0
    fi
    sleep 0.5
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for log pattern: $pattern" >&2
  echo "--- server log ---" >&2
  sed -n '1,220p' "$SERVER_LOG" >&2
  echo "--- client log ---" >&2
  sed -n '1,260p' "$CLIENT_LOG" >&2
  return 1
}

sed "s/127.0.0.1:33073/${SERVER_ADDR}/" "$ROOT_DIR/deploy/examples/client-config.toml" >"$CLIENT_CONFIG"
cat >>"$CLIENT_CONFIG" <<'EOF'
node_name = "phase01-client"
os = "test"
public_key = "phase01-public-key"
EOF

(
  cd "$ROOT_DIR/server"
  go run ./cmd/managementd \
    -listen "$SERVER_ADDR" \
    -http-listen 127.0.0.1:0 \
    -state-db "$STATE_DB"
) >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

sleep 2

timeout 5s cargo run \
  --manifest-path "$ROOT_DIR/client/Cargo.toml" \
  --bin meshlinkd -- \
  --config "$CLIENT_CONFIG" \
  >"$CLIENT_LOG" 2>&1 || true

wait_for_log "$SERVER_LOG" "managementd gRPC listening on ${SERVER_ADDR}"
wait_for_log "$CLIENT_LOG" "device registered"
wait_for_log "$CLIENT_LOG" "received config event"

echo "phase01 smoke passed"
