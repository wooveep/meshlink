# Current Progress

## Current milestone

Phase 03 Linux direct tunnel now runs end to end, and the VM lab can verify both control-plane discovery and static WireGuard overlay traffic.

## Completed

1. Local Go and protobuf toolchain enabled.
2. `managementd` implemented with bootstrap token validation, in-memory device registry, overlay IPv4 allocation, and `SyncConfig` streaming.
3. `meshlinkd` implemented with config loading, registration, reconnect loop, and config stream handling.
4. `phase01-smoke.sh` verifies the server/client control-plane loop.
5. Project task files and dual Skills are in place.
6. Peer discovery now flows through `SyncConfig`, with stable revision ordering and per-device peer views.
7. The Rust client caches peer views and logs added, updated, and removed peers without touching WireGuard.
8. `phase02-smoke.sh` verifies two-client discovery over the live config stream.
9. A repeatable libvirt lab skeleton exists for `mgmt-1`, `client-a`, and `client-b`, with scripted create/destroy/acceptance commands.
10. Management and peer views now propagate optional static direct endpoints for Linux peering.
11. The Rust client can register a direct endpoint, reconcile a Linux WireGuard interface, and keep peer changes idempotent without recreating the interface.
12. `run-phase03.sh` verifies VM-to-VM static WireGuard peering plus overlay ping.

## Next

1. Start the STUN and signaling path now that the static Linux tunnel path is stable.
2. Add candidate collection and endpoint selection over `SignalService`.
3. Keep the relay path as a later fallback-only step.

## Risks

1. `INCREMENTAL` events still carry the latest full peer set rather than true patches.
2. Linux data-plane reconciliation shells out to `ip` and `wg`, so it must run with root privileges in the guest.
3. VM lab acceptance depends on a local cloud image and SSH key being configured in `tests/nat-lab/libvirt.env`.
