# Phase 06: Relay Fallback

## Goal

Build the relay fallback path on top of a repeatable dual-NAT VM lab, so relay development and recovery logic can be validated under controlled direct-path failure.

## Tasks

### TASK-010 Implement relay fallback and direct-path recovery on dual-NAT

Status: `done`

Required behavior:

1. `tests/nat-lab/create-lab.sh` supports a `dual-nat` topology with `mgmt-1`, `nat-a`, `nat-b`, `client-a`, and `client-b`.
2. `tests/nat-lab/common.sh` exposes fail-fast host-to-VM preflight checks and repeatable helpers to inject and clear peer-to-peer WireGuard UDP drop rules while keeping management, signaling, and relay services reachable.
3. `relayd` exposes authenticated relay reservation APIs and forwards raw WireGuard UDP for a reserved peer pair over a dedicated relay socket.
4. Linux clients reserve relay fallback after punch timeout or remote punch failure, switch WireGuard endpoint overrides to the relay socket, keep refreshing the reservation while active, and release the reservation after direct recovery.
5. `tests/nat-lab/run-phase06.sh` validates initial direct NAT-WAN convergence, forced relay fallback under injected packet loss, and automatic recovery back to direct after the fault is removed.

Verification:

1. `bash -n tests/nat-lab/common.sh tests/nat-lab/create-lab.sh tests/nat-lab/destroy-lab.sh tests/nat-lab/run-phase06.sh`
2. `cd server && go test ./...`
3. `cargo test --manifest-path client/Cargo.toml --workspace`
4. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase06.sh`

## Notes

1. Relay remains fallback-only; the steady-state target is still the direct hole-punched path.
2. Relay forwards raw WireGuard UDP only; no extra payload encryption layer is added because WireGuard already encrypts the data plane.
