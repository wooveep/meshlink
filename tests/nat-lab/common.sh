#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LAB_ENV_FILE="${MESHLINK_LAB_ENV:-$ROOT_DIR/tests/nat-lab/libvirt.env}"
MESHLINK_LAB_PREFIX="${MESHLINK_LAB_PREFIX:-meshlink}"
MESHLINK_LIBVIRT_NETWORK="${MESHLINK_LIBVIRT_NETWORK:-default}"
MESHLINK_LIBVIRT_POOL="${MESHLINK_LIBVIRT_POOL:-default}"
MESHLINK_OS_VARIANT="${MESHLINK_OS_VARIANT:-ubuntu22.04}"
MESHLINK_VM_MEMORY_MB="${MESHLINK_VM_MEMORY_MB:-2048}"
MESHLINK_VM_VCPUS="${MESHLINK_VM_VCPUS:-2}"
MESHLINK_BASE_IMAGE="${MESHLINK_BASE_IMAGE:-}"
MESHLINK_SSH_USER="${MESHLINK_SSH_USER:-meshlink}"
MESHLINK_SSH_PUBLIC_KEY="${MESHLINK_SSH_PUBLIC_KEY:-}"
MESHLINK_SSH_PUBLIC_KEY_FILE="${MESHLINK_SSH_PUBLIC_KEY_FILE:-}"
MESHLINK_SSH_PASSWORD="${MESHLINK_SSH_PASSWORD:-meshlink-lab}"
MESHLINK_LAB_STATE_DIR="${MESHLINK_LAB_STATE_DIR:-/tmp/meshlink-nat-lab}"
MESHLINK_MGMT_IP="${MESHLINK_MGMT_IP:-192.168.122.201}"
MESHLINK_CLIENT_A_IP="${MESHLINK_CLIENT_A_IP:-192.168.122.202}"
MESHLINK_CLIENT_B_IP="${MESHLINK_CLIENT_B_IP:-192.168.122.203}"
MESHLINK_NET_PREFIX="${MESHLINK_NET_PREFIX:-24}"
MESHLINK_GATEWAY="${MESHLINK_GATEWAY:-192.168.122.1}"
MESHLINK_DNS_1="${MESHLINK_DNS_1:-1.1.1.1}"
MESHLINK_DNS_2="${MESHLINK_DNS_2:-8.8.8.8}"
MESHLINK_MANAGEMENT_PORT="${MESHLINK_MANAGEMENT_PORT:-33073}"
MESHLINK_SYNC_INTERVAL="${MESHLINK_SYNC_INTERVAL:-1s}"

load_lab_env() {
  if [[ -f "$LAB_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$LAB_ENV_FILE"
  fi
}

require_commands() {
  local missing=()
  for command_name in "$@"; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
      missing+=("$command_name")
    fi
  done

  if [[ "${#missing[@]}" -gt 0 ]]; then
    echo "missing required commands: ${missing[*]}" >&2
    exit 1
  fi
}

pool_path() {
  virsh pool-dumpxml "$MESHLINK_LIBVIRT_POOL" | sed -n 's:.*<path>\(.*\)</path>.*:\1:p' | head -n1
}

vm_name() {
  printf '%s-%s\n' "$MESHLINK_LAB_PREFIX" "$1"
}

vm_ip() {
  case "$1" in
    mgmt-1) printf '%s\n' "$MESHLINK_MGMT_IP" ;;
    client-a) printf '%s\n' "$MESHLINK_CLIENT_A_IP" ;;
    client-b) printf '%s\n' "$MESHLINK_CLIENT_B_IP" ;;
    *)
      echo "unknown node $1" >&2
      exit 1
      ;;
  esac
}

resolved_vm_ip() {
  local node="$1"
  local name
  name="$(vm_name "$node")"

  local detected
  detected="$(virsh domifaddr "$name" --source lease 2>/dev/null | awk '/ipv4/ { sub(/\/.*/, "", $4); print $4; exit }')"
  if [[ -n "$detected" ]]; then
    printf '%s\n' "$detected"
    return
  fi

  local mac
  mac="$(virsh dumpxml "$name" 2>/dev/null | sed -n "s/.*<mac address='\([^']*\)'.*/\1/p" | head -n1)"
  if [[ -n "$mac" ]]; then
    detected="$(virsh net-dhcp-leases "$MESHLINK_LIBVIRT_NETWORK" 2>/dev/null | awk -v target_mac="$mac" '$2 == target_mac { sub(/\/.*/, "", $5); print $5; exit }')"
    if [[ -n "$detected" ]]; then
      printf '%s\n' "$detected"
      return
    fi
  fi

  return 1
}

ensure_ssh_public_key() {
  if [[ -n "$MESHLINK_SSH_PUBLIC_KEY" ]]; then
    printf '%s\n' "$MESHLINK_SSH_PUBLIC_KEY"
    return
  fi

  if [[ -n "$MESHLINK_SSH_PUBLIC_KEY_FILE" && -f "$MESHLINK_SSH_PUBLIC_KEY_FILE" ]]; then
    cat "$MESHLINK_SSH_PUBLIC_KEY_FILE"
    return
  fi

  if [[ -f "$HOME/.ssh/id_ed25519.pub" ]]; then
    cat "$HOME/.ssh/id_ed25519.pub"
    return
  fi

  if [[ -f "$HOME/.ssh/id_rsa.pub" ]]; then
    cat "$HOME/.ssh/id_rsa.pub"
    return
  fi

  echo "no SSH public key found; set MESHLINK_SSH_PUBLIC_KEY or MESHLINK_SSH_PUBLIC_KEY_FILE" >&2
  exit 1
}

ssh_opts() {
  printf '%s\n' \
    "-o" "StrictHostKeyChecking=no" \
    "-o" "UserKnownHostsFile=/dev/null" \
    "-o" "ConnectTimeout=5"
}

with_askpass() {
  local password="$1"
  shift

  local askpass_script
  askpass_script="$(mktemp)"
  cat >"$askpass_script" <<EOF
#!/usr/bin/env bash
printf '%s\n' '$password'
EOF
  chmod 700 "$askpass_script"

  DISPLAY=dummy SSH_ASKPASS="$askpass_script" SSH_ASKPASS_REQUIRE=force setsid -w "$@"
  local status=$?
  rm -f "$askpass_script"
  return $status
}

ssh_to_vm() {
  local node="$1"
  shift
  local ip
  ip="$(resolved_vm_ip "$node")" || return 1
  with_askpass "$MESHLINK_SSH_PASSWORD" ssh $(ssh_opts) -o PreferredAuthentications=password -o PubkeyAuthentication=no "${MESHLINK_SSH_USER}@${ip}" "$@"
}

scp_to_vm() {
  local source_path="$1"
  local node="$2"
  local target_path="$3"
  local ip
  ip="$(resolved_vm_ip "$node")" || return 1
  with_askpass "$MESHLINK_SSH_PASSWORD" scp $(ssh_opts) -o PreferredAuthentications=password -o PubkeyAuthentication=no "$source_path" "${MESHLINK_SSH_USER}@${ip}:$target_path"
}

scp_from_vm() {
  local node="$1"
  local source_path="$2"
  local target_path="$3"
  local ip
  ip="$(resolved_vm_ip "$node")" || return 1
  with_askpass "$MESHLINK_SSH_PASSWORD" scp $(ssh_opts) -o PreferredAuthentications=password -o PubkeyAuthentication=no "${MESHLINK_SSH_USER}@${ip}:$source_path" "$target_path"
}

wait_for_ssh() {
  local node="$1"
  local attempts="${2:-60}"
  local attempt=1

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" true >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  local ip="unknown"
  if detected_ip="$(resolved_vm_ip "$node" 2>/dev/null)"; then
    ip="$detected_ip"
  fi
  echo "timed out waiting for SSH on $node ($ip)" >&2
  return 1
}

write_cloud_init_files() {
  local node="$1"
  local node_dir="$MESHLINK_LAB_STATE_DIR/$node"
  local ssh_key
  ssh_key="$(ensure_ssh_public_key)"

  mkdir -p "$node_dir"

  cat >"$node_dir/user-data" <<EOF
#cloud-config
hostname: $(vm_name "$node")
manage_etc_hosts: true
users:
  - default
  - name: $MESHLINK_SSH_USER
    groups: [sudo]
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
      - $ssh_key
package_update: true
packages:
  - wireguard-tools
  - iputils-ping
ssh_pwauth: true
chpasswd:
  list: |
    $MESHLINK_SSH_USER:$MESHLINK_SSH_PASSWORD
  expire: false
runcmd:
  - mkdir -p /home/$MESHLINK_SSH_USER/meshlink/bin /home/$MESHLINK_SSH_USER/meshlink/config /home/$MESHLINK_SSH_USER/meshlink/logs
  - chown -R $MESHLINK_SSH_USER:$MESHLINK_SSH_USER /home/$MESHLINK_SSH_USER/meshlink
EOF

  cat >"$node_dir/meta-data" <<EOF
instance-id: $(vm_name "$node")
local-hostname: $(vm_name "$node")
EOF

  : >"$node_dir/network-config"
}

build_seed_iso() {
  local node="$1"
  local node_dir="$MESHLINK_LAB_STATE_DIR/$node"
  local seed_iso="$node_dir/seed.iso"

  rm -f "$seed_iso"
  xorriso -as mkisofs \
    -output "$seed_iso" \
    -volid cidata \
    -joliet \
    -rock \
    -graft-points \
    "user-data=$node_dir/user-data" \
    "meta-data=$node_dir/meta-data" \
    >/dev/null 2>&1
}
