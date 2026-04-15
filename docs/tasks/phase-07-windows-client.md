# Phase 07: Windows Client Transition

## Goal

Capture the now-complete transition milestone where Windows joined the existing
control-plane, path-selection, relay fallback, and route-publication model
before the embedded-runtime work began.

## Tasks

### TASK-015 Preserve the external-WireGuard Windows minimum path as a transition milestone

Status: `done`

Required behavior:

1. `agent-core` stays platform-neutral and continues to drive registration,
   `SyncConfig`, `SignalService`, relay fallback, and direct-path recovery.
2. Windows reuses the same `Peer.allowed_ips` semantics as Linux, including
   static routed subnets published by peers.
3. The original `wintun-windows` backend proved out a minimal route-aware
   tunnel-service reconcile path against the same peer model used by Linux.
4. No separate Windows-only route publication API was introduced;
   `RegisterDevice.advertised_routes` remained the only public route-publish
   surface.

Verification:

1. `cargo test --manifest-path client/Cargo.toml --workspace`
2. Existing Windows package-first validation notes remain available as a
   baseline for the embedded-runtime follow-up.

## Notes

1. Phase 07 is now treated as a transition milestone, not the final Windows
   runtime architecture.
2. Embedded runtime work now lives in
   `docs/tasks/phase-08-embedded-wireguard-runtime.md`.
