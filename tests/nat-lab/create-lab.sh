#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/nat-lab/common.sh
source "$SCRIPT_DIR/common.sh"

load_lab_env
require_commands virsh virt-install qemu-img xorriso

if [[ -z "$MESHLINK_BASE_IMAGE" ]]; then
  echo "MESHLINK_BASE_IMAGE is required. Copy tests/nat-lab/libvirt.env.example to tests/nat-lab/libvirt.env and set it." >&2
  exit 1
fi

if [[ ! -f "$MESHLINK_BASE_IMAGE" ]]; then
  echo "base image not found: $MESHLINK_BASE_IMAGE" >&2
  exit 1
fi

POOL_PATH="$(pool_path)"
if [[ -z "$POOL_PATH" ]]; then
  echo "failed to resolve pool path for $MESHLINK_LIBVIRT_POOL" >&2
  exit 1
fi

mkdir -p "$MESHLINK_LAB_STATE_DIR"

create_vm() {
  local node="$1"
  local name
  name="$(vm_name "$node")"
  local disk_path="$POOL_PATH/${name}.qcow2"
  local ssh_key_file
  ssh_key_file="${MESHLINK_SSH_PUBLIC_KEY_FILE:-}"

  if [[ -z "$ssh_key_file" ]]; then
    echo "MESHLINK_SSH_PUBLIC_KEY_FILE is required for cloud-init SSH access" >&2
    exit 1
  fi
  if [[ ! -f "$ssh_key_file" ]]; then
    echo "SSH public key file not found: $ssh_key_file" >&2
    exit 1
  fi

  if virsh dominfo "$name" >/dev/null 2>&1; then
    echo "vm already exists: $name"
    return
  fi

  write_cloud_init_files "$node"
  build_seed_iso "$node"
  qemu-img create -f qcow2 -F qcow2 -b "$MESHLINK_BASE_IMAGE" "$disk_path" >/dev/null

  virt-install \
    --name "$name" \
    --memory "$MESHLINK_VM_MEMORY_MB" \
    --vcpus "$MESHLINK_VM_VCPUS" \
    --import \
    --os-variant "$MESHLINK_OS_VARIANT" \
    --disk "path=$disk_path,format=qcow2,bus=virtio" \
    --disk "path=$MESHLINK_LAB_STATE_DIR/$node/seed.iso,device=cdrom" \
    --network "network=$MESHLINK_LIBVIRT_NETWORK,model=virtio" \
    --graphics none \
    --noautoconsole

  echo "created $name at $(vm_ip "$node")"
}

create_vm mgmt-1
create_vm client-a
create_vm client-b

echo "meshlink nat lab created"
