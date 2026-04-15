#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_dual_nat_topology
require_commands virsh ssh scp go cargo timeout grep wg ping

mkdir -p "$MESHLINK_LAB_STATE_DIR/runtime/bin" "$MESHLINK_LAB_STATE_DIR/runtime/config"
REMOTE_ROOT="/home/${MESHLINK_SSH_USER}/meshlink"
MESHLINK_INTERFACE_NAME="${MESHLINK_INTERFACE_NAME:-sdwan0}"
MESHLINK_SIGNAL_PORT="${MESHLINK_SIGNAL_PORT:-10000}"
MESHLINK_STUN_PORT="${MESHLINK_STUN_PORT:-3479}"
MESHLINK_RELAY_PORT="${MESHLINK_RELAY_PORT:-3478}"
MESHLINK_CLIENT_A_WG_PORT="${MESHLINK_CLIENT_A_WG_PORT:-51820}"
MESHLINK_CLIENT_B_WG_PORT="${MESHLINK_CLIENT_B_WG_PORT:-51821}"
CLIENT_A_OVERLAY="100.64.0.1"
CLIENT_B_OVERLAY="100.64.0.2"
MGMT_IP="$(vm_ip mgmt-1)"
CLIENT_A_PRIVATE_KEY=""
CLIENT_A_PUBLIC_KEY=""
CLIENT_B_PRIVATE_KEY=""
CLIENT_B_PUBLIC_KEY=""

trap 'clear_phase06_drop_rules >/dev/null 2>&1 || true' EXIT

build_artifacts() {
  echo "building host artifacts"
  (
    cd "$ROOT_DIR/server"
    go build -o "$MESHLINK_LAB_STATE_DIR/runtime/bin/managementd" ./cmd/managementd
    go build -o "$MESHLINK_LAB_STATE_DIR/runtime/bin/signald" ./cmd/signald
    go build -o "$MESHLINK_LAB_STATE_DIR/runtime/bin/relayd" ./cmd/relayd
  )
  cargo build --manifest-path "$ROOT_DIR/client/Cargo.toml" --bin meshlinkd >/dev/null
  cp "$ROOT_DIR/client/target/debug/meshlinkd" "$MESHLINK_LAB_STATE_DIR/runtime/bin/meshlinkd"
}

generate_wireguard_keys() {
  CLIENT_A_PRIVATE_KEY="$(wg genkey)"
  CLIENT_A_PUBLIC_KEY="$(printf '%s' "$CLIENT_A_PRIVATE_KEY" | wg pubkey)"
  CLIENT_B_PRIVATE_KEY="$(wg genkey)"
  CLIENT_B_PUBLIC_KEY="$(printf '%s' "$CLIENT_B_PRIVATE_KEY" | wg pubkey)"
}

write_client_config() {
  local node="$1"
  local public_key="$2"
  local private_key="$3"
  local listen_port="$4"

  cat >"$MESHLINK_LAB_STATE_DIR/runtime/config/${node}.toml" <<EOF
node_name = "${node}"
management_addr = "${MGMT_IP}:${MESHLINK_MANAGEMENT_PORT}"
signal_addr = "${MGMT_IP}:${MESHLINK_SIGNAL_PORT}"
relay_addr = "${MGMT_IP}:${MESHLINK_RELAY_PORT}"
bootstrap_token = "meshlink-dev-token"
public_key = "${public_key}"
private_key = "${private_key}"
interface_name = "${MESHLINK_INTERFACE_NAME}"
listen_port = ${listen_port}
log_level = "info"
punch_timeout = "4s"
EOF
}

copy_management_runtime() {
  ssh_to_vm mgmt-1 "mkdir -p ${REMOTE_ROOT}/bin ${REMOTE_ROOT}/logs && sudo pkill -x managementd || true && sudo pkill -x signald || true && sudo pkill -x relayd || true"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/bin/managementd" mgmt-1 "${REMOTE_ROOT}/bin/managementd.new"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/bin/signald" mgmt-1 "${REMOTE_ROOT}/bin/signald.new"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/bin/relayd" mgmt-1 "${REMOTE_ROOT}/bin/relayd.new"
  ssh_to_vm mgmt-1 "mv -f ${REMOTE_ROOT}/bin/managementd.new ${REMOTE_ROOT}/bin/managementd && mv -f ${REMOTE_ROOT}/bin/signald.new ${REMOTE_ROOT}/bin/signald && mv -f ${REMOTE_ROOT}/bin/relayd.new ${REMOTE_ROOT}/bin/relayd"
}

copy_client_runtime() {
  local node="$1"
  ssh_to_vm "$node" "mkdir -p ${REMOTE_ROOT}/bin ${REMOTE_ROOT}/config ${REMOTE_ROOT}/logs && sudo pkill -x meshlinkd || true"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/bin/meshlinkd" "$node" "${REMOTE_ROOT}/bin/meshlinkd.new"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/config/${node}.toml" "$node" "${REMOTE_ROOT}/config/client.toml"
  ssh_to_vm "$node" "mv -f ${REMOTE_ROOT}/bin/meshlinkd.new ${REMOTE_ROOT}/bin/meshlinkd"
}

start_managementd() {
  ssh_to_vm mgmt-1 "sudo pkill -x managementd || true; nohup ${REMOTE_ROOT}/bin/managementd -listen 0.0.0.0:${MESHLINK_MANAGEMENT_PORT} -sync-interval ${MESHLINK_SYNC_INTERVAL} > ${REMOTE_ROOT}/logs/managementd.log 2>&1 < /dev/null &"
}

start_signald() {
  ssh_to_vm mgmt-1 "sudo pkill -x signald || true; nohup ${REMOTE_ROOT}/bin/signald -listen 0.0.0.0:${MESHLINK_SIGNAL_PORT} -stun-listen 0.0.0.0:${MESHLINK_STUN_PORT} -management-addr 127.0.0.1:${MESHLINK_MANAGEMENT_PORT} > ${REMOTE_ROOT}/logs/signald.log 2>&1 < /dev/null &"
}

start_relayd() {
  ssh_to_vm mgmt-1 "sudo pkill -x relayd || true; nohup ${REMOTE_ROOT}/bin/relayd -listen 0.0.0.0:${MESHLINK_RELAY_PORT} -management-addr 127.0.0.1:${MESHLINK_MANAGEMENT_PORT} -advertise-host ${MGMT_IP} > ${REMOTE_ROOT}/logs/relayd.log 2>&1 < /dev/null &"
}

start_client() {
  local node="$1"
  ssh_to_vm "$node" "sudo pkill -x meshlinkd || true; sudo ip link del ${MESHLINK_INTERFACE_NAME} 2>/dev/null || true; nohup sudo timeout 120s ${REMOTE_ROOT}/bin/meshlinkd --config ${REMOTE_ROOT}/config/client.toml > ${REMOTE_ROOT}/logs/meshlinkd.log 2>&1 < /dev/null &"
}

wait_for_router_tools() {
  local node="$1"
  local attempts="${2:-90}"
  local attempt=1

  echo "waiting for router tools on ${node}"

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "command -v iptables >/dev/null 2>&1 && command -v iptables-save >/dev/null 2>&1 && command -v ping >/dev/null 2>&1 && sudo true" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for router tools on $node" >&2
  return 1
}

wait_for_client_tools() {
  local node="$1"
  local attempts="${2:-90}"
  local attempt=1

  echo "waiting for client tools on ${node}"

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "command -v ping >/dev/null 2>&1 && command -v wg >/dev/null 2>&1 && sudo true" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for client tools on $node" >&2
  return 1
}

wait_for_remote_log() {
  local node="$1"
  local path="$2"
  local pattern="$3"
  local attempts="${4:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "grep -q '$pattern' '$path'" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for log pattern on ${node}: ${pattern}" >&2
  return 1
}

wg_endpoint_for_peer() {
  local node="$1"
  local peer_public_key="$2"
  ssh_to_vm "$node" "sudo wg show ${MESHLINK_INTERFACE_NAME} endpoints | awk '\$1 == \"${peer_public_key}\" {print \$2}'" 2>/dev/null || true
}

wait_for_wg_endpoint() {
  local node="$1"
  local peer_public_key="$2"
  local expected_host="$3"
  local expected_port="$4"
  local attempts="${5:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    local endpoint=""
    endpoint="$(wg_endpoint_for_peer "$node" "$peer_public_key")"
    if [[ "$endpoint" == "${expected_host}:${expected_port}" ]]; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for ${node} endpoint ${expected_host}:${expected_port}" >&2
  return 1
}

wait_for_relay_endpoint() {
  local node="$1"
  local peer_public_key="$2"
  local expected_host="$3"
  local attempts="${4:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    local endpoint=""
    endpoint="$(wg_endpoint_for_peer "$node" "$peer_public_key")"
    if [[ "$endpoint" == "${expected_host}:"* ]]; then
      printf '%s\n' "$endpoint"
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for ${node} relay endpoint on host ${expected_host}" >&2
  return 1
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

for node in mgmt-1 nat-a nat-b client-a client-b; do
  if ! virsh domstate "$(vm_name "$node")" >/dev/null 2>&1; then
    echo "vm is missing: $(vm_name "$node"); run tests/nat-lab/create-lab.sh first" >&2
    exit 1
  fi
done

preflight_dual_nat_access

for node in nat-a nat-b; do
  wait_for_router_tools "$node"
  wait_for_nat_router_ready "$node"
done

ensure_dual_nat_wireguard_port_mapping

for node in client-a client-b; do
  wait_for_client_tools "$node"
done

build_artifacts
generate_wireguard_keys
write_client_config client-a "$CLIENT_A_PUBLIC_KEY" "$CLIENT_A_PRIVATE_KEY" "$MESHLINK_CLIENT_A_WG_PORT"
write_client_config client-b "$CLIENT_B_PUBLIC_KEY" "$CLIENT_B_PRIVATE_KEY" "$MESHLINK_CLIENT_B_WG_PORT"

copy_management_runtime
copy_client_runtime client-a
copy_client_runtime client-b

clear_phase06_drop_rules
start_managementd
sleep 2
start_signald
sleep 2
start_relayd
sleep 2
start_client client-a
sleep 2
start_client client-b

wait_for_remote_log mgmt-1 "${REMOTE_ROOT}/logs/managementd.log" "managementd listening on 0.0.0.0:${MESHLINK_MANAGEMENT_PORT}"
wait_for_remote_log mgmt-1 "${REMOTE_ROOT}/logs/signald.log" "signald listening on 0.0.0.0:${MESHLINK_SIGNAL_PORT}"
wait_for_remote_log mgmt-1 "${REMOTE_ROOT}/logs/signald.log" "signald STUN listening on 0.0.0.0:${MESHLINK_STUN_PORT}"
wait_for_remote_log mgmt-1 "${REMOTE_ROOT}/logs/relayd.log" "relayd listening on 0.0.0.0:${MESHLINK_RELAY_PORT}"
wait_for_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "device registered"
wait_for_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "device registered"
wait_for_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "hole punch handshake observed" 50
wait_for_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "hole punch handshake observed" 50

wait_for_wg_endpoint client-a "$CLIENT_B_PUBLIC_KEY" "$MESHLINK_NAT_B_WAN_IP" "$MESHLINK_CLIENT_B_WG_PORT" 50
wait_for_wg_endpoint client-b "$CLIENT_A_PUBLIC_KEY" "$MESHLINK_NAT_A_WAN_IP" "$MESHLINK_CLIENT_A_WG_PORT" 50

ssh_to_vm client-a "sudo ping -c 2 -W 2 ${CLIENT_B_OVERLAY} >/dev/null"
ssh_to_vm client-b "sudo ping -c 2 -W 2 ${CLIENT_A_OVERLAY} >/dev/null"

collect_router_state nat-a direct
collect_router_state nat-b direct

install_phase06_drop_rules
collect_router_state nat-a blocked
collect_router_state nat-b blocked

wait_for_remote_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log" "relay fallback activated" 50
wait_for_remote_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log" "relay fallback activated" 50

CLIENT_A_RELAY_ENDPOINT="$(wait_for_relay_endpoint client-a "$CLIENT_B_PUBLIC_KEY" "$MGMT_IP" 50)"
CLIENT_B_RELAY_ENDPOINT="$(wait_for_relay_endpoint client-b "$CLIENT_A_PUBLIC_KEY" "$MGMT_IP" 50)"

if [[ "${CLIENT_A_RELAY_ENDPOINT#*:}" != "${CLIENT_B_RELAY_ENDPOINT#*:}" ]]; then
  echo "relay endpoints differ between clients: ${CLIENT_A_RELAY_ENDPOINT} vs ${CLIENT_B_RELAY_ENDPOINT}" >&2
  exit 1
fi

ssh_to_vm client-a "sudo ping -c 2 -W 2 ${CLIENT_B_OVERLAY} >/dev/null"
ssh_to_vm client-b "sudo ping -c 2 -W 2 ${CLIENT_A_OVERLAY} >/dev/null"

clear_phase06_drop_rules
collect_router_state nat-a cleared
collect_router_state nat-b cleared

wait_for_wg_endpoint client-a "$CLIENT_B_PUBLIC_KEY" "$MESHLINK_NAT_B_WAN_IP" "$MESHLINK_CLIENT_B_WG_PORT" 50
wait_for_wg_endpoint client-b "$CLIENT_A_PUBLIC_KEY" "$MESHLINK_NAT_A_WAN_IP" "$MESHLINK_CLIENT_A_WG_PORT" 50

ssh_to_vm client-a "sudo ping -c 2 -W 2 ${CLIENT_B_OVERLAY} >/dev/null"
ssh_to_vm client-b "sudo ping -c 2 -W 2 ${CLIENT_A_OVERLAY} >/dev/null"

collect_log mgmt-1 "${REMOTE_ROOT}/logs/managementd.log"
collect_log mgmt-1 "${REMOTE_ROOT}/logs/signald.log"
collect_log mgmt-1 "${REMOTE_ROOT}/logs/relayd.log"
collect_log client-a "${REMOTE_ROOT}/logs/meshlinkd.log"
collect_log client-b "${REMOTE_ROOT}/logs/meshlinkd.log"
collect_wg_state client-a
collect_wg_state client-b

echo "vm lab phase06 relay fallback acceptance passed"
