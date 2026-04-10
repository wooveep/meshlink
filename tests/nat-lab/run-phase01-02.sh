#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_commands virsh ssh scp go cargo timeout grep

mkdir -p "$MESHLINK_LAB_STATE_DIR/runtime/bin" "$MESHLINK_LAB_STATE_DIR/runtime/config"
REMOTE_ROOT="/home/${MESHLINK_SSH_USER}/meshlink"
MGMT_IP=""

build_artifacts() {
  echo "building host artifacts"
  (
    cd "$ROOT_DIR/server"
    go build -o "$MESHLINK_LAB_STATE_DIR/runtime/bin/managementd" ./cmd/managementd
  )
  cargo build --manifest-path "$ROOT_DIR/client/Cargo.toml" --bin meshlinkd >/dev/null
  cp "$ROOT_DIR/client/target/debug/meshlinkd" "$MESHLINK_LAB_STATE_DIR/runtime/bin/meshlinkd"
}

write_client_config() {
  local node="$1"
  local public_key="$2"

  cat >"$MESHLINK_LAB_STATE_DIR/runtime/config/${node}.toml" <<EOF
node_name = "${node}"
management_addr = "${MGMT_IP}:${MESHLINK_MANAGEMENT_PORT}"
bootstrap_token = "meshlink-dev-token"
public_key = "${public_key}"
interface_name = "sdwan0"
log_level = "info"
EOF
}

copy_runtime() {
  local node="$1"
  ssh_to_vm "$node" "mkdir -p ${REMOTE_ROOT}/bin ${REMOTE_ROOT}/config ${REMOTE_ROOT}/logs && pkill -x managementd || true && pkill -x meshlinkd || true"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/bin/managementd" "$node" "${REMOTE_ROOT}/bin/managementd.new"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/bin/meshlinkd" "$node" "${REMOTE_ROOT}/bin/meshlinkd.new"
  ssh_to_vm "$node" "mv -f ${REMOTE_ROOT}/bin/managementd.new ${REMOTE_ROOT}/bin/managementd && mv -f ${REMOTE_ROOT}/bin/meshlinkd.new ${REMOTE_ROOT}/bin/meshlinkd"
  if [[ -f "$MESHLINK_LAB_STATE_DIR/runtime/config/${node}.toml" ]]; then
    scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/config/${node}.toml" "$node" "${REMOTE_ROOT}/config/client.toml"
  fi
}

start_managementd() {
  ssh_to_vm mgmt-1 "pkill -x managementd || true; nohup ${REMOTE_ROOT}/bin/managementd -listen 0.0.0.0:${MESHLINK_MANAGEMENT_PORT} -sync-interval ${MESHLINK_SYNC_INTERVAL} > ${REMOTE_ROOT}/logs/managementd.log 2>&1 < /dev/null &"
}

start_client() {
  local node="$1"
  ssh_to_vm "$node" "pkill -x meshlinkd || true; nohup timeout 20s ${REMOTE_ROOT}/bin/meshlinkd --config ${REMOTE_ROOT}/config/client.toml > ${REMOTE_ROOT}/logs/meshlinkd.log 2>&1 < /dev/null &"
}

assert_remote_log() {
  local node="$1"
  local path="$2"
  local pattern="$3"
  ssh_to_vm "$node" "grep -q '$pattern' '$path'"
}

collect_log() {
  local node="$1"
  local remote_path="$2"
  local local_path="$MESHLINK_LAB_STATE_DIR/runtime/${node}-$(basename "$remote_path")"
  scp_from_vm "$node" "$remote_path" "$local_path" >/dev/null
  echo "collected $local_path"
}

for node in mgmt-1 client-a client-b; do
  if ! virsh domstate "$(vm_name "$node")" >/dev/null 2>&1; then
    echo "vm is missing: $(vm_name "$node"); run tests/nat-lab/create-lab.sh first" >&2
    exit 1
  fi
done

for node in mgmt-1 client-a client-b; do
  wait_for_ssh "$node"
done

MGMT_IP="$(resolved_vm_ip mgmt-1)"

build_artifacts
write_client_config client-a meshlink-client-a-public-key
write_client_config client-b meshlink-client-b-public-key

copy_runtime mgmt-1
copy_runtime client-a
copy_runtime client-b

start_managementd
sleep 2
start_client client-a
sleep 2
start_client client-b
sleep 8

assert_remote_log mgmt-1 "${REMOTE_ROOT}/logs/managementd.log" "managementd listening on 0.0.0.0:${MESHLINK_MANAGEMENT_PORT}"
assert_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "device registered"
assert_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "device registered"
assert_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "tracked_peers=1"
assert_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "tracked_peers=1"
assert_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "peer_added=1"
assert_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "peer_added=1"

collect_log mgmt-1 "${REMOTE_ROOT}/logs/managementd.log"
collect_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log"
collect_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log"

echo "vm lab phase01-02 acceptance passed"
