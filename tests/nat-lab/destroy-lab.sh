#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_commands virsh

IMAGE_DIR="$(image_dir)"

destroy_vm() {
  local node="$1"
  local name

  name="$(vm_name "$node")"

  if virsh dominfo "$name" >/dev/null 2>&1; then
    virsh destroy "$name" >/dev/null 2>&1 || true
    virsh undefine "$name" --managed-save --snapshots-metadata --nvram >/dev/null 2>&1 || virsh undefine "$name" >/dev/null 2>&1 || true
  fi

  if [[ -n "${IMAGE_DIR:-}" ]]; then
    remove_file "$IMAGE_DIR/${name}.qcow2"
  fi
}

while IFS= read -r node; do
  destroy_vm "$node"
done <<'EOF'
mgmt-1
nat-a
nat-b
client-a
client-b
EOF

clear_phase06_drop_rules || true
clear_dual_nat_host_isolation || true
destroy_isolated_network "$MESHLINK_NAT_A_NETWORK_NAME"
destroy_isolated_network "$MESHLINK_NAT_B_NETWORK_NAME"

rm -rf "$MESHLINK_LAB_STATE_DIR"

echo "meshlink nat lab destroyed"
