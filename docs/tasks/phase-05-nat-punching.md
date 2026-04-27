# Phase 05: NAT Punching

## Goal

Establish a Linux-first dynamic direct path using self-hosted STUN, `SignalService`, candidate exchange, and runtime WireGuard endpoint override without recreating the interface.

## Tasks

### TASK-009 Linux NAT traversal via STUN + SignalService

Status: `done`

Required behavior:

1. `signald` accepts authenticated long-lived sessions and forwards candidate, punch-request, punch-result, and heartbeat messages.
2. `signald` exposes a minimal self-hosted STUN responder on UDP `3479`.
3. `meshlinkd` uses a long-lived `PunchSocket` runtime to collect provisional public IPv4 candidates, exchange them over `SignalService`, and choose an initiator deterministically.
4. Linux clients can promote peer-observed source candidates fast enough to re-announce corrected public endpoints during the active punch window.
5. Linux clients can override a peer endpoint at runtime, drive direct probes through the punch runtime, and reconcile WireGuard without recreating the interface.
6. Punch timeout clears the runtime override and falls back to the static endpoint when one exists.

Verification:

1. `cd server && go test ./...`
2. `cargo test --manifest-path client/Cargo.toml --workspace`
3. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase05.sh`

## Notes

1. `dual-nat` is now the primary acceptance topology for Phase 05 because it exercises STUN-derived public IPv4 candidates and NAT-WAN endpoint convergence.
2. `flat` remains available as a lighter regression path, but it is no longer the canonical Phase 05 verification target.
3. The original dual-NAT closure gap and packet-level evidence are documented in `docs/tasks/phase-05-dual-nat-root-cause.md`.
4. `dual-nat` acceptance now only requires explicit punch execution plus final
   handshake convergence; observed-candidate correction is useful telemetry,
   not a mandatory log on every successful run.
5. The final Linux closure path keeps kernel WireGuard as the crypto/data
   plane and inserts a userspace public UDP proxy in front of it. The proxy
   owns the public same-port socket, forwards inbound/outbound encrypted UDP
   between WAN and loopback sockets, and emits runtime observed-source
   telemetry without requiring a userspace same-port WireGuard receiver.
6. The last dual-NAT blocker was a convergence bug in `remote_direct_progress`:
   `PunchResult.selected_candidate` reflects the candidate chosen by the peer
   for the local device, not the remote endpoint that should be installed in
   the local proxy. Reusing it as a proxy remote mapping caused post-handshake
   packets to be dropped until the acceptance test failed.
7. Verified on 2026-04-20 with `MESHLINK_LAB_TOPOLOGY=dual-nat
   ./tests/nat-lab/run-phase05.sh`, which now completes with end-to-end
   overlay ping success.
