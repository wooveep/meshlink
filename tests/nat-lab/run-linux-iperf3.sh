#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_commands virsh ssh jq ping

MESHLINK_INTERFACE_NAME="${MESHLINK_INTERFACE_NAME:-sdwan0}"
MESHLINK_LINUX_IPERF_DURATION="${MESHLINK_LINUX_IPERF_DURATION:-300}"
MESHLINK_LINUX_IPERF_PORT="${MESHLINK_LINUX_IPERF_PORT:-5201}"
MESHLINK_LINUX_IPERF_TIMEOUT="${MESHLINK_LINUX_IPERF_TIMEOUT:-$((MESHLINK_LINUX_IPERF_DURATION + 30))}"
MESHLINK_LINUX_IPERF_ARTIFACT_DIR="${MESHLINK_LINUX_IPERF_ARTIFACT_DIR:-${MESHLINK_LAB_STATE_DIR}/linux-linux-iperf3}"

cleanup_iperf3() {
  for node in client-a client-b; do
    ssh_to_vm "$node" "sudo pkill -x iperf3 >/dev/null 2>&1 || true" >/dev/null 2>&1 || true
  done
}
trap cleanup_iperf3 EXIT

ensure_guest_iperf3() {
  local node="$1"

  if ssh_to_vm "$node" "command -v iperf3 >/dev/null 2>&1" >/dev/null 2>&1; then
    return
  fi

  ssh_to_vm "$node" "sudo apt-get update && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y iperf3"
}

overlay_ip() {
  local node="$1"
  ssh_to_vm "$node" "ip -4 -o addr show ${MESHLINK_INTERFACE_NAME} | awk '{split(\$4,a,\"/\"); print a[1]}'" | tr -d '\r\n'
}

run_iperf3_once() {
  local client_node="$1"
  local server_node="$2"
  local server_ip="$3"
  local json_name="$4"
  local server_log_name="$5"

  ssh_to_vm "$server_node" "rm -f /tmp/meshlink-iperf3-server.log; nohup iperf3 -s -1 -p ${MESHLINK_LINUX_IPERF_PORT} > /tmp/meshlink-iperf3-server.log 2>&1 < /dev/null &"
  sleep 2
  ssh_to_vm "$client_node" "timeout ${MESHLINK_LINUX_IPERF_TIMEOUT} iperf3 -c ${server_ip} -p ${MESHLINK_LINUX_IPERF_PORT} -t ${MESHLINK_LINUX_IPERF_DURATION} --json" >"${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}/${json_name}"
  ssh_to_vm "$server_node" "cat /tmp/meshlink-iperf3-server.log" >"${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}/${server_log_name}" || true
}

if [[ "${MESHLINK_LINUX_IPERF_SKIP_PHASE05:-0}" != "1" ]]; then
  MESHLINK_CLIENT_RUNTIME_TIMEOUT="${MESHLINK_CLIENT_RUNTIME_TIMEOUT:-$((MESHLINK_LINUX_IPERF_DURATION * 2 + 360))s}" "$SCRIPT_DIR/run-phase05.sh"
fi

mkdir -p "$MESHLINK_LINUX_IPERF_ARTIFACT_DIR"
rm -f "${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}"/*

ensure_guest_iperf3 client-a
ensure_guest_iperf3 client-b

CLIENT_A_OVERLAY="$(overlay_ip client-a)"
CLIENT_B_OVERLAY="$(overlay_ip client-b)"

echo "client-a overlay=${CLIENT_A_OVERLAY}"
echo "client-b overlay=${CLIENT_B_OVERLAY}"

ssh_to_vm client-a "ping -c 3 -W 2 ${CLIENT_B_OVERLAY}" | tee "${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}/ping-client-a-to-client-b.txt"
ssh_to_vm client-b "ping -c 3 -W 2 ${CLIENT_A_OVERLAY}" | tee "${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}/ping-client-b-to-client-a.txt"

cleanup_iperf3
run_iperf3_once client-a client-b "$CLIENT_B_OVERLAY" client-a-to-client-b.json client-b-server-a-to-b.log
run_iperf3_once client-b client-a "$CLIENT_A_OVERLAY" client-b-to-client-a.json client-a-server-b-to-a.log

printf '%s\n' \
  "client-a=${CLIENT_A_OVERLAY}" \
  "client-b=${CLIENT_B_OVERLAY}" \
  "duration=${MESHLINK_LINUX_IPERF_DURATION}" \
  "port=${MESHLINK_LINUX_IPERF_PORT}" \
  >"${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}/summary.env"

for json in client-a-to-client-b.json client-b-to-client-a.json; do
  echo "--- ${json} ---"
  jq -r '"receiver_mbps=" + ((.end.sum_received.bits_per_second / 1000000)|tostring) + " sender_mbps=" + ((.end.sum_sent.bits_per_second / 1000000)|tostring) + " retransmits=" + (.end.sum_sent.retransmits|tostring) + " seconds=" + (.end.sum_received.seconds|tostring)' "${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}/${json}"
done

echo "artifacts=${MESHLINK_LINUX_IPERF_ARTIFACT_DIR}"
