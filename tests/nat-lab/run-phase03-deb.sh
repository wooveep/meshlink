#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_flat_topology
require_commands virsh ssh scp timeout grep wg dpkg-deb

mkdir -p "$MESHLINK_LAB_STATE_DIR/runtime/deb" "$MESHLINK_LAB_STATE_DIR/runtime/config" "$MESHLINK_LAB_STATE_DIR/runtime/keys"
REMOTE_ROOT="/home/${MESHLINK_SSH_USER}/meshlink"
REMOTE_DEB_DIR="${REMOTE_ROOT}/deb"
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
PACKAGE_DIR="$ROOT_DIR/dist/deb"
MANAGEMENTD_DEB=""
SIGNALD_DEB=""
RELAYD_DEB=""
CLIENT_DEB=""

ensure_deb_artifacts() {
  MANAGEMENTD_DEB="$(ls "$PACKAGE_DIR"/meshlink-managementd_*_amd64.deb 2>/dev/null | head -n1 || true)"
  SIGNALD_DEB="$(ls "$PACKAGE_DIR"/meshlink-signald_*_amd64.deb 2>/dev/null | head -n1 || true)"
  RELAYD_DEB="$(ls "$PACKAGE_DIR"/meshlink-relayd_*_amd64.deb 2>/dev/null | head -n1 || true)"
  CLIENT_DEB="$(ls "$PACKAGE_DIR"/meshlink-client_*_amd64.deb 2>/dev/null | head -n1 || true)"

  if [[ -z "$MANAGEMENTD_DEB" || -z "$SIGNALD_DEB" || -z "$RELAYD_DEB" || -z "$CLIENT_DEB" ]]; then
    echo "missing deb artifacts under $PACKAGE_DIR; run 'make package-deb' first" >&2
    exit 1
  fi

  for package in "$MANAGEMENTD_DEB" "$SIGNALD_DEB" "$RELAYD_DEB" "$CLIENT_DEB"; do
    dpkg-deb -I "$package" >/dev/null
  done
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

write_management_env() {
  cat >"$MESHLINK_LAB_STATE_DIR/runtime/config/meshlink-managementd.env" <<EOF
MESHLINK_MANAGEMENTD_LISTEN=0.0.0.0:${MESHLINK_MANAGEMENT_PORT}
MESHLINK_MANAGEMENTD_BOOTSTRAP_TOKEN=meshlink-dev-token
MESHLINK_MANAGEMENTD_OVERLAY_CIDR=100.64.0.0/10
MESHLINK_MANAGEMENTD_SYNC_INTERVAL=${MESHLINK_SYNC_INTERVAL}
EOF
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

wait_for_management_tools() {
  local node="$1"
  local attempts="${2:-90}"
  local attempt=1

  echo "waiting for management-node tools on ${node}"

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "command -v dpkg >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1 && command -v journalctl >/dev/null 2>&1 && sudo true" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for guest tools on $node; recreate the lab so cloud-init can finish package setup" >&2
  return 1
}

wait_for_client_system_tools() {
  local node="$1"
  local attempts="${2:-90}"
  local attempt=1

  echo "waiting for client-node system tools on ${node}"

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "command -v ping >/dev/null 2>&1 && command -v ip >/dev/null 2>&1 && command -v dpkg >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1 && command -v journalctl >/dev/null 2>&1 && sudo true" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for guest tools on $node; recreate the lab so cloud-init can finish package setup" >&2
  return 1
}

wait_for_cloud_init() {
  local node="$1"
  local attempts="${2:-90}"
  local attempt=1

  echo "waiting for cloud-init on ${node}"

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "output=\$(cloud-init status --wait 2>/dev/null || true); printf '%s\n' \"\$output\" | grep -q '^status: done$'"; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for cloud-init on $node" >&2
  return 1
}

assert_client_runtime_dependencies() {
  local node="$1"
  if ! ssh_to_vm "$node" "command -v wg >/dev/null 2>&1 && command -v ip >/dev/null 2>&1"; then
    echo "client runtime dependencies are missing on $node; recreate the lab so cloud-init installs wireguard-tools and iproute2" >&2
    exit 1
  fi
}

prepare_remote_dirs() {
  local node="$1"
  ssh_to_vm "$node" "mkdir -p ${REMOTE_DEB_DIR} ${REMOTE_ROOT}/config ${REMOTE_ROOT}/logs"
}

copy_deb_packages() {
  echo "copying deb packages to guests"
  scp_to_vm "$MANAGEMENTD_DEB" mgmt-1 "${REMOTE_DEB_DIR}/$(basename "$MANAGEMENTD_DEB")"
  scp_to_vm "$SIGNALD_DEB" mgmt-1 "${REMOTE_DEB_DIR}/$(basename "$SIGNALD_DEB")"
  scp_to_vm "$RELAYD_DEB" mgmt-1 "${REMOTE_DEB_DIR}/$(basename "$RELAYD_DEB")"
  scp_to_vm "$CLIENT_DEB" client-a "${REMOTE_DEB_DIR}/$(basename "$CLIENT_DEB")"
  scp_to_vm "$CLIENT_DEB" client-b "${REMOTE_DEB_DIR}/$(basename "$CLIENT_DEB")"
}

copy_configs() {
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/config/meshlink-managementd.env" mgmt-1 "${REMOTE_ROOT}/config/meshlink-managementd.env"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/config/client-a.toml" client-a "${REMOTE_ROOT}/config/client.toml"
  scp_to_vm "$MESHLINK_LAB_STATE_DIR/runtime/config/client-b.toml" client-b "${REMOTE_ROOT}/config/client.toml"
}

reset_remote_state() {
  local node="$1"
  ssh_to_vm "$node" "sudo systemctl stop meshlink-managementd meshlink-client 2>/dev/null || true; sudo systemctl reset-failed meshlink-managementd meshlink-client 2>/dev/null || true"
}

install_management_packages() {
  ssh_to_vm mgmt-1 "sudo dpkg -i ${REMOTE_DEB_DIR}/$(basename "$MANAGEMENTD_DEB") ${REMOTE_DEB_DIR}/$(basename "$SIGNALD_DEB") ${REMOTE_DEB_DIR}/$(basename "$RELAYD_DEB")"
}

install_client_package() {
  local node="$1"
  ssh_to_vm "$node" "sudo dpkg -i ${REMOTE_DEB_DIR}/$(basename "$CLIENT_DEB")"
}

apply_runtime_config() {
  ssh_to_vm mgmt-1 "sudo install -m 0644 ${REMOTE_ROOT}/config/meshlink-managementd.env /etc/default/meshlink-managementd.env"
  ssh_to_vm client-a "sudo install -m 0644 ${REMOTE_ROOT}/config/client.toml /etc/meshlink/client.toml"
  ssh_to_vm client-b "sudo install -m 0644 ${REMOTE_ROOT}/config/client.toml /etc/meshlink/client.toml"
}

start_services() {
  ssh_to_vm mgmt-1 "sudo systemctl daemon-reload && sudo systemctl restart meshlink-managementd"
  ssh_to_vm client-a "sudo ip link del ${MESHLINK_INTERFACE_NAME} 2>/dev/null || true; sudo systemctl daemon-reload && sudo systemctl restart meshlink-client"
  ssh_to_vm client-b "sudo ip link del ${MESHLINK_INTERFACE_NAME} 2>/dev/null || true; sudo systemctl daemon-reload && sudo systemctl restart meshlink-client"
}

assert_remote_command() {
  local node="$1"
  local command="$2"
  ssh_to_vm "$node" "$command"
}

assert_remote_journal() {
  local node="$1"
  local unit="$2"
  local pattern="$3"
  ssh_to_vm "$node" "sudo journalctl -u ${unit} --no-pager -n 200 | grep -q '$pattern'"
}

collect_journal() {
  local node="$1"
  local unit="$2"
  local local_path="$MESHLINK_LAB_STATE_DIR/runtime/${node}-${unit}.log"
  ssh_to_vm "$node" "sudo journalctl -u ${unit} --no-pager -n 200" >"$local_path"
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

wait_for_ssh mgmt-1
wait_for_management_tools mgmt-1
wait_for_cloud_init mgmt-1

for node in client-a client-b; do
  wait_for_ssh "$node"
  wait_for_client_system_tools "$node"
  wait_for_cloud_init "$node"
  assert_client_runtime_dependencies "$node"
done

ensure_deb_artifacts

MGMT_IP="$(resolved_vm_ip mgmt-1)"
CLIENT_A_IP="$(resolved_vm_ip client-a)"
CLIENT_B_IP="$(resolved_vm_ip client-b)"

generate_wireguard_keys
write_management_env
write_client_config client-a "$CLIENT_A_PUBLIC_KEY" "$CLIENT_A_PRIVATE_KEY" "$CLIENT_A_IP" "$MESHLINK_CLIENT_A_WG_PORT"
write_client_config client-b "$CLIENT_B_PUBLIC_KEY" "$CLIENT_B_PRIVATE_KEY" "$CLIENT_B_IP" "$MESHLINK_CLIENT_B_WG_PORT"

for node in mgmt-1 client-a client-b; do
  prepare_remote_dirs "$node"
  reset_remote_state "$node"
done

copy_deb_packages
copy_configs

install_management_packages
install_client_package client-a
install_client_package client-b

apply_runtime_config
start_services
sleep 10

assert_remote_command mgmt-1 "dpkg -s meshlink-managementd >/dev/null"
assert_remote_command mgmt-1 "dpkg -s meshlink-signald >/dev/null"
assert_remote_command mgmt-1 "dpkg -s meshlink-relayd >/dev/null"
assert_remote_command client-a "dpkg -s meshlink-client >/dev/null"
assert_remote_command client-b "dpkg -s meshlink-client >/dev/null"

assert_remote_command mgmt-1 "test -x /usr/bin/signald && test -x /usr/bin/relayd"
assert_remote_command mgmt-1 "test -f /etc/default/meshlink-signald.env && test -f /etc/default/meshlink-relayd.env"

assert_remote_command mgmt-1 "sudo systemctl status meshlink-managementd --no-pager >/dev/null"
assert_remote_command client-a "sudo systemctl status meshlink-client --no-pager >/dev/null"
assert_remote_command client-b "sudo systemctl status meshlink-client --no-pager >/dev/null"

assert_remote_journal mgmt-1 "meshlink-managementd" "managementd listening on 0.0.0.0:${MESHLINK_MANAGEMENT_PORT}"
assert_remote_journal client-a "meshlink-client" "device registered"
assert_remote_journal client-b "meshlink-client" "device registered"
assert_remote_journal client-a "meshlink-client" "wireguard state reconciled"
assert_remote_journal client-b "meshlink-client" "wireguard state reconciled"

assert_remote_command client-a "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'peer: ${CLIENT_B_PUBLIC_KEY}'"
assert_remote_command client-b "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'peer: ${CLIENT_A_PUBLIC_KEY}'"
assert_remote_command client-a "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'endpoint: ${CLIENT_B_IP}:${MESHLINK_CLIENT_B_WG_PORT}'"
assert_remote_command client-b "sudo wg show ${MESHLINK_INTERFACE_NAME} | grep -q 'endpoint: ${CLIENT_A_IP}:${MESHLINK_CLIENT_A_WG_PORT}'"

assert_remote_command client-a "sudo ping -c 2 -W 2 ${CLIENT_B_OVERLAY} >/dev/null"
assert_remote_command client-b "sudo ping -c 2 -W 2 ${CLIENT_A_OVERLAY} >/dev/null"

collect_journal mgmt-1 "meshlink-managementd"
collect_journal client-a "meshlink-client"
collect_journal client-b "meshlink-client"
collect_wg_state client-a
collect_wg_state client-b

echo "vm lab phase03 deb acceptance passed"
