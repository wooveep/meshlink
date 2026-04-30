#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SERVER_ADDR="${MESHLINK_PHASE02_ADDR:-127.0.0.1:33074}"
SERVER_LOG="$(mktemp)"
CLIENT_A_LOG="$(mktemp)"
CLIENT_B_LOG="$(mktemp)"
WORK_DIR="$(mktemp -d)"
CLIENT_A_CONFIG="$WORK_DIR/client-a.toml"
CLIENT_B_CONFIG="$WORK_DIR/client-b.toml"
STATE_DB="$WORK_DIR/management.db"

cleanup() {
  for pid_var in SERVER_PID CLIENT_A_PID CLIENT_B_PID; do
    pid="${!pid_var:-}"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  rm -f "$SERVER_LOG" "$CLIENT_A_LOG" "$CLIENT_B_LOG"
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

wait_for_log() {
  local file="$1"
  local pattern="$2"
  local attempts="${3:-30}"
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
  sed -n '1,260p' "$SERVER_LOG" >&2
  echo "--- client-a log ---" >&2
  sed -n '1,320p' "$CLIENT_A_LOG" >&2
  echo "--- client-b log ---" >&2
  sed -n '1,320p' "$CLIENT_B_LOG" >&2
  return 1
}

sed "s/127.0.0.1:33073/${SERVER_ADDR}/" "$ROOT_DIR/deploy/examples/client-config.toml" >"$CLIENT_A_CONFIG"
sed "s/127.0.0.1:33073/${SERVER_ADDR}/" "$ROOT_DIR/deploy/examples/client-b-config.toml" >"$CLIENT_B_CONFIG"
cat >>"$CLIENT_A_CONFIG" <<'EOF'
node_name = "phase02-client-a"
os = "test"
public_key = "phase02-public-key-a"
EOF
cat >>"$CLIENT_B_CONFIG" <<'EOF'
node_name = "phase02-client-b"
os = "test"
public_key = "phase02-public-key-b"
EOF

(
  cd "$ROOT_DIR/server"
  go run ./cmd/managementd \
    -listen "$SERVER_ADDR" \
    -http-listen 127.0.0.1:0 \
    -state-db "$STATE_DB" \
    -sync-interval 1s
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

wait_for_log "$SERVER_LOG" "managementd gRPC listening on ${SERVER_ADDR}"
wait_for_log "$CLIENT_A_LOG" "device registered"
wait_for_log "$CLIENT_B_LOG" "device registered"
wait_for_log "$CLIENT_A_LOG" "tracked_peers=1"
wait_for_log "$CLIENT_B_LOG" "tracked_peers=1"
wait_for_log "$CLIENT_A_LOG" "peer_added=1"
wait_for_log "$CLIENT_B_LOG" "peer_added=1"

echo "phase02 smoke passed"
