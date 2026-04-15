#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LAB_ENV_FILE="${MESHLINK_LAB_ENV:-$ROOT_DIR/tests/nat-lab/libvirt.env}"
MESHLINK_LAB_PREFIX="${MESHLINK_LAB_PREFIX:-meshlink}"
MESHLINK_LAB_TOPOLOGY="${MESHLINK_LAB_TOPOLOGY:-}"
MESHLINK_LIBVIRT_NETWORK="${MESHLINK_LIBVIRT_NETWORK:-default}"
MESHLINK_UPSTREAM_NETWORK="${MESHLINK_UPSTREAM_NETWORK:-}"
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
MESHLINK_MGMT_IP="${MESHLINK_MGMT_IP:-}"
MESHLINK_CLIENT_A_IP="${MESHLINK_CLIENT_A_IP:-}"
MESHLINK_CLIENT_B_IP="${MESHLINK_CLIENT_B_IP:-}"
MESHLINK_NAT_A_WAN_IP="${MESHLINK_NAT_A_WAN_IP:-}"
MESHLINK_NAT_A_LAN_IP="${MESHLINK_NAT_A_LAN_IP:-}"
MESHLINK_NAT_B_WAN_IP="${MESHLINK_NAT_B_WAN_IP:-}"
MESHLINK_NAT_B_LAN_IP="${MESHLINK_NAT_B_LAN_IP:-}"
MESHLINK_NAT_A_LAN_CIDR="${MESHLINK_NAT_A_LAN_CIDR:-}"
MESHLINK_NAT_B_LAN_CIDR="${MESHLINK_NAT_B_LAN_CIDR:-}"
MESHLINK_NAT_A_LAN_HOST_IP="${MESHLINK_NAT_A_LAN_HOST_IP:-}"
MESHLINK_NAT_B_LAN_HOST_IP="${MESHLINK_NAT_B_LAN_HOST_IP:-}"
MESHLINK_NAT_A_NETWORK_NAME="${MESHLINK_NAT_A_NETWORK_NAME:-}"
MESHLINK_NAT_B_NETWORK_NAME="${MESHLINK_NAT_B_NETWORK_NAME:-}"
MESHLINK_NAT_A_BRIDGE_NAME="${MESHLINK_NAT_A_BRIDGE_NAME:-}"
MESHLINK_NAT_B_BRIDGE_NAME="${MESHLINK_NAT_B_BRIDGE_NAME:-}"
MESHLINK_NET_PREFIX="${MESHLINK_NET_PREFIX:-24}"
MESHLINK_GATEWAY="${MESHLINK_GATEWAY:-}"
MESHLINK_DNS_1="${MESHLINK_DNS_1:-1.1.1.1}"
MESHLINK_DNS_2="${MESHLINK_DNS_2:-8.8.8.8}"
MESHLINK_MANAGEMENT_PORT="${MESHLINK_MANAGEMENT_PORT:-33073}"
MESHLINK_SYNC_INTERVAL="${MESHLINK_SYNC_INTERVAL:-1s}"
MESHLINK_CLIENT_A_WG_PORT="${MESHLINK_CLIENT_A_WG_PORT:-51820}"
MESHLINK_CLIENT_B_WG_PORT="${MESHLINK_CLIENT_B_WG_PORT:-51821}"

upstream_network_gateway() {
  virsh net-dumpxml "$MESHLINK_UPSTREAM_NETWORK" 2>/dev/null | sed -n "s/.*<ip address='\([^']*\)'.*/\1/p" | head -n1
}

upstream_network_netmask() {
  virsh net-dumpxml "$MESHLINK_UPSTREAM_NETWORK" 2>/dev/null | sed -n "s/.*<ip address='[^']*' netmask='\([^']*\)'.*/\1/p" | head -n1
}

netmask_to_prefix() {
  local netmask="$1"
  local prefix=0
  local octet

  IFS='.' read -r -a octets <<<"$netmask"
  for octet in "${octets[@]}"; do
    case "$octet" in
      255) prefix=$((prefix + 8)) ;;
      254) prefix=$((prefix + 7)) ;;
      252) prefix=$((prefix + 6)) ;;
      248) prefix=$((prefix + 5)) ;;
      240) prefix=$((prefix + 4)) ;;
      224) prefix=$((prefix + 3)) ;;
      192) prefix=$((prefix + 2)) ;;
      128) prefix=$((prefix + 1)) ;;
      0) ;;
      *)
        echo "unsupported netmask octet: $octet" >&2
        return 1
        ;;
    esac
  done

  printf '%s\n' "$prefix"
}

ipv4_with_host_octet() {
  local address="$1"
  local host_octet="$2"
  local prefix="${address%.*}"
  printf '%s.%s\n' "$prefix" "$host_octet"
}

load_lab_env() {
  if [[ -f "$LAB_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$LAB_ENV_FILE"
  fi

  apply_topology_defaults
}

apply_topology_defaults() {
  if [[ -z "$MESHLINK_LAB_TOPOLOGY" ]]; then
    MESHLINK_LAB_TOPOLOGY="flat"
  fi

  if [[ -z "$MESHLINK_UPSTREAM_NETWORK" ]]; then
    MESHLINK_UPSTREAM_NETWORK="$MESHLINK_LIBVIRT_NETWORK"
  fi

  local detected_gateway=""
  local detected_netmask=""
  if command -v virsh >/dev/null 2>&1; then
    detected_gateway="$(upstream_network_gateway)"
    detected_netmask="$(upstream_network_netmask)"
  fi

  if [[ -z "$MESHLINK_GATEWAY" && -n "$detected_gateway" ]]; then
    MESHLINK_GATEWAY="$detected_gateway"
  fi
  if [[ "$MESHLINK_NET_PREFIX" == "24" && -n "$detected_netmask" ]]; then
    MESHLINK_NET_PREFIX="$(netmask_to_prefix "$detected_netmask" 2>/dev/null || printf '24\n')"
  fi

  if [[ -z "$MESHLINK_MGMT_IP" ]]; then
    MESHLINK_MGMT_IP="$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 201)"
  fi

  if [[ -z "$MESHLINK_NAT_A_NETWORK_NAME" ]]; then
    MESHLINK_NAT_A_NETWORK_NAME="${MESHLINK_LAB_PREFIX}-nat-a-lan"
  fi
  if [[ -z "$MESHLINK_NAT_B_NETWORK_NAME" ]]; then
    MESHLINK_NAT_B_NETWORK_NAME="${MESHLINK_LAB_PREFIX}-nat-b-lan"
  fi
  if [[ -z "$MESHLINK_NAT_A_BRIDGE_NAME" ]]; then
    MESHLINK_NAT_A_BRIDGE_NAME="mlnata0"
  fi
  if [[ -z "$MESHLINK_NAT_B_BRIDGE_NAME" ]]; then
    MESHLINK_NAT_B_BRIDGE_NAME="mlnatb0"
  fi

  case "$MESHLINK_LAB_TOPOLOGY" in
    flat)
      if [[ -z "$MESHLINK_CLIENT_A_IP" ]]; then
        MESHLINK_CLIENT_A_IP="$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 202)"
      fi
      if [[ -z "$MESHLINK_CLIENT_B_IP" ]]; then
        MESHLINK_CLIENT_B_IP="$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 203)"
      fi
      ;;
    dual-nat)
      if [[ -z "$MESHLINK_CLIENT_A_IP" || "$MESHLINK_CLIENT_A_IP" == "$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 202)" ]]; then
        MESHLINK_CLIENT_A_IP="10.10.1.10"
      fi
      if [[ -z "$MESHLINK_CLIENT_B_IP" || "$MESHLINK_CLIENT_B_IP" == "$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 203)" ]]; then
        MESHLINK_CLIENT_B_IP="10.10.2.10"
      fi
      if [[ -z "$MESHLINK_NAT_A_WAN_IP" ]]; then
        MESHLINK_NAT_A_WAN_IP="$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 211)"
      fi
      if [[ -z "$MESHLINK_NAT_A_LAN_IP" ]]; then
        MESHLINK_NAT_A_LAN_IP="10.10.1.1"
      fi
      if [[ -z "$MESHLINK_NAT_B_WAN_IP" ]]; then
        MESHLINK_NAT_B_WAN_IP="$(ipv4_with_host_octet "$MESHLINK_GATEWAY" 221)"
      fi
      if [[ -z "$MESHLINK_NAT_B_LAN_IP" ]]; then
        MESHLINK_NAT_B_LAN_IP="10.10.2.1"
      fi
      if [[ -z "$MESHLINK_NAT_A_LAN_CIDR" ]]; then
        MESHLINK_NAT_A_LAN_CIDR="10.10.1.0/24"
      fi
      if [[ -z "$MESHLINK_NAT_B_LAN_CIDR" ]]; then
        MESHLINK_NAT_B_LAN_CIDR="10.10.2.0/24"
      fi
      if [[ -z "$MESHLINK_NAT_A_LAN_HOST_IP" ]]; then
        MESHLINK_NAT_A_LAN_HOST_IP="10.10.1.254"
      fi
      if [[ -z "$MESHLINK_NAT_B_LAN_HOST_IP" ]]; then
        MESHLINK_NAT_B_LAN_HOST_IP="10.10.2.254"
      fi
      ;;
    *)
      echo "unsupported MESHLINK_LAB_TOPOLOGY: $MESHLINK_LAB_TOPOLOGY" >&2
      exit 1
      ;;
  esac
}

require_flat_topology() {
  if [[ "$MESHLINK_LAB_TOPOLOGY" != "flat" ]]; then
    echo "this script currently supports MESHLINK_LAB_TOPOLOGY=flat only" >&2
    exit 1
  fi
}

require_dual_nat_topology() {
  if [[ "$MESHLINK_LAB_TOPOLOGY" != "dual-nat" ]]; then
    echo "this script requires MESHLINK_LAB_TOPOLOGY=dual-nat" >&2
    exit 1
  fi
}

lab_nodes() {
  if [[ "$MESHLINK_LAB_TOPOLOGY" == "dual-nat" ]]; then
    printf '%s\n' mgmt-1 nat-a nat-b client-a client-b
    return
  fi

  printf '%s\n' mgmt-1 client-a client-b
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

cidr_prefix() {
  printf '%s\n' "${1#*/}"
}

prefix_to_netmask() {
  local prefix="$1"
  local mask=""
  local remaining="$prefix"
  local octet

  for _ in 1 2 3 4; do
    if (( remaining >= 8 )); then
      octet=255
      remaining=$((remaining - 8))
    elif (( remaining <= 0 )); then
      octet=0
    else
      octet=$((256 - 2 ** (8 - remaining)))
      remaining=0
    fi

    if [[ -z "$mask" ]]; then
      mask="$octet"
    else
      mask="${mask}.${octet}"
    fi
  done

  printf '%s\n' "$mask"
}

vm_name() {
  printf '%s-%s\n' "$MESHLINK_LAB_PREFIX" "$1"
}

vm_ip() {
  case "$1" in
    mgmt-1) printf '%s\n' "$MESHLINK_MGMT_IP" ;;
    nat-a) printf '%s\n' "$MESHLINK_NAT_A_WAN_IP" ;;
    nat-b) printf '%s\n' "$MESHLINK_NAT_B_WAN_IP" ;;
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
    detected="$(virsh net-dhcp-leases "$MESHLINK_UPSTREAM_NETWORK" 2>/dev/null | awk -v target_mac="$mac" '$2 == target_mac { sub(/\/.*/, "", $5); print $5; exit }')"
    if [[ -n "$detected" ]]; then
      printf '%s\n' "$detected"
      return
    fi
  fi

  vm_ip "$node"
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

print_vm_access_diagnostics() {
  local node="$1"
  local expected_ip
  expected_ip="$(vm_ip "$node")"

  echo "[$node] expected_ip=${expected_ip}" >&2
  echo "[$node] domstate:" >&2
  virsh domstate "$(vm_name "$node")" >&2 || true
  echo "[$node] interfaces:" >&2
  virsh domiflist "$(vm_name "$node")" >&2 || true
  echo "[$node] ping check:" >&2
  ping -c 1 -W 1 "$expected_ip" >&2 || true
  echo "[$node] ssh check:" >&2
  timeout 8s bash -lc "source '$ROOT_DIR/tests/nat-lab/common.sh'; load_lab_env; ssh_to_vm '$node' true" >&2 || true
}

verify_host_vm_access() {
  local node="$1"
  local expected_ip
  local attempts="${2:-45}"
  local attempt=1
  expected_ip="$(vm_ip "$node")"

  if ! virsh domstate "$(vm_name "$node")" 2>/dev/null | grep -q '^running$'; then
    echo "vm is not running: $(vm_name "$node")" >&2
    return 1
  fi

  while (( attempt <= attempts )); do
    local ping_ok=1
    local ssh_ok=1

    if ping -c 1 -W 1 "$expected_ip" >/dev/null 2>&1; then
      ping_ok=0
    fi
    if timeout 8s bash -lc "source '$ROOT_DIR/tests/nat-lab/common.sh'; load_lab_env; ssh_to_vm '$node' true" >/dev/null 2>&1; then
      ssh_ok=0
    fi

    if (( ssh_ok == 0 )); then
      return 0
    fi

    sleep 2
    attempt=$((attempt + 1))
  done

  if ! ping -c 1 -W 1 "$expected_ip" >/dev/null 2>&1; then
    echo "host cannot ping ${node} at ${expected_ip}" >&2
  fi
  if ! timeout 8s bash -lc "source '$ROOT_DIR/tests/nat-lab/common.sh'; load_lab_env; ssh_to_vm '$node' true" >/dev/null 2>&1; then
    echo "host cannot ssh to ${node} at ${expected_ip}" >&2
  fi
  return 1
}

preflight_dual_nat_access() {
  require_dual_nat_topology

  local failed=0
  for node in mgmt-1 nat-a nat-b client-a client-b; do
    if ! verify_host_vm_access "$node"; then
      failed=1
      print_vm_access_diagnostics "$node"
    fi
  done

  if (( failed > 0 )); then
    echo "dual-nat preflight failed; recreate the lab with tests/nat-lab/destroy-lab.sh && tests/nat-lab/create-lab.sh" >&2
    exit 1
  fi
}

vm_mac() {
  local node="$1"
  local iface="${2:-primary}"

  case "${node}:${iface}" in
    mgmt-1:primary) printf '%s\n' "52:54:00:60:00:11" ;;
    nat-a:primary) printf '%s\n' "52:54:00:60:00:21" ;;
    nat-a:secondary) printf '%s\n' "52:54:00:60:10:21" ;;
    nat-b:primary) printf '%s\n' "52:54:00:60:00:31" ;;
    nat-b:secondary) printf '%s\n' "52:54:00:60:10:31" ;;
    client-a:primary) printf '%s\n' "52:54:00:60:20:41" ;;
    client-b:primary) printf '%s\n' "52:54:00:60:20:51" ;;
    *)
      echo "unknown node/interface ${node}:${iface}" >&2
      exit 1
      ;;
  esac
}

ensure_isolated_network() {
  local network_name="$1"
  local bridge_name="$2"
  local host_ip="$3"
  local cidr="$4"
  local prefix
  local netmask
  local xml_path

  prefix="$(cidr_prefix "$cidr")"
  netmask="$(prefix_to_netmask "$prefix")"
  xml_path="$MESHLINK_LAB_STATE_DIR/networks/${network_name}.xml"

  mkdir -p "$(dirname "$xml_path")"

  if ! virsh net-info "$network_name" >/dev/null 2>&1; then
    cat >"$xml_path" <<EOF
<network>
  <name>${network_name}</name>
  <bridge name='${bridge_name}' stp='on' delay='0'/>
  <forward mode='none'/>
  <ip address='${host_ip}' netmask='${netmask}'/>
</network>
EOF
    virsh net-define "$xml_path" >/dev/null
  fi

  if ! virsh net-info "$network_name" | grep -q 'Active:.*yes'; then
    virsh net-start "$network_name" >/dev/null
  fi
}

destroy_isolated_network() {
  local network_name="$1"

  if virsh net-info "$network_name" >/dev/null 2>&1; then
    virsh net-destroy "$network_name" >/dev/null 2>&1 || true
    virsh net-undefine "$network_name" >/dev/null 2>&1 || true
  fi
}

write_standard_user_data() {
  local node="$1"
  local node_dir="$2"
  local ssh_key="$3"

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
}

write_router_user_data() {
  local node="$1"
  local node_dir="$2"
  local ssh_key="$3"

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
  - conntrack
  - iptables
  - iputils-ping
ssh_pwauth: true
chpasswd:
  list: |
    $MESHLINK_SSH_USER:$MESHLINK_SSH_PASSWORD
  expire: false
write_files:
  - path: /usr/local/sbin/meshlink-router-init.sh
    permissions: '0755'
    content: |
      #!/usr/bin/env bash
      set -euo pipefail
      sysctl -w net.ipv4.ip_forward=1 >/dev/null
      iptables -t nat -C POSTROUTING -o wan0 -j MASQUERADE 2>/dev/null || iptables -t nat -A POSTROUTING -o wan0 -j MASQUERADE
      iptables -C FORWARD -i lan0 -o wan0 -j ACCEPT 2>/dev/null || iptables -A FORWARD -i lan0 -o wan0 -j ACCEPT
      iptables -C FORWARD -i wan0 -o lan0 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || iptables -A FORWARD -i wan0 -o lan0 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT
runcmd:
  - mkdir -p /home/$MESHLINK_SSH_USER/meshlink/logs
  - chown -R $MESHLINK_SSH_USER:$MESHLINK_SSH_USER /home/$MESHLINK_SSH_USER/meshlink
  - printf 'net.ipv4.ip_forward=1\n' >/etc/sysctl.d/99-meshlink-router.conf
  - sysctl --system
  - /usr/local/sbin/meshlink-router-init.sh
EOF
}

write_network_config() {
  local node="$1"
  local node_dir="$2"
  local upstream_cidr="${MESHLINK_MGMT_IP%.*}.0/${MESHLINK_NET_PREFIX}"

  case "$MESHLINK_LAB_TOPOLOGY:$node" in
    flat:mgmt-1)
      cat >"$node_dir/network-config" <<EOF
version: 2
ethernets:
  uplink0:
    match:
      macaddress: "$(vm_mac "$node")"
    set-name: uplink0
    dhcp4: false
    addresses:
      - ${MESHLINK_MGMT_IP}/${MESHLINK_NET_PREFIX}
    routes:
      - to: default
        via: ${MESHLINK_GATEWAY}
    nameservers:
      addresses: [${MESHLINK_DNS_1}, ${MESHLINK_DNS_2}]
EOF
      ;;
    flat:client-a|flat:client-b)
      cat >"$node_dir/network-config" <<EOF
version: 2
ethernets:
  uplink0:
    match:
      macaddress: "$(vm_mac "$node")"
    set-name: uplink0
    dhcp4: false
    addresses:
      - $(vm_ip "$node")/${MESHLINK_NET_PREFIX}
    routes:
      - to: default
        via: ${MESHLINK_GATEWAY}
    nameservers:
      addresses: [${MESHLINK_DNS_1}, ${MESHLINK_DNS_2}]
EOF
      ;;
    dual-nat:mgmt-1)
      cat >"$node_dir/network-config" <<EOF
version: 2
ethernets:
  uplink0:
    match:
      macaddress: "$(vm_mac "$node")"
    set-name: uplink0
    dhcp4: false
    addresses:
      - ${MESHLINK_MGMT_IP}/${MESHLINK_NET_PREFIX}
    routes:
      - to: default
        via: ${MESHLINK_GATEWAY}
    nameservers:
      addresses: [${MESHLINK_DNS_1}, ${MESHLINK_DNS_2}]
EOF
      ;;
    dual-nat:nat-a)
      cat >"$node_dir/network-config" <<EOF
version: 2
ethernets:
  wan0:
    match:
      macaddress: "$(vm_mac "$node" primary)"
    set-name: wan0
    dhcp4: false
    addresses:
      - ${MESHLINK_NAT_A_WAN_IP}/${MESHLINK_NET_PREFIX}
    routes:
      - to: default
        via: ${MESHLINK_GATEWAY}
    nameservers:
      addresses: [${MESHLINK_DNS_1}, ${MESHLINK_DNS_2}]
  lan0:
    match:
      macaddress: "$(vm_mac "$node" secondary)"
    set-name: lan0
    dhcp4: false
    addresses:
      - ${MESHLINK_NAT_A_LAN_IP}/$(cidr_prefix "$MESHLINK_NAT_A_LAN_CIDR")
EOF
      ;;
    dual-nat:nat-b)
      cat >"$node_dir/network-config" <<EOF
version: 2
ethernets:
  wan0:
    match:
      macaddress: "$(vm_mac "$node" primary)"
    set-name: wan0
    dhcp4: false
    addresses:
      - ${MESHLINK_NAT_B_WAN_IP}/${MESHLINK_NET_PREFIX}
    routes:
      - to: default
        via: ${MESHLINK_GATEWAY}
    nameservers:
      addresses: [${MESHLINK_DNS_1}, ${MESHLINK_DNS_2}]
  lan0:
    match:
      macaddress: "$(vm_mac "$node" secondary)"
    set-name: lan0
    dhcp4: false
    addresses:
      - ${MESHLINK_NAT_B_LAN_IP}/$(cidr_prefix "$MESHLINK_NAT_B_LAN_CIDR")
EOF
      ;;
    dual-nat:client-a)
      cat >"$node_dir/network-config" <<EOF
version: 2
ethernets:
  lan0:
    match:
      macaddress: "$(vm_mac "$node")"
    set-name: lan0
    dhcp4: false
    addresses:
      - ${MESHLINK_CLIENT_A_IP}/$(cidr_prefix "$MESHLINK_NAT_A_LAN_CIDR")
    routes:
      - to: default
        via: ${MESHLINK_NAT_A_LAN_IP}
    nameservers:
      addresses: [${MESHLINK_DNS_1}, ${MESHLINK_DNS_2}]
EOF
      ;;
    dual-nat:client-b)
      cat >"$node_dir/network-config" <<EOF
version: 2
ethernets:
  lan0:
    match:
      macaddress: "$(vm_mac "$node")"
    set-name: lan0
    dhcp4: false
    addresses:
      - ${MESHLINK_CLIENT_B_IP}/$(cidr_prefix "$MESHLINK_NAT_B_LAN_CIDR")
    routes:
      - to: default
        via: ${MESHLINK_NAT_B_LAN_IP}
    nameservers:
      addresses: [${MESHLINK_DNS_1}, ${MESHLINK_DNS_2}]
EOF
      ;;
    *)
      echo "unsupported network-config target ${MESHLINK_LAB_TOPOLOGY}:${node}" >&2
      exit 1
      ;;
  esac
}

write_cloud_init_files() {
  local node="$1"
  local node_dir="$MESHLINK_LAB_STATE_DIR/$node"
  local ssh_key
  ssh_key="$(ensure_ssh_public_key)"

  mkdir -p "$node_dir"

  case "$node" in
    nat-a|nat-b)
      write_router_user_data "$node" "$node_dir" "$ssh_key"
      ;;
    *)
      write_standard_user_data "$node" "$node_dir" "$ssh_key"
      ;;
  esac

  cat >"$node_dir/meta-data" <<EOF
instance-id: $(vm_name "$node")
local-hostname: $(vm_name "$node")
EOF

  write_network_config "$node" "$node_dir"
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
    "network-config=$node_dir/network-config" \
    >/dev/null 2>&1
}

assert_nat_router_ready() {
  local node="$1"
  ssh_to_vm "$node" "sudo sysctl net.ipv4.ip_forward | grep -q ' = 1' && sudo iptables -t nat -C POSTROUTING -o wan0 -j MASQUERADE && sudo iptables -C FORWARD -i lan0 -o wan0 -j ACCEPT && sudo iptables -C FORWARD -i wan0 -o lan0 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT"
}

wait_for_nat_router_ready() {
  local node="$1"
  local attempts="${2:-45}"
  local attempt=1

  while (( attempt <= attempts )); do
    if assert_nat_router_ready "$node" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for NAT router readiness on ${node}" >&2
  return 1
}

ensure_udp_source_port_preserving_nat() {
  local node="$1"
  local client_ip="$2"
  local public_ip="$3"
  local source_port="$4"

  ssh_to_vm "$node" "sudo iptables -t nat -C POSTROUTING -o wan0 -p udp -s ${client_ip} --sport ${source_port} -j SNAT --to-source ${public_ip}:${source_port}-${source_port} 2>/dev/null || sudo iptables -t nat -I POSTROUTING 1 -o wan0 -p udp -s ${client_ip} --sport ${source_port} -j SNAT --to-source ${public_ip}:${source_port}-${source_port}"
}

ensure_udp_destination_port_forward() {
  local node="$1"
  local client_ip="$2"
  local destination_port="$3"

  ssh_to_vm "$node" "sudo iptables -t nat -C PREROUTING -i wan0 -p udp --dport ${destination_port} -j DNAT --to-destination ${client_ip}:${destination_port} 2>/dev/null || sudo iptables -t nat -I PREROUTING 1 -i wan0 -p udp --dport ${destination_port} -j DNAT --to-destination ${client_ip}:${destination_port}"
  ssh_to_vm "$node" "sudo iptables -C FORWARD -i wan0 -o lan0 -p udp -d ${client_ip} --dport ${destination_port} -j ACCEPT 2>/dev/null || sudo iptables -I FORWARD 1 -i wan0 -o lan0 -p udp -d ${client_ip} --dport ${destination_port} -j ACCEPT"
}

ensure_dual_nat_wireguard_port_mapping() {
  require_dual_nat_topology
  ensure_udp_source_port_preserving_nat nat-a "$MESHLINK_CLIENT_A_IP" "$MESHLINK_NAT_A_WAN_IP" "$MESHLINK_CLIENT_A_WG_PORT"
  ensure_udp_source_port_preserving_nat nat-b "$MESHLINK_CLIENT_B_IP" "$MESHLINK_NAT_B_WAN_IP" "$MESHLINK_CLIENT_B_WG_PORT"
  ensure_udp_destination_port_forward nat-a "$MESHLINK_CLIENT_A_IP" "$MESHLINK_CLIENT_A_WG_PORT"
  ensure_udp_destination_port_forward nat-b "$MESHLINK_CLIENT_B_IP" "$MESHLINK_CLIENT_B_WG_PORT"
}

install_phase06_drop_rule() {
  local node="$1"
  local peer_wan_ip="$2"
  local peer_port="$3"
  local local_client_ip="$4"
  local local_port="$5"
  local comment="$6"
  ssh_to_vm "$node" "sudo iptables -C FORWARD -i lan0 -o wan0 -p udp -d ${peer_wan_ip} --dport ${peer_port} -m comment --comment ${comment}-out -j DROP 2>/dev/null || sudo iptables -I FORWARD 1 -i lan0 -o wan0 -p udp -d ${peer_wan_ip} --dport ${peer_port} -m comment --comment ${comment}-out -j DROP"
  ssh_to_vm "$node" "sudo iptables -C FORWARD -i wan0 -o lan0 -p udp -s ${peer_wan_ip} --sport ${peer_port} -d ${local_client_ip} --dport ${local_port} -m comment --comment ${comment}-in -j DROP 2>/dev/null || sudo iptables -I FORWARD 1 -i wan0 -o lan0 -p udp -s ${peer_wan_ip} --sport ${peer_port} -d ${local_client_ip} --dport ${local_port} -m comment --comment ${comment}-in -j DROP"
}

clear_phase06_drop_rule() {
  local node="$1"
  local peer_wan_ip="$2"
  local peer_port="$3"
  local local_client_ip="$4"
  local local_port="$5"
  local comment="$6"
  ssh_to_vm "$node" "sudo iptables -D FORWARD -i lan0 -o wan0 -p udp -d ${peer_wan_ip} --dport ${peer_port} -m comment --comment ${comment}-out -j DROP 2>/dev/null || true"
  ssh_to_vm "$node" "sudo iptables -D FORWARD -i wan0 -o lan0 -p udp -s ${peer_wan_ip} --sport ${peer_port} -d ${local_client_ip} --dport ${local_port} -m comment --comment ${comment}-in -j DROP 2>/dev/null || true"
}

install_phase06_drop_rules() {
  require_dual_nat_topology
  install_phase06_drop_rule nat-a "$MESHLINK_NAT_B_WAN_IP" "$MESHLINK_CLIENT_B_WG_PORT" "$MESHLINK_CLIENT_A_IP" "$MESHLINK_CLIENT_A_WG_PORT" "meshlink-phase06-drop-b"
  install_phase06_drop_rule nat-b "$MESHLINK_NAT_A_WAN_IP" "$MESHLINK_CLIENT_A_WG_PORT" "$MESHLINK_CLIENT_B_IP" "$MESHLINK_CLIENT_B_WG_PORT" "meshlink-phase06-drop-a"
}

clear_phase06_drop_rules() {
  if [[ "$MESHLINK_LAB_TOPOLOGY" != "dual-nat" ]]; then
    return
  fi

  clear_phase06_drop_rule nat-a "$MESHLINK_NAT_B_WAN_IP" "$MESHLINK_CLIENT_B_WG_PORT" "$MESHLINK_CLIENT_A_IP" "$MESHLINK_CLIENT_A_WG_PORT" "meshlink-phase06-drop-b"
  clear_phase06_drop_rule nat-b "$MESHLINK_NAT_A_WAN_IP" "$MESHLINK_CLIENT_A_WG_PORT" "$MESHLINK_CLIENT_B_IP" "$MESHLINK_CLIENT_B_WG_PORT" "meshlink-phase06-drop-a"
}

collect_router_state() {
  local node="$1"
  local suffix="${2:-current}"
  local base="$MESHLINK_LAB_STATE_DIR/runtime/${node}-${suffix}"

  mkdir -p "$MESHLINK_LAB_STATE_DIR/runtime"
  ssh_to_vm "$node" "sudo iptables-save" >"${base}-iptables-save.txt"
  ssh_to_vm "$node" "sudo bash -lc 'if command -v conntrack >/dev/null 2>&1; then conntrack -L; fi'" >"${base}-conntrack.txt" || true
}
