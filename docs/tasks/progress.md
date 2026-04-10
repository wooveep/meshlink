# Current Progress

## Current milestone

Phase 02 client discovery is running end to end, and the first libvirt VM lab slice is ready for Phase 01/02 verification.

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

## Next

1. Implement `TASK-006` Linux direct tunnel setup over the newly discovered peer set.
2. Extend the VM lab from Phase 01/02 verification to Phase 03 overlay tunnel checks.
3. Start the STUN and signaling path once the static Linux tunnel path is stable.

## Risks

1. `INCREMENTAL` events still carry the latest full peer set rather than true patches.
2. WireGuard data plane is still a stub until `TASK-006`.
3. VM lab acceptance depends on a local cloud image and SSH key being configured in `tests/nat-lab/libvirt.env`.
