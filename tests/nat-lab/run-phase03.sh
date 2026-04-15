#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_flat_topology
require_commands virsh ssh scp go cargo timeout grep wg

mkdir -p "$MESHLINK_LAB_STATE_DIR/runtime/bin" "$MESHLINK_LAB_STATE_DIR/runtime/config" "$MESHLINK_LAB_STATE_DIR/runtime/keys"
REMOTE_ROOT="/home/${MESHLINK_SSH_USER}/meshlink"
MESHLINK_INTERFACE_NAME="${MESHLINK_INTERFACE_NAME:-sdwan0}"
MESHLINK_CLIENT_A_WG_PORT="${MESHLINK_CLIENT_A_WG_PORT:-51820}"
MESHLINK_CLIENT_B_WG_PORT="${MESHLINK_CLIENT_B_WG_PORT:-51821}"
CLIENT_A_OVERLAY="100.64.0.1"
CLIENT_B_OVERLAY="100.64.0.2"
MGMT_IP=""
CLIENT_A_IP=""
CLIENT_B_IP=""
CLIENT_A_PRIVATE_KEY=""
CLIENT_A_PUBLIC_KEY=""
CLIENT_B_PRIVATE_KEY=""
CLIENT_B_PUBLIC_KEY=""

build_artifacts() {
  echo "building host artifacts"
  (
    cd "$ROOT_DIR/server"
    go build -o "$MESHLINK_LAB_STATE_DIR/runtime/bin/managementd" ./cmd/managementd
  )
  cargo build --manifest-path "$ROOT_DIR/client/Cargo.toml" --bin meshlinkd >/dev/null
  cp "$ROOT_DIR/client/target/debug/meshlinkd" "$MESHLINK_LAB_STATE_DIR/runtime/bin/meshlinkd"
}

generate_wireguard_keys() {
  CLIENT_A_PRIVATE_KEY="$(wg genkey)"
  CLIENT_A_PUBLIC_KEY="$(printf '%s' "$CLIENT_A_PRIVATE_KEY" | wg pubkey)"
  CLIENT_B_PRIVATE_KEY="$(wg genkey)"
  CLIENT_B_PUBLIC_KEY="$(printf '%s' "$CLIENT_B_PRIVATE_KEY" | wg pubkey)"

  printf '%s\n' "$CLIENT_A_PRIVATE_KEY" >"$MESHLINK_LAB_STATE_DIR/runtime/keys/client-a.private"
  printf '%s\n' "$CLIENT_A_PUBLIC_KEY" >"$MESHLINK_LAB_STATE_DIR/runtime/keys/client-a.public"
  printf '%s\n' "$CLIENT_B_PRIVATE_KEY" >"$MESHLINK_LAB_STATE_DIR/runtime/keys/client-b.private"
  printf '%s\n' "$CLIENT_B_PUBLIC_KEY" >"$MESHLINK_LAB_STATE_DIR/runtime/keys/client-b.public"
}

write_client_config() {
  local node="$1"
  local public_key="$2"
  local private_key="$3"
  local advertise_host="$4"
  local listen_port="$5"

  cat >"$MESHLINK_LAB_STATE_DIR/runtime/config/${node}.toml" <<EOF
node_name = "${node}"
management_addr = "${MGMT_IP}:${MESHLINK_MANAGEMENT_PORT}"
bootstrap_token = "meshlink-dev-token"
public_key = "${public_key}"
private_key = "${private_key}"
interface_name = "${MESHLINK_INTERFACE_NAME}"
listen_port = ${listen_port}
advertise_host = "${advertise_host}"
log_level = "info"
EOF
}

copy_runtime() {
  local node="$1"
  ssh_to_vm "$node" "mkdir -p ${REMOTE_ROOT}/bin ${REMOTE_ROOT}/config ${REMOTE_ROOT}/logs && pkill -x managementd || true && sudo pkill -x meshlinkd || true"
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
  ssh_to_vm "$node" "sudo pkill -x meshlinkd || true; sudo ip link del ${MESHLINK_INTERFACE_NAME} 2>/dev/null || true; nohup sudo timeout 45s ${REMOTE_ROOT}/bin/meshlinkd --config ${REMOTE_ROOT}/config/client.toml > ${REMOTE_ROOT}/logs/meshlinkd.log 2>&1 < /dev/null &"
}

wait_for_guest_tools() {
  local node="$1"
  local attempts="${2:-90}"
  local attempt=1

  echo "waiting for wireguard tools on ${node}"

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "command -v wg >/dev/null 2>&1 && command -v ping >/dev/null 2>&1 && sudo true" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for packages on $node; if these VMs predate Phase 03, destroy and recreate the lab so cloud-init can install wireguard-tools" >&2
  return 1
}

assert_remote_log() {
  local node="$1"
  local path="$2"
  local pattern="$3"
  ssh_to_vm "$node" "grep -q '$pattern' '$path'"
}

assert_remote_command() {
  local node="$1"
  local command="$2"
  ssh_to_vm "$node" "$command"
}

collect_log() {
  local node="$1"
  local remote_path="$2"
  local local_path="$MESHLINK_LAB_STATE_DIR/runtime/${node}-$(basename "$remote_path")"
  scp_from_vm "$node" "$remote_path" "$local_path" >/dev/null
  echo "collected $local_path"
}

collect_wg_state() {
  local node="$1"
  local local_path="$MESHLINK_LAB_STATE_DIR/runtime/${node}-wg-show.txt"
  ssh_to_vm "$node" "sudo wg show ${MESHLINK_INTERFACE_NAME}" >"$local_path"
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

for node in client-a client-b; do
  wait_for_guest_tools "$node"
done

MGMT_IP="$(resolved_vm_ip mgmt-1)"
CLIENT_A_IP="$(resolved_vm_ip client-a)"
CLIENT_B_IP="$(resolved_vm_ip client-b)"

build_artifacts
generate_wireguard_keys
write_client_config client-a "$CLIENT_A_PUBLIC_KEY" "$CLIENT_A_PRIVATE_KEY" "$CLIENT_A_IP" "$MESHLINK_CLIENT_A_WG_PORT"
write_client_config client-b "$CLIENT_B_PUBLIC_KEY" "$CLIENT_B_PRIVATE_KEY" "$CLIENT_B_IP" "$MESHLINK_CLIENT_B_WG_PORT"

copy_runtime mgmt-1
copy_runtime client-a
copy_runtime client-b

start_managementd
sleep 2
start_client client-a
sleep 2
start_client client-b
sleep 10

assert_remote_log mgmt-1 "${REMOTE_ROOT}/logs/managementd.log" "managementd listening on 0.0.0.0:${MESHLINK_MANAGEMENT_PORT}"
assert_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "device registered"
assert_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "device registered"
assert_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "wireguard state reconciled"
assert_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "wireguard state reconciled"

assert_remote_command client-a "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'peer: ${CLIENT_B_PUBLIC_KEY}'"
assert_remote_command client-b "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'peer: ${CLIENT_A_PUBLIC_KEY}'"
assert_remote_command client-a "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'endpoint: ${CLIENT_B_IP}:${MESHLINK_CLIENT_B_WG_PORT}'"
assert_remote_command client-b "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'endpoint: ${CLIENT_A_IP}:${MESHLINK_CLIENT_A_WG_PORT}'"

assert_remote_command client-a "sudo ping -c 2 -W 2 ${CLIENT_B_OVERLAY} >/dev/null"
assert_remote_command client-b "sudo ping -c 2 -W 2 ${CLIENT_A_OVERLAY} >/dev/null"

collect_log mgmt-1 "${REMOTE_ROOT}/logs/managementd.log"
collect_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log"
collect_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log"
collect_wg_state client-a
collect_wg_state client-b

echo "vm lab phase03 acceptance passed"
