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

if [[ "$MESHLINK_LAB_TOPOLOGY" == "dual-nat" ]]; then
  ensure_isolated_network "$MESHLINK_NAT_A_NETWORK_NAME" "$MESHLINK_NAT_A_BRIDGE_NAME" "$MESHLINK_NAT_A_LAN_HOST_IP" "$MESHLINK_NAT_A_LAN_CIDR"
  ensure_isolated_network "$MESHLINK_NAT_B_NETWORK_NAME" "$MESHLINK_NAT_B_BRIDGE_NAME" "$MESHLINK_NAT_B_LAN_HOST_IP" "$MESHLINK_NAT_B_LAN_CIDR"
fi

create_vm() {
  local node="$1"
  local name
  local disk_path
  local -a network_args

  name="$(vm_name "$node")"
  disk_path="$POOL_PATH/${name}.qcow2"
  network_args=()

  if virsh dominfo "$name" >/dev/null 2>&1; then
    echo "vm already exists: $name"
    return
  fi

  case "$MESHLINK_LAB_TOPOLOGY:$node" in
    flat:mgmt-1|flat:client-a|flat:client-b)
      network_args+=(--network "network=${MESHLINK_UPSTREAM_NETWORK},model=virtio,mac=$(vm_mac "$node")")
      ;;
    dual-nat:mgmt-1)
      network_args+=(--network "network=${MESHLINK_UPSTREAM_NETWORK},model=virtio,mac=$(vm_mac "$node")")
      ;;
    dual-nat:nat-a)
      network_args+=(--network "network=${MESHLINK_UPSTREAM_NETWORK},model=virtio,mac=$(vm_mac "$node" primary)")
      network_args+=(--network "network=${MESHLINK_NAT_A_NETWORK_NAME},model=virtio,mac=$(vm_mac "$node" secondary)")
      ;;
    dual-nat:nat-b)
      network_args+=(--network "network=${MESHLINK_UPSTREAM_NETWORK},model=virtio,mac=$(vm_mac "$node" primary)")
      network_args+=(--network "network=${MESHLINK_NAT_B_NETWORK_NAME},model=virtio,mac=$(vm_mac "$node" secondary)")
      ;;
    dual-nat:client-a)
      network_args+=(--network "network=${MESHLINK_NAT_A_NETWORK_NAME},model=virtio,mac=$(vm_mac "$node")")
      ;;
    dual-nat:client-b)
      network_args+=(--network "network=${MESHLINK_NAT_B_NETWORK_NAME},model=virtio,mac=$(vm_mac "$node")")
      ;;
    *)
      echo "unsupported create target ${MESHLINK_LAB_TOPOLOGY}:${node}" >&2
      exit 1
      ;;
  esac

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
    "${network_args[@]}" \
    --graphics none \
    --noautoconsole

  echo "created $name at $(vm_ip "$node")"
}

while IFS= read -r node; do
  create_vm "$node"
done < <(lab_nodes)

echo "meshlink nat lab created (${MESHLINK_LAB_TOPOLOGY})"
