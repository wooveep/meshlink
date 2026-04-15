# Phase 05: NAT Punching

## Goal

Establish a Linux-first dynamic direct path using self-hosted STUN, `SignalService`, candidate exchange, and runtime WireGuard endpoint override without recreating the interface.

## Tasks

### TASK-009 Linux NAT traversal via STUN + SignalService

Status: `done`

Required behavior:

1. `signald` accepts authenticated long-lived sessions and forwards candidate, punch-request, punch-result, and heartbeat messages.
2. `signald` exposes a minimal self-hosted STUN responder on UDP `3479`.
3. `meshlinkd` can collect LAN and public IPv4 candidates, exchange them over `SignalService`, and choose an initiator deterministically.
4. Linux clients can override a peer endpoint at runtime and reconcile WireGuard without recreating the interface.
5. Punch timeout clears the runtime override and falls back to the static endpoint when one exists.

Verification:

1. `cd server && go test ./...`
2. `cargo test --manifest-path client/Cargo.toml --workspace`
3. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh`

## Notes

1. `dual-nat` is now the primary acceptance topology for Phase 05 because it exercises STUN-derived public IPv4 candidates and NAT-WAN endpoint convergence.
2. `flat` remains available as a lighter regression path, but it is no longer the canonical Phase 05 verification target.
