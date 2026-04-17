#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WINDOWS_ENV_FILE="${MESHLINK_WINDOWS_ENV:-$ROOT_DIR/tests/windows-vm/windows.env}"

load_windows_env() {
  if [[ -f "$WINDOWS_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$WINDOWS_ENV_FILE"
  fi
}

load_windows_env

MESHLINK_WINDOWS_VM_NAME="${MESHLINK_WINDOWS_VM_NAME:-meshlink-win-a}"
MESHLINK_WINDOWS_VIRTIO_ISO="${MESHLINK_WINDOWS_VIRTIO_ISO:-}"

command -v virsh >/dev/null 2>&1 || {
  echo "virsh is required" >&2
  exit 1
}
command -v virt-xml >/dev/null 2>&1 || {
  echo "virt-xml is required" >&2
  exit 1
}

was_running=0
if [[ "$(virsh domstate "$MESHLINK_WINDOWS_VM_NAME" 2>/dev/null | tr '[:upper:]' '[:lower:]')" == "running" ]]; then
  was_running=1
  echo "shutting down $MESHLINK_WINDOWS_VM_NAME to update persistent devices"
  virsh shutdown "$MESHLINK_WINDOWS_VM_NAME" >/dev/null || true
  for _ in $(seq 1 60); do
    state="$(virsh domstate "$MESHLINK_WINDOWS_VM_NAME" | tr '[:upper:]' '[:lower:]')"
    if [[ "$state" == "shut off" ]]; then
      break
    fi
    sleep 2
  done
fi

state="$(virsh domstate "$MESHLINK_WINDOWS_VM_NAME" | tr '[:upper:]' '[:lower:]')"
if [[ "$state" != "shut off" ]]; then
  echo "domain must be shut off before editing devices: $MESHLINK_WINDOWS_VM_NAME ($state)" >&2
  exit 1
fi

if ! virsh dumpxml "$MESHLINK_WINDOWS_VM_NAME" | grep -q "org.qemu.guest_agent.0"; then
  virt-xml "$MESHLINK_WINDOWS_VM_NAME" \
    --add-device \
    --channel "unix,target.type=virtio,target.name=org.qemu.guest_agent.0" >/dev/null
  echo "added qemu guest agent channel to $MESHLINK_WINDOWS_VM_NAME"
else
  echo "qemu guest agent channel already present on $MESHLINK_WINDOWS_VM_NAME"
fi

if [[ -n "$MESHLINK_WINDOWS_VIRTIO_ISO" ]]; then
  if [[ ! -f "$MESHLINK_WINDOWS_VIRTIO_ISO" ]]; then
    echo "virtio ISO does not exist: $MESHLINK_WINDOWS_VIRTIO_ISO" >&2
    exit 1
  fi
  if ! virsh dumpxml "$MESHLINK_WINDOWS_VM_NAME" | grep -Fq "$MESHLINK_WINDOWS_VIRTIO_ISO"; then
    virt-xml "$MESHLINK_WINDOWS_VM_NAME" \
      --add-device \
      --disk "path=$MESHLINK_WINDOWS_VIRTIO_ISO,device=cdrom" >/dev/null
    echo "attached virtio ISO to $MESHLINK_WINDOWS_VM_NAME"
  else
    echo "virtio ISO already attached to $MESHLINK_WINDOWS_VM_NAME"
  fi
fi

if (( was_running )); then
  virsh start "$MESHLINK_WINDOWS_VM_NAME" >/dev/null
  echo "restarted $MESHLINK_WINDOWS_VM_NAME"
fi
