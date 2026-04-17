#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WINDOWS_ENV_FILE="${MESHLINK_WINDOWS_ENV:-$ROOT_DIR/tests/windows-vm/windows.env}"
VALIDATION_STATE_DIR="${MESHLINK_WINDOWS_VALIDATION_STATE_DIR:-${MESHLINK_LAB_STATE_DIR:-/tmp/meshlink-nat-lab}/windows-validation}"
MESHLINK_BOOTSTRAP_TOKEN="${MESHLINK_BOOTSTRAP_TOKEN:-meshlink-dev-token}"
MESHLINK_WINDOWS_INTERFACE_NAME="${MESHLINK_WINDOWS_INTERFACE_NAME:-MeshLink}"
MESHLINK_WINDOWS_LOG_LEVEL="${MESHLINK_WINDOWS_LOG_LEVEL:-debug}"
MESHLINK_WINDOWS_PUNCH_TIMEOUT="${MESHLINK_WINDOWS_PUNCH_TIMEOUT:-5s}"
MESHLINK_WINDOWS_PACKAGE_DIR="${MESHLINK_WINDOWS_PACKAGE_DIR:-C:\\MeshLink}"
MESHLINK_WINDOWS_SERVICE_CONFIG_DIR="${MESHLINK_WINDOWS_SERVICE_CONFIG_DIR:-C:\\ProgramData\\MeshLink}"

# shellcheck disable=SC1091
source "$ROOT_DIR/tests/nat-lab/common.sh"

ROUTED_SUBNET="10.20.0.0/24"
ROUTED_GATEWAY_IP="10.20.0.1"
ROUTED_TARGET_IP="10.20.0.2"
ROUTE_NAMESPACE="meshlink-route-ns"
ROUTE_HOST_IF="meshlink-veth0"
ROUTE_NS_IF="meshlink-veth1"
ROUTE_ADVERTISER_NODE="client-a"

RESULT_DIRECT="pending"
RESULT_RELAY="pending"
RESULT_RECOVERY="pending"
RESULT_ROUTE_ADV="pending"
RESULT_ROUTE_WITHDRAW="pending"
WINDOWS_PRIVATE_KEY=""
WINDOWS_PUBLIC_KEY=""

load_windows_env() {
  if [[ -f "$WINDOWS_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$WINDOWS_ENV_FILE"
  fi
}

require_local_commands() {
  require_commands virsh ssh scp wg iconv base64 grep awk sed mktemp
  command -v jq >/dev/null 2>&1 || {
    echo "jq is required" >&2
    exit 1
  }
}

windows_qga() {
  "$ROOT_DIR/tests/windows-vm/qga.sh" "$@"
}

windows_ps_script() {
  local script="$1"
  local encoded

  encoded="$(printf '%s' "$script" | iconv -f UTF-8 -t UTF-16LE | base64 -w0)"
  windows_qga exec powershell.exe \
    -NoProfile \
    -NonInteractive \
    -ExecutionPolicy Bypass \
    -EncodedCommand "$encoded"
}

detect_windows_side() {
  case "${MESHLINK_WINDOWS_ATTACH_NETWORK:-}" in
    nat-a-lan) printf '%s\n' "nat-a" ;;
    nat-b-lan) printf '%s\n' "nat-b" ;;
    *)
      printf '%s\n' "${MESHLINK_WINDOWS_SIDE:-nat-a}"
      ;;
  esac
}

derive_validation_topology() {
  MESHLINK_WINDOWS_SIDE="$(detect_windows_side)"
  case "$MESHLINK_WINDOWS_SIDE" in
    nat-a)
      DIRECT_NODE="client-a"
      RELAY_NODE="client-b"
      WINDOWS_IP="${MESHLINK_WINDOWS_STATIC_IP:-10.10.1.20}"
      WINDOWS_PUBLIC_IP="$MESHLINK_NAT_A_WAN_IP"
      DIRECT_EXPECTED_HOST="$MESHLINK_CLIENT_A_IP"
      DIRECT_EXPECTED_PORT="${MESHLINK_CLIENT_A_WG_PORT:-51820}"
      RELAY_DIRECT_EXPECTED_HOST="$MESHLINK_NAT_B_WAN_IP"
      RELAY_DIRECT_EXPECTED_PORT="${MESHLINK_CLIENT_B_WG_PORT:-51821}"
      ;;
    nat-b)
      DIRECT_NODE="client-b"
      RELAY_NODE="client-a"
      WINDOWS_IP="${MESHLINK_WINDOWS_STATIC_IP:-10.10.2.20}"
      WINDOWS_PUBLIC_IP="$MESHLINK_NAT_B_WAN_IP"
      DIRECT_EXPECTED_HOST="$MESHLINK_CLIENT_B_IP"
      DIRECT_EXPECTED_PORT="${MESHLINK_CLIENT_B_WG_PORT:-51821}"
      RELAY_DIRECT_EXPECTED_HOST="$MESHLINK_NAT_A_WAN_IP"
      RELAY_DIRECT_EXPECTED_PORT="${MESHLINK_CLIENT_A_WG_PORT:-51820}"
      ;;
    *)
      echo "unsupported MESHLINK_WINDOWS_SIDE: $MESHLINK_WINDOWS_SIDE" >&2
      exit 1
      ;;
  esac
}

select_windows_validation_port() {
  local same_side_port

  case "$MESHLINK_WINDOWS_SIDE" in
    nat-a) same_side_port="${MESHLINK_CLIENT_A_WG_PORT:-51820}" ;;
    nat-b) same_side_port="${MESHLINK_CLIENT_B_WG_PORT:-51821}" ;;
    *)
      echo "unsupported MESHLINK_WINDOWS_SIDE: $MESHLINK_WINDOWS_SIDE" >&2
      exit 1
      ;;
  esac

  MESHLINK_WINDOWS_WG_PORT="${MESHLINK_WINDOWS_WG_PORT:-51820}"
  if [[ "$MESHLINK_WINDOWS_WG_PORT" == "$same_side_port" ]]; then
    MESHLINK_WINDOWS_WG_PORT="${MESHLINK_WINDOWS_ALT_WG_PORT:-51830}"
  fi
}

generate_windows_keys() {
  if [[ -n "${MESHLINK_WINDOWS_PRIVATE_KEY:-}" ]]; then
    WINDOWS_PRIVATE_KEY="$MESHLINK_WINDOWS_PRIVATE_KEY"
    WINDOWS_PUBLIC_KEY="${MESHLINK_WINDOWS_PUBLIC_KEY:-$(printf '%s' "$WINDOWS_PRIVATE_KEY" | wg pubkey)}"
  else
    WINDOWS_PRIVATE_KEY="$(wg genkey)"
    WINDOWS_PUBLIC_KEY="$(printf '%s' "$WINDOWS_PRIVATE_KEY" | wg pubkey)"
  fi

  mkdir -p "$VALIDATION_STATE_DIR"
  printf '%s\n' "$WINDOWS_PRIVATE_KEY" >"$VALIDATION_STATE_DIR/windows.private"
  printf '%s\n' "$WINDOWS_PUBLIC_KEY" >"$VALIDATION_STATE_DIR/windows.public"
}

linux_peer_public_key() {
  local node="$1"
  ssh_to_vm "$node" "sudo wg show sdwan0 public-key 2>/dev/null || awk -F '\"' '/^public_key = / {print \$2; exit}' $REMOTE_ROOT/config/client.toml"
}

ensure_linux_client_relay_config() {
  local node="$1"
  local relay_addr="$MGMT_IP:$MESHLINK_RELAY_PORT"

  ssh_to_vm "$node" "cfg=$REMOTE_ROOT/config/client.toml; tmp=\$(mktemp); awk 'BEGIN{inserted=0} /^relay_addr = / {next} /^signal_addr = / && !inserted {print; print \"relay_addr = \\\"$relay_addr\\\"\"; inserted=1; next} {print} END{if(!inserted) print \"relay_addr = \\\"$relay_addr\\\"\"}' \"\$cfg\" >\"\$tmp\" && mv \"\$tmp\" \"\$cfg\""
}

update_linux_client_routes() {
  local node="$1"
  local routes="${2:-}"

  ssh_to_vm "$node" "cfg=$REMOTE_ROOT/config/client.toml; tmp=\$(mktemp); awk '/^advertised_routes = / {next} {print}' \"\$cfg\" >\"\$tmp\"; if [ -n '$routes' ]; then printf 'advertised_routes = [%s]\n' '$routes' >>\"\$tmp\"; fi; mv \"\$tmp\" \"\$cfg\""
}

restart_linux_client() {
  local node="$1"
  ssh_to_vm "$node" "sudo pkill -x meshlinkd || true; sudo ip link del ${MESHLINK_INTERFACE_NAME:-sdwan0} 2>/dev/null || true; nohup sudo timeout 180s $REMOTE_ROOT/bin/meshlinkd --config $REMOTE_ROOT/config/client.toml > $REMOTE_ROOT/logs/meshlinkd.log 2>&1 < /dev/null &"
}

stop_linux_client() {
  local node="$1"
  ssh_to_vm "$node" "sudo pkill -x meshlinkd || true; sudo ip link del ${MESHLINK_INTERFACE_NAME:-sdwan0} 2>/dev/null || true"
}

start_managementd() {
  ssh_to_vm mgmt-1 "sudo pkill -x managementd || true; nohup ${REMOTE_ROOT}/bin/managementd -listen 0.0.0.0:${MESHLINK_MANAGEMENT_PORT} -sync-interval ${MESHLINK_SYNC_INTERVAL} > ${REMOTE_ROOT}/logs/managementd.log 2>&1 < /dev/null &"
}

start_signald() {
  ssh_to_vm mgmt-1 "sudo pkill -x signald || true; nohup ${REMOTE_ROOT}/bin/signald -listen 0.0.0.0:${MESHLINK_SIGNAL_PORT} -stun-listen 0.0.0.0:${MESHLINK_STUN_PORT} -management-addr 127.0.0.1:${MESHLINK_MANAGEMENT_PORT} > ${REMOTE_ROOT}/logs/signald.log 2>&1 < /dev/null &"
}

start_relayd() {
  ssh_to_vm mgmt-1 "sudo pkill -x relayd || true; nohup ${REMOTE_ROOT}/bin/relayd -listen 0.0.0.0:${MESHLINK_RELAY_PORT} -management-addr 127.0.0.1:${MESHLINK_MANAGEMENT_PORT} -advertise-host ${MGMT_IP} > ${REMOTE_ROOT}/logs/relayd.log 2>&1 < /dev/null &"
}

wait_for_linux_log() {
  local node="$1"
  local pattern="$2"
  local attempts="${3:-30}"
  local attempt=1

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "grep -q '$pattern' '$REMOTE_ROOT/logs/meshlinkd.log'" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for ${node} log pattern: ${pattern}" >&2
  return 1
}

wait_for_remote_log() {
  local node="$1"
  local path="$2"
  local pattern="$3"
  local attempts="${4:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    if ssh_to_vm "$node" "grep -q '$pattern' '$path'" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for ${node} log pattern on ${path}: ${pattern}" >&2
  return 1
}

ensure_linux_clients_ready() {
  wait_for_ssh mgmt-1 30
  ssh_to_vm mgmt-1 "mkdir -p ${REMOTE_ROOT}/logs"

  for node in client-a client-b; do
    wait_for_ssh "$node" 30
    stop_linux_client "$node"
  done

  start_managementd
  sleep 2
  start_signald
  sleep 2
  start_relayd
  wait_for_remote_log mgmt-1 "${REMOTE_ROOT}/logs/managementd.log" "managementd listening on 0.0.0.0:${MESHLINK_MANAGEMENT_PORT}" 40
  wait_for_remote_log mgmt-1 "${REMOTE_ROOT}/logs/signald.log" "signald listening on 0.0.0.0:${MESHLINK_SIGNAL_PORT}" 40
  wait_for_remote_log mgmt-1 "${REMOTE_ROOT}/logs/relayd.log" "relayd listening on 0.0.0.0:${MESHLINK_RELAY_PORT}" 40

  for node in client-a client-b; do
    wait_for_ssh "$node" 30
    ensure_linux_client_relay_config "$node"
  done

  update_linux_client_routes client-a "\"${ROUTED_SUBNET}\""
  update_linux_client_routes client-b ""

  for node in client-a client-b; do
    restart_linux_client "$node"
    wait_for_linux_log "$node" "device registered" 40
  done
}

refresh_windows_package() {
  "$ROOT_DIR/tests/windows-vm/check-package.sh"
  "$ROOT_DIR/tests/windows-vm/refresh-package-iso.sh"
}

deploy_windows_package() {
  local script

  script="$(cat <<'EOF'
$ErrorActionPreference = "Stop"
if (Get-Service -Name "WireGuardTunnel$MeshLink" -ErrorAction SilentlyContinue) {
  Stop-Service -Name "WireGuardTunnel$MeshLink" -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
}
Get-Process meshlinkd -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$drives = Get-PSDrive -PSProvider FileSystem | ForEach-Object { $_.Root }
$zip = $null
foreach ($drive in $drives) {
  $candidate = Join-Path $drive "MESHLINK.ZIP"
  if (Test-Path -LiteralPath $candidate) {
    $zip = $candidate
    break
  }
}
if (-not $zip) {
  throw "MESHLINK.ZIP was not found on any mounted drive."
}

$packageDir = "C:\MeshLink"
$staging = Join-Path $packageDir "_stage"
if (Test-Path $staging) {
  Remove-Item -Recurse -Force $staging
}
New-Item -ItemType Directory -Path $packageDir -Force | Out-Null
Expand-Archive -LiteralPath $zip -DestinationPath $staging -Force
$payload = Get-ChildItem -Path $staging | Where-Object { $_.PSIsContainer } | Select-Object -First 1 -ExpandProperty FullName
if (-not $payload) {
  throw "package payload directory not found"
}
Copy-Item -Path (Join-Path $payload "*") -Destination $packageDir -Recurse -Force
Remove-Item -Recurse -Force $staging
Get-ChildItem $packageDir | Select-Object Name,Length | Sort-Object Name | Format-Table -AutoSize
EOF
)"

  windows_ps_script "$script"
}

write_windows_config() {
  local script

  script="$(cat <<EOF
\$ErrorActionPreference = "Stop"
\$content = @"
node_name = "win-a"
management_addr = "$MGMT_IP:$MESHLINK_MANAGEMENT_PORT"
signal_addr = "$MGMT_IP:$MESHLINK_SIGNAL_PORT"
relay_addr = "$MGMT_IP:$MESHLINK_RELAY_PORT"
stun_addr = "$MGMT_IP:$MESHLINK_STUN_PORT"
bootstrap_token = "$MESHLINK_BOOTSTRAP_TOKEN"
public_key = "$WINDOWS_PUBLIC_KEY"
private_key = "$WINDOWS_PRIVATE_KEY"
interface_name = "$MESHLINK_WINDOWS_INTERFACE_NAME"
listen_port = ${MESHLINK_WINDOWS_WG_PORT:-51820}
advertise_host = "$WINDOWS_PUBLIC_IP"
log_level = "$MESHLINK_WINDOWS_LOG_LEVEL"
punch_timeout = "$MESHLINK_WINDOWS_PUNCH_TIMEOUT"
"@
Set-Content -Path "$MESHLINK_WINDOWS_PACKAGE_DIR\\client.toml" -Value \$content
if (Test-Path "$MESHLINK_WINDOWS_PACKAGE_DIR\\client.example.toml") {
  Set-Content -Path "$MESHLINK_WINDOWS_PACKAGE_DIR\\client.example.toml" -Value \$content
}
Get-Content "$MESHLINK_WINDOWS_PACKAGE_DIR\\client.toml"
EOF
)"

  windows_ps_script "$script"
}

restart_windows_meshlinkd() {
  local script

  script="$(cat <<EOF
\$ErrorActionPreference = "Stop"
if (Get-Service -Name 'WireGuardTunnel\$${MESHLINK_WINDOWS_INTERFACE_NAME}' -ErrorAction SilentlyContinue) {
  Stop-Service -Name 'WireGuardTunnel\$${MESHLINK_WINDOWS_INTERFACE_NAME}' -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
}
Get-Process meshlinkd -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
Remove-Item "$MESHLINK_WINDOWS_PACKAGE_DIR\\meshlinkd.stdout.log","$MESHLINK_WINDOWS_PACKAGE_DIR\\meshlinkd.stderr.log" -Force -ErrorAction SilentlyContinue
Start-Process -FilePath "$MESHLINK_WINDOWS_PACKAGE_DIR\\meshlinkd.exe" -ArgumentList "--config","$MESHLINK_WINDOWS_PACKAGE_DIR\\client.toml" -WorkingDirectory "$MESHLINK_WINDOWS_PACKAGE_DIR" -WindowStyle Hidden
Start-Sleep -Seconds 4
if (-not (Get-Process meshlinkd -ErrorAction SilentlyContinue)) {
  throw "meshlinkd did not start"
}
EOF
)"

  windows_ps_script "$script"
}

stop_windows_meshlinkd() {
  local script

  script="$(cat <<EOF
\$ErrorActionPreference = "Stop"
if (Get-Service -Name 'WireGuardTunnel\$${MESHLINK_WINDOWS_INTERFACE_NAME}' -ErrorAction SilentlyContinue) {
  Stop-Service -Name 'WireGuardTunnel\$${MESHLINK_WINDOWS_INTERFACE_NAME}' -Force -ErrorAction SilentlyContinue
  Start-Sleep -Seconds 2
}
Get-Process meshlinkd -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2
EOF
)"

  windows_ps_script "$script" >/dev/null
}

read_windows_file() {
  local path="$1"
  windows_qga read "$path"
}

wait_for_windows_powershell_success() {
  local description="$1"
  local script="$2"
  local attempts="${3:-40}"
  local attempt=1
  local output=""

  while (( attempt <= attempts )); do
    if output="$(windows_ps_script "$script" 2>&1)"; then
      if [[ -n "$output" ]]; then
        printf '%s\n' "$output"
      fi
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for Windows state: $description" >&2
  if [[ -n "$output" ]]; then
    echo "$output" >&2
  fi
  return 1
}

wait_for_windows_service_running() {
  wait_for_windows_powershell_success \
    "embedded service running" \
    "\$svc = Get-Service -Name 'WireGuardTunnel\$$MESHLINK_WINDOWS_INTERFACE_NAME' -ErrorAction Stop; if (\$svc.Status -ne 'Running') { throw 'service not running' }; \$svc | Select-Object Name,Status | Format-Table -HideTableHeaders" \
    "${1:-40}"
}

wait_for_windows_interface_ready() {
  wait_for_windows_powershell_success \
    "overlay interface ready" \
    "\$adapter = Get-NetAdapter -Name '$MESHLINK_WINDOWS_INTERFACE_NAME' -ErrorAction Stop; if (\$adapter.Status -ne 'Up') { throw 'interface not up' }; Get-NetIPAddress -InterfaceAlias '$MESHLINK_WINDOWS_INTERFACE_NAME' -AddressFamily IPv4 -ErrorAction Stop | Select-Object IPAddress,PrefixLength | Format-Table -HideTableHeaders" \
    "${1:-40}"
}

wait_for_windows_service_config() {
  wait_for_windows_powershell_success \
    "rendered service config present" \
    "if (-not (Test-Path '$MESHLINK_WINDOWS_SERVICE_CONFIG_DIR\\$MESHLINK_WINDOWS_INTERFACE_NAME.conf')) { throw 'config missing' }; Get-Item '$MESHLINK_WINDOWS_SERVICE_CONFIG_DIR\\$MESHLINK_WINDOWS_INTERFACE_NAME.conf' | Select-Object FullName,Length | Format-Table -HideTableHeaders" \
    "${1:-40}"
}

extract_peer_endpoint_from_conf() {
  local config_text="$1"
  local public_key="$2"

  awk -v key="$public_key" '
    /^\[Peer\]/ { in_peer = 1; match_key = 0; next }
    in_peer && /^PublicKey = / {
      match_key = ($3 == key)
      next
    }
    in_peer && match_key && /^Endpoint = / {
      print $3
      exit
    }
  ' <<<"$config_text"
}

extract_peer_allowed_ips_from_conf() {
  local config_text="$1"
  local public_key="$2"

  awk -v key="$public_key" '
    /^\[Peer\]/ { in_peer = 1; match_key = 0; next }
    in_peer && /^PublicKey = / {
      match_key = ($3 == key)
      next
    }
    in_peer && match_key && /^AllowedIPs = / {
      sub(/^AllowedIPs = /, "")
      print
      exit
    }
  ' <<<"$config_text"
}

extract_peer_overlay_ip_from_conf() {
  local config_text="$1"
  local public_key="$2"
  local allowed_ips token

  allowed_ips="$(extract_peer_allowed_ips_from_conf "$config_text" "$public_key")"
  IFS=',' read -r -a tokens <<<"$allowed_ips"
  for token in "${tokens[@]}"; do
    token="${token## }"
    token="${token%% }"
    if [[ "$token" == */32 ]]; then
      printf '%s\n' "${token%/32}"
      return 0
    fi
  done

  return 1
}

wait_for_windows_peer_present() {
  local public_key="$1"
  local attempts="${2:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    local config_text
    config_text="$(read_windows_file "$MESHLINK_WINDOWS_SERVICE_CONFIG_DIR\\$MESHLINK_WINDOWS_INTERFACE_NAME.conf" 2>/dev/null || true)"
    if grep -Fq "PublicKey = $public_key" <<<"$config_text"; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for Windows peer $public_key to appear in service config" >&2
  return 1
}

wait_for_windows_peer_endpoint_host() {
  local public_key="$1"
  local expected_host="$2"
  local attempts="${3:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    local config_text endpoint host
    config_text="$(read_windows_file "$MESHLINK_WINDOWS_SERVICE_CONFIG_DIR\\$MESHLINK_WINDOWS_INTERFACE_NAME.conf" 2>/dev/null || true)"
    endpoint="$(extract_peer_endpoint_from_conf "$config_text" "$public_key")"
    host="${endpoint%:*}"
    if [[ -n "$endpoint" && "$host" == "$expected_host" ]]; then
      printf '%s\n' "$endpoint"
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for Windows peer $public_key endpoint host $expected_host" >&2
  return 1
}

wait_for_windows_peer_allowed_ip_state() {
  local public_key="$1"
  local route="$2"
  local expected_state="$3"
  local attempts="${4:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    local config_text allowed_ips
    config_text="$(read_windows_file "$MESHLINK_WINDOWS_SERVICE_CONFIG_DIR\\$MESHLINK_WINDOWS_INTERFACE_NAME.conf" 2>/dev/null || true)"
    allowed_ips="$(extract_peer_allowed_ips_from_conf "$config_text" "$public_key")"
    if grep -Eq "(^|, )${route}(, |$)" <<<"$allowed_ips"; then
      if [[ "$expected_state" == "present" ]]; then
        printf '%s\n' "$allowed_ips"
        return 0
      fi
    else
      if [[ "$expected_state" == "absent" ]]; then
        printf '%s\n' "$allowed_ips"
        return 0
      fi
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for Windows peer $public_key allowed IP ${route} to become ${expected_state}" >&2
  return 1
}

wait_for_windows_route_state() {
  local route="$1"
  local expected_state="$2"
  local attempts="${3:-40}"

  if [[ "$expected_state" == "present" ]]; then
    wait_for_windows_powershell_success \
      "route ${route} present on Windows" \
      "\$route = Get-NetRoute -AddressFamily IPv4 | Where-Object DestinationPrefix -eq '$route'; if (-not \$route) { throw 'route missing' }; \$route | Select-Object ifIndex,DestinationPrefix,NextHop,RouteMetric | Format-Table -HideTableHeaders" \
      "$attempts"
  else
    wait_for_windows_powershell_success \
      "route ${route} absent on Windows" \
      "\$route = Get-NetRoute -AddressFamily IPv4 | Where-Object DestinationPrefix -eq '$route'; if (\$route) { throw 'route still present' }" \
      "$attempts" >/dev/null
  fi
}

wait_for_windows_ping_state() {
  local target="$1"
  local expected_state="$2"
  local attempts="${3:-25}"

  if [[ "$expected_state" == "present" ]]; then
    wait_for_windows_powershell_success \
      "Windows reachability to ${target}" \
      "if (-not (Test-Connection -ComputerName '$target' -Count 2 -Quiet)) { throw 'target unreachable' }; 'reachable'" \
      "$attempts" >/dev/null
  else
    wait_for_windows_powershell_success \
      "Windows loss of reachability to ${target}" \
      "if (Test-Connection -ComputerName '$target' -Count 1 -Quiet) { throw 'target still reachable' }" \
      "$attempts" >/dev/null
  fi
}

wait_for_windows_peer_overlay_ping_state() {
  local public_key="$1"
  local expected_state="$2"
  local attempts="${3:-25}"
  local attempt=1

  while (( attempt <= attempts )); do
    local config_text overlay_ip
    config_text="$(read_windows_file "$MESHLINK_WINDOWS_SERVICE_CONFIG_DIR\\$MESHLINK_WINDOWS_INTERFACE_NAME.conf" 2>/dev/null || true)"
    overlay_ip="$(extract_peer_overlay_ip_from_conf "$config_text" "$public_key" 2>/dev/null || true)"
    if [[ -n "$overlay_ip" ]]; then
      if wait_for_windows_ping_state "$overlay_ip" "$expected_state" 1; then
        printf '%s\n' "$overlay_ip"
        return 0
      fi
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for Windows overlay ping to peer $public_key to become ${expected_state}" >&2
  return 1
}

linux_peer_endpoint() {
  local node="$1"
  local public_key="$2"
  ssh_to_vm "$node" "sudo wg show ${MESHLINK_INTERFACE_NAME:-sdwan0} endpoints | awk '\$1 == \"${public_key}\" {print \$2; exit}'" 2>/dev/null || true
}

wait_for_linux_peer_endpoint_host() {
  local node="$1"
  local public_key="$2"
  local expected_host="$3"
  local attempts="${4:-40}"
  local attempt=1

  while (( attempt <= attempts )); do
    local endpoint host
    endpoint="$(linux_peer_endpoint "$node" "$public_key")"
    host="${endpoint%:*}"
    if [[ -n "$endpoint" && "$host" == "$expected_host" ]]; then
      printf '%s\n' "$endpoint"
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "timed out waiting for ${node} peer endpoint host $expected_host" >&2
  return 1
}

setup_routed_subnet_target() {
  ssh_to_vm client-a "sudo ip netns del ${ROUTE_NAMESPACE} 2>/dev/null || true; sudo ip link del ${ROUTE_HOST_IF} 2>/dev/null || true; sudo ip netns add ${ROUTE_NAMESPACE}; sudo ip link add ${ROUTE_HOST_IF} type veth peer name ${ROUTE_NS_IF}; sudo ip link set ${ROUTE_NS_IF} netns ${ROUTE_NAMESPACE}; sudo ip addr add ${ROUTED_GATEWAY_IP}/24 dev ${ROUTE_HOST_IF}; sudo ip link set ${ROUTE_HOST_IF} up; sudo ip netns exec ${ROUTE_NAMESPACE} ip addr add ${ROUTED_TARGET_IP}/24 dev ${ROUTE_NS_IF}; sudo ip netns exec ${ROUTE_NAMESPACE} ip link set lo up; sudo ip netns exec ${ROUTE_NAMESPACE} ip link set ${ROUTE_NS_IF} up; sudo ip netns exec ${ROUTE_NAMESPACE} ip route replace 100.64.0.0/10 via ${ROUTED_GATEWAY_IP}; sudo sysctl -w net.ipv4.ip_forward=1 >/dev/null; sudo iptables -C FORWARD -i ${MESHLINK_INTERFACE_NAME} -o ${ROUTE_HOST_IF} -d ${ROUTED_SUBNET} -j ACCEPT 2>/dev/null || sudo iptables -I FORWARD 1 -i ${MESHLINK_INTERFACE_NAME} -o ${ROUTE_HOST_IF} -d ${ROUTED_SUBNET} -j ACCEPT; sudo iptables -C FORWARD -i ${ROUTE_HOST_IF} -o ${MESHLINK_INTERFACE_NAME} -s ${ROUTED_SUBNET} -j ACCEPT 2>/dev/null || sudo iptables -I FORWARD 1 -i ${ROUTE_HOST_IF} -o ${MESHLINK_INTERFACE_NAME} -s ${ROUTED_SUBNET} -j ACCEPT; sudo ip netns exec ${ROUTE_NAMESPACE} ping -c 1 -W 2 ${ROUTED_GATEWAY_IP} >/dev/null"
}

collect_validation_artifacts() {
  mkdir -p "$VALIDATION_STATE_DIR"
  read_windows_file "$MESHLINK_WINDOWS_SERVICE_CONFIG_DIR\\$MESHLINK_WINDOWS_INTERFACE_NAME.conf" \
    >"$VALIDATION_STATE_DIR/windows-${MESHLINK_WINDOWS_INTERFACE_NAME}.conf" 2>/dev/null || true
  windows_ps_script "Get-Content -Path '$MESHLINK_WINDOWS_PACKAGE_DIR\\meshlinkd.stdout.log' -Raw -ErrorAction Stop" \
    >"$VALIDATION_STATE_DIR/windows-meshlinkd.stdout.log" 2>/dev/null || true
  windows_ps_script "Get-Content -Path '$MESHLINK_WINDOWS_PACKAGE_DIR\\meshlinkd.stderr.log' -Raw -ErrorAction Stop" \
    >"$VALIDATION_STATE_DIR/windows-meshlinkd.stderr.log" 2>/dev/null || true
  windows_ps_script "Get-NetRoute -AddressFamily IPv4 | Sort-Object ifIndex,DestinationPrefix | Format-Table -AutoSize" \
    >"$VALIDATION_STATE_DIR/windows-routes.txt" 2>/dev/null || true
  windows_ps_script "Get-NetIPAddress -AddressFamily IPv4 | Sort-Object InterfaceAlias,IPAddress | Format-Table -AutoSize" \
    >"$VALIDATION_STATE_DIR/windows-ip-addresses.txt" 2>/dev/null || true
  windows_ps_script "Get-Service -Name 'WireGuardTunnel\$$MESHLINK_WINDOWS_INTERFACE_NAME' -ErrorAction SilentlyContinue | Format-List *" \
    >"$VALIDATION_STATE_DIR/windows-service.txt" 2>/dev/null || true
  ssh_to_vm client-a "sudo wg show ${MESHLINK_INTERFACE_NAME:-sdwan0}" \
    >"$VALIDATION_STATE_DIR/client-a-wg-show.txt" 2>/dev/null || true
  ssh_to_vm client-b "sudo wg show ${MESHLINK_INTERFACE_NAME:-sdwan0}" \
    >"$VALIDATION_STATE_DIR/client-b-wg-show.txt" 2>/dev/null || true
  ssh_to_vm client-a "sudo ip route show" >"$VALIDATION_STATE_DIR/client-a-routes.txt" 2>/dev/null || true
  ssh_to_vm client-b "sudo ip route show" >"$VALIDATION_STATE_DIR/client-b-routes.txt" 2>/dev/null || true
  ssh_to_vm client-a "cat $REMOTE_ROOT/config/client.toml" >"$VALIDATION_STATE_DIR/client-a.toml" 2>/dev/null || true
  ssh_to_vm client-b "cat $REMOTE_ROOT/config/client.toml" >"$VALIDATION_STATE_DIR/client-b.toml" 2>/dev/null || true
}

print_summary() {
  printf 'Windows VM embedded-runtime validation %s\n' "$1"
  printf 'direct: %s\n' "$RESULT_DIRECT"
  printf 'relay fallback: %s\n' "$RESULT_RELAY"
  printf 'direct recovery: %s\n' "$RESULT_RECOVERY"
  printf 'route advertisement: %s\n' "$RESULT_ROUTE_ADV"
  printf 'route withdrawal: %s\n' "$RESULT_ROUTE_WITHDRAW"
  printf 'artifacts: %s\n' "$VALIDATION_STATE_DIR"
}

cleanup_validation() {
  local exit_code=$?

  "$ROOT_DIR/tests/windows-vm/prepare-dual-nat.sh" clear >/dev/null 2>&1 || true
  ssh_to_vm client-a "sudo iptables -D FORWARD -i ${MESHLINK_INTERFACE_NAME:-sdwan0} -o ${ROUTE_HOST_IF} -d ${ROUTED_SUBNET} -j ACCEPT 2>/dev/null || true; sudo iptables -D FORWARD -i ${ROUTE_HOST_IF} -o ${MESHLINK_INTERFACE_NAME:-sdwan0} -s ${ROUTED_SUBNET} -j ACCEPT 2>/dev/null || true; sudo ip link del ${ROUTE_HOST_IF} 2>/dev/null || true; sudo ip netns del ${ROUTE_NAMESPACE} 2>/dev/null || true" >/dev/null 2>&1 || true
  collect_validation_artifacts || true
  if (( exit_code != 0 )); then
    print_summary "failed"
  fi
  trap - EXIT
  exit "$exit_code"
}

main() {
  load_windows_env
  load_lab_env
  require_dual_nat_topology
  require_local_commands

  REMOTE_ROOT="/home/${MESHLINK_SSH_USER}/meshlink"
  MESHLINK_INTERFACE_NAME="${MESHLINK_INTERFACE_NAME:-sdwan0}"
  MESHLINK_SIGNAL_PORT="${MESHLINK_SIGNAL_PORT:-10000}"
  MESHLINK_STUN_PORT="${MESHLINK_STUN_PORT:-3479}"
  MESHLINK_RELAY_PORT="${MESHLINK_RELAY_PORT:-3478}"
  MGMT_IP="$(vm_ip mgmt-1)"

  derive_validation_topology
  select_windows_validation_port
  trap cleanup_validation EXIT

  rm -rf "$VALIDATION_STATE_DIR"
  mkdir -p "$VALIDATION_STATE_DIR"
  printf '%s\n' "$MESHLINK_WINDOWS_WG_PORT" >"$VALIDATION_STATE_DIR/windows-listen-port.txt"

  windows_qga wait 60 >/dev/null
  stop_windows_meshlinkd
  wait_for_ssh mgmt-1 30
  setup_routed_subnet_target
  ensure_linux_clients_ready
  DIRECT_PEER_PUBLIC_KEY="$(linux_peer_public_key "$DIRECT_NODE")"
  RELAY_PEER_PUBLIC_KEY="$(linux_peer_public_key "$RELAY_NODE")"
  ROUTE_ADVERTISER_PUBLIC_KEY="$(linux_peer_public_key "$ROUTE_ADVERTISER_NODE")"
  generate_windows_keys

  refresh_windows_package
  deploy_windows_package >/dev/null
  write_windows_config >/dev/null
  restart_windows_meshlinkd >/dev/null

  wait_for_windows_service_running 40 >"$VALIDATION_STATE_DIR/windows-service-running.txt"
  wait_for_windows_interface_ready 40 >"$VALIDATION_STATE_DIR/windows-interface.txt"
  wait_for_windows_service_config 40 >"$VALIDATION_STATE_DIR/windows-config-file.txt"

  wait_for_windows_peer_present "$DIRECT_PEER_PUBLIC_KEY" 50
  wait_for_windows_peer_present "$RELAY_PEER_PUBLIC_KEY" 50

  "$ROOT_DIR/tests/windows-vm/prepare-dual-nat.sh" map >/dev/null

  wait_for_linux_peer_endpoint_host "$DIRECT_NODE" "$WINDOWS_PUBLIC_KEY" "$WINDOWS_IP" 50 \
    >"$VALIDATION_STATE_DIR/${DIRECT_NODE}-windows-direct-endpoint.txt"
  wait_for_windows_peer_endpoint_host "$DIRECT_PEER_PUBLIC_KEY" "$DIRECT_EXPECTED_HOST" 50 \
    >"$VALIDATION_STATE_DIR/windows-direct-endpoint.txt"
  RESULT_DIRECT="pass"

  "$ROOT_DIR/tests/windows-vm/prepare-dual-nat.sh" drop >/dev/null

  wait_for_windows_peer_endpoint_host "$RELAY_PEER_PUBLIC_KEY" "$MGMT_IP" 60 \
    >"$VALIDATION_STATE_DIR/windows-relay-endpoint.txt"
  wait_for_linux_peer_endpoint_host "$RELAY_NODE" "$WINDOWS_PUBLIC_KEY" "$MGMT_IP" 60 \
    >"$VALIDATION_STATE_DIR/${RELAY_NODE}-windows-relay-endpoint.txt"
  RESULT_RELAY="pass"

  "$ROOT_DIR/tests/windows-vm/prepare-dual-nat.sh" clear >/dev/null

  wait_for_linux_peer_endpoint_host "$RELAY_NODE" "$WINDOWS_PUBLIC_KEY" "$WINDOWS_PUBLIC_IP" 90 \
    >"$VALIDATION_STATE_DIR/${RELAY_NODE}-windows-recovery-linux-endpoint.txt"
  printf '%s\n' "$WINDOWS_PUBLIC_IP" >"$VALIDATION_STATE_DIR/windows-recovery-endpoint.txt"
  RESULT_RECOVERY="pass"

  wait_for_windows_peer_allowed_ip_state "$ROUTE_ADVERTISER_PUBLIC_KEY" "$ROUTED_SUBNET" present 60 \
    >"$VALIDATION_STATE_DIR/windows-route-allowed-ips.txt"
  wait_for_windows_route_state "$ROUTED_SUBNET" present 60 \
    >"$VALIDATION_STATE_DIR/windows-route-present.txt"
  wait_for_windows_ping_state "$ROUTED_TARGET_IP" present 25
  RESULT_ROUTE_ADV="pass"

  update_linux_client_routes "$ROUTE_ADVERTISER_NODE" ""
  restart_linux_client "$ROUTE_ADVERTISER_NODE"
  wait_for_linux_log "$ROUTE_ADVERTISER_NODE" "device registered" 40
  wait_for_windows_peer_allowed_ip_state "$ROUTE_ADVERTISER_PUBLIC_KEY" "$ROUTED_SUBNET" absent 60 \
    >"$VALIDATION_STATE_DIR/windows-route-allowed-ips-withdrawn.txt"
  wait_for_windows_route_state "$ROUTED_SUBNET" absent 60
  wait_for_windows_ping_state "$ROUTED_TARGET_IP" absent 25
  RESULT_ROUTE_WITHDRAW="pass"

  print_summary "passed"
}

main "$@"
