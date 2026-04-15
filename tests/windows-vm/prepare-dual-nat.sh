#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ACTION="${1:-map}"
MESHLINK_LAB_TOPOLOGY="${MESHLINK_LAB_TOPOLOGY:-dual-nat}"

# shellcheck disable=SC1091
source "$ROOT_DIR/tests/nat-lab/common.sh"

load_lab_env
require_dual_nat_topology

MESHLINK_WINDOWS_SIDE="${MESHLINK_WINDOWS_SIDE:-nat-a}"
MESHLINK_WINDOWS_WG_PORT="${MESHLINK_WINDOWS_WG_PORT:-51820}"

case "$MESHLINK_WINDOWS_SIDE" in
  nat-a)
    NAT_NODE="nat-a"
    WINDOWS_IP="${MESHLINK_WINDOWS_IP:-10.10.1.20}"
    WINDOWS_PUBLIC_IP="$MESHLINK_NAT_A_WAN_IP"
    PEER_WAN_IP="$MESHLINK_NAT_B_WAN_IP"
    PEER_PORT="${MESHLINK_PEER_WG_PORT:-$MESHLINK_CLIENT_B_WG_PORT}"
    ;;
  nat-b)
    NAT_NODE="nat-b"
    WINDOWS_IP="${MESHLINK_WINDOWS_IP:-10.10.2.20}"
    WINDOWS_PUBLIC_IP="$MESHLINK_NAT_B_WAN_IP"
    PEER_WAN_IP="$MESHLINK_NAT_A_WAN_IP"
    PEER_PORT="${MESHLINK_PEER_WG_PORT:-$MESHLINK_CLIENT_A_WG_PORT}"
    ;;
  *)
    echo "unsupported MESHLINK_WINDOWS_SIDE: $MESHLINK_WINDOWS_SIDE" >&2
    exit 1
    ;;
esac

case "$ACTION" in
  map)
    ensure_udp_source_port_preserving_nat "$NAT_NODE" "$WINDOWS_IP" "$WINDOWS_PUBLIC_IP" "$MESHLINK_WINDOWS_WG_PORT"
    ensure_udp_destination_port_forward "$NAT_NODE" "$WINDOWS_IP" "$MESHLINK_WINDOWS_WG_PORT"
    echo "dual-NAT port mapping prepared for $MESHLINK_WINDOWS_SIDE ($WINDOWS_IP:$MESHLINK_WINDOWS_WG_PORT)"
    ;;
  drop)
    install_phase06_drop_rule "$NAT_NODE" "$PEER_WAN_IP" "$PEER_PORT" "$WINDOWS_IP" "$MESHLINK_WINDOWS_WG_PORT" "meshlink-windows-drop"
    echo "peer-to-peer WireGuard UDP drop rule installed for $MESHLINK_WINDOWS_SIDE"
    ;;
  clear)
    clear_phase06_drop_rule "$NAT_NODE" "$PEER_WAN_IP" "$PEER_PORT" "$WINDOWS_IP" "$MESHLINK_WINDOWS_WG_PORT" "meshlink-windows-drop"
    echo "peer-to-peer WireGuard UDP drop rule cleared for $MESHLINK_WINDOWS_SIDE"
    ;;
  *)
    echo "usage: $0 [map|drop|clear]" >&2
    exit 1
    ;;
esac
