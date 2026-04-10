#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_commands virsh

POOL_PATH="$(pool_path)"

destroy_vm() {
  local node="$1"
  local name
  name="$(vm_name "$node")"

  if virsh dominfo "$name" >/dev/null 2>&1; then
    virsh destroy "$name" >/dev/null 2>&1 || true
    virsh undefine "$name" --managed-save --snapshots-metadata --nvram >/dev/null 2>&1 || virsh undefine "$name" >/dev/null 2>&1 || true
  fi

  if [[ -n "${POOL_PATH:-}" ]]; then
    rm -f "$POOL_PATH/${name}.qcow2"
  fi
}

destroy_vm mgmt-1
destroy_vm client-a
destroy_vm client-b

rm -rf "$MESHLINK_LAB_STATE_DIR"

echo "meshlink nat lab destroyed"
