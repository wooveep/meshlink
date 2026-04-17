#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WINDOWS_ENV_FILE="${MESHLINK_WINDOWS_ENV:-$ROOT_DIR/tests/windows-vm/windows.env}"

# shellcheck disable=SC1091
source "$ROOT_DIR/tests/nat-lab/common.sh"

load_windows_env() {
  if [[ -f "$WINDOWS_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$WINDOWS_ENV_FILE"
  fi
}

resolve_windows_network() {
  case "${MESHLINK_WINDOWS_ATTACH_NETWORK:-nat-a-lan}" in
    upstream)
      printf '%s\n' "$MESHLINK_UPSTREAM_NETWORK"
      ;;
    nat-a-lan)
      printf '%s\n' "$MESHLINK_NAT_A_NETWORK_NAME"
      ;;
    nat-b-lan)
      printf '%s\n' "$MESHLINK_NAT_B_NETWORK_NAME"
      ;;
    *)
      echo "unsupported MESHLINK_WINDOWS_ATTACH_NETWORK: ${MESHLINK_WINDOWS_ATTACH_NETWORK:-}" >&2
      exit 1
      ;;
  esac
}

default_static_ip() {
  case "${MESHLINK_WINDOWS_ATTACH_NETWORK:-nat-a-lan}" in
    upstream) printf '%s\n' "$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 230)" ;;
    nat-a-lan) printf '%s\n' "10.10.1.20" ;;
    nat-b-lan) printf '%s\n' "10.10.2.20" ;;
  esac
}

default_gateway() {
  case "${MESHLINK_WINDOWS_ATTACH_NETWORK:-nat-a-lan}" in
    upstream) printf '%s\n' "$MESHLINK_GATEWAY" ;;
    nat-a-lan) printf '%s\n' "$MESHLINK_NAT_A_LAN_IP" ;;
    nat-b-lan) printf '%s\n' "$MESHLINK_NAT_B_LAN_IP" ;;
  esac
}

ensure_iso_with_package() {
  local package_path="$1"
  local output_iso="$2"
  local staging_dir

  if [[ ! -f "$package_path" ]]; then
    echo "package file does not exist: $package_path" >&2
    exit 1
  fi
  require_commands xorriso

  staging_dir="$(mktemp -d)"
  trap "rm -rf '$staging_dir'" EXIT

  cp "$package_path" "$staging_dir/"
  xorriso -as mkisofs -quiet -o "$output_iso" "$staging_dir"
}

run_qemu_img() {
  if [[ "${MESHLINK_WINDOWS_QEMU_IMG_USE_SUDO:-}" == "1" ]]; then
    sudo qemu-img "$@"
  else
    qemu-img "$@"
  fi
}

load_lab_env
load_windows_env

require_commands virsh virt-install qemu-img

MESHLINK_WINDOWS_VM_NAME="${MESHLINK_WINDOWS_VM_NAME:-meshlink-win-a}"
MESHLINK_WINDOWS_OS_VARIANT="${MESHLINK_WINDOWS_OS_VARIANT:-win11}"
MESHLINK_WINDOWS_MEMORY_MB="${MESHLINK_WINDOWS_MEMORY_MB:-4096}"
MESHLINK_WINDOWS_VCPUS="${MESHLINK_WINDOWS_VCPUS:-4}"
MESHLINK_WINDOWS_DISK_GB="${MESHLINK_WINDOWS_DISK_GB:-64}"
MESHLINK_WINDOWS_ISO="${MESHLINK_WINDOWS_ISO:-}"
MESHLINK_WINDOWS_BASE_DISK="${MESHLINK_WINDOWS_BASE_DISK:-}"
MESHLINK_WINDOWS_CLONE_MODE="${MESHLINK_WINDOWS_CLONE_MODE:-backing}"
MESHLINK_WINDOWS_PACKAGE="${MESHLINK_WINDOWS_PACKAGE:-}"
MESHLINK_WINDOWS_VIRTIO_ISO="${MESHLINK_WINDOWS_VIRTIO_ISO:-}"
MESHLINK_WINDOWS_ENABLE_QEMU_AGENT="${MESHLINK_WINDOWS_ENABLE_QEMU_AGENT:-1}"
MESHLINK_WINDOWS_STATIC_IP="${MESHLINK_WINDOWS_STATIC_IP:-$(default_static_ip)}"
MESHLINK_WINDOWS_PREFIX="${MESHLINK_WINDOWS_PREFIX:-24}"
MESHLINK_WINDOWS_GATEWAY="${MESHLINK_WINDOWS_GATEWAY:-$(default_gateway)}"
MESHLINK_WINDOWS_DNS_1="${MESHLINK_WINDOWS_DNS_1:-$MESHLINK_DNS_1}"
MESHLINK_WINDOWS_DNS_2="${MESHLINK_WINDOWS_DNS_2:-$MESHLINK_DNS_2}"

if virsh dominfo "$MESHLINK_WINDOWS_VM_NAME" >/dev/null 2>&1; then
  echo "VM already exists: $MESHLINK_WINDOWS_VM_NAME" >&2
  exit 1
fi

POOL_PATH="$(pool_path)"
if [[ -z "$POOL_PATH" ]]; then
  echo "failed to resolve libvirt pool path for $MESHLINK_LIBVIRT_POOL" >&2
  exit 1
fi

NETWORK_NAME="$(resolve_windows_network)"
WINDOWS_DISK_PATH="$POOL_PATH/${MESHLINK_WINDOWS_VM_NAME}.qcow2"
WINDOWS_STATE_DIR="$MESHLINK_LAB_STATE_DIR/windows/$MESHLINK_WINDOWS_VM_NAME"
PACKAGE_ISO_PATH="$WINDOWS_STATE_DIR/meshlink-package.iso"
BOOT_ARGS=()

if [[ -e "$WINDOWS_DISK_PATH" ]]; then
  echo "target disk already exists: $WINDOWS_DISK_PATH" >&2
  exit 1
fi

mkdir -p "$WINDOWS_STATE_DIR"

if [[ -n "$MESHLINK_WINDOWS_BASE_DISK" ]]; then
  if [[ ! -f "$MESHLINK_WINDOWS_BASE_DISK" ]]; then
    echo "base disk does not exist: $MESHLINK_WINDOWS_BASE_DISK" >&2
    exit 1
  fi

  if [[ ! -r "$MESHLINK_WINDOWS_BASE_DISK" ]]; then
    MESHLINK_WINDOWS_QEMU_IMG_USE_SUDO=1
  fi

  case "$MESHLINK_WINDOWS_CLONE_MODE" in
    backing)
      run_qemu_img create -f qcow2 -F qcow2 -b "$MESHLINK_WINDOWS_BASE_DISK" "$WINDOWS_DISK_PATH" >/dev/null
      ;;
    copy)
      run_qemu_img convert -p -O qcow2 "$MESHLINK_WINDOWS_BASE_DISK" "$WINDOWS_DISK_PATH"
      ;;
    *)
      echo "unsupported MESHLINK_WINDOWS_CLONE_MODE: $MESHLINK_WINDOWS_CLONE_MODE" >&2
      exit 1
      ;;
  esac

  if [[ "${MESHLINK_WINDOWS_QEMU_IMG_USE_SUDO:-}" == "1" ]]; then
    sudo chown "$(id -un):$(id -gn)" "$WINDOWS_DISK_PATH"
  fi

  BOOT_ARGS+=(--import)
else
  if [[ -z "$MESHLINK_WINDOWS_ISO" ]]; then
    echo "MESHLINK_WINDOWS_ISO is required when MESHLINK_WINDOWS_BASE_DISK is unset" >&2
    exit 1
  fi

  run_qemu_img create -f qcow2 "$WINDOWS_DISK_PATH" "${MESHLINK_WINDOWS_DISK_GB}G" >/dev/null
  BOOT_ARGS+=(--cdrom "$MESHLINK_WINDOWS_ISO")
fi

EXTRA_DISK_ARGS=()
if [[ -n "$MESHLINK_WINDOWS_PACKAGE" ]]; then
  ensure_iso_with_package "$MESHLINK_WINDOWS_PACKAGE" "$PACKAGE_ISO_PATH"
  EXTRA_DISK_ARGS+=(--disk "path=$PACKAGE_ISO_PATH,device=cdrom")
fi

if [[ -n "$MESHLINK_WINDOWS_VIRTIO_ISO" ]]; then
  if [[ ! -f "$MESHLINK_WINDOWS_VIRTIO_ISO" ]]; then
    echo "virtio ISO does not exist: $MESHLINK_WINDOWS_VIRTIO_ISO" >&2
    exit 1
  fi
  EXTRA_DISK_ARGS+=(--disk "path=$MESHLINK_WINDOWS_VIRTIO_ISO,device=cdrom")
fi

CHANNEL_ARGS=()
if [[ "$MESHLINK_WINDOWS_ENABLE_QEMU_AGENT" == "1" ]]; then
  CHANNEL_ARGS+=(--channel "unix,target.type=virtio,target.name=org.qemu.guest_agent.0")
fi

virt-install \
  --name "$MESHLINK_WINDOWS_VM_NAME" \
  --memory "$MESHLINK_WINDOWS_MEMORY_MB" \
  --vcpus "$MESHLINK_WINDOWS_VCPUS" \
  --os-variant "$MESHLINK_WINDOWS_OS_VARIANT" \
  --disk "path=$WINDOWS_DISK_PATH,format=qcow2,bus=sata" \
  "${BOOT_ARGS[@]}" \
  "${EXTRA_DISK_ARGS[@]}" \
  --network "network=$NETWORK_NAME,model=e1000e" \
  "${CHANNEL_ARGS[@]}" \
  --graphics spice \
  --video qxl \
  --noautoconsole

cat <<EOF
Windows VM created: $MESHLINK_WINDOWS_VM_NAME
Attached network: $NETWORK_NAME
Recommended static IPv4:
  address: $MESHLINK_WINDOWS_STATIC_IP/$MESHLINK_WINDOWS_PREFIX
  gateway: $MESHLINK_WINDOWS_GATEWAY
  dns: $MESHLINK_WINDOWS_DNS_1, $MESHLINK_WINDOWS_DNS_2

Open the console with:
  virt-viewer $MESHLINK_WINDOWS_VM_NAME

Provisioning mode:
  $([[ -n "$MESHLINK_WINDOWS_BASE_DISK" ]] && printf 'clone from base disk (%s, mode=%s)\n' "$MESHLINK_WINDOWS_BASE_DISK" "$MESHLINK_WINDOWS_CLONE_MODE" || printf 'fresh install from ISO (%s)\n' "$MESHLINK_WINDOWS_ISO")

If a MeshLink package was attached, it is available as a read-only CD inside the guest.
$( [[ -n "$MESHLINK_WINDOWS_VIRTIO_ISO" ]] && printf 'If a virtio ISO was attached, install qemu-ga inside the guest and then use tests/windows-vm/qga.sh from the host.\n' )
EOF
