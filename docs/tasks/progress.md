# Current Progress

## Current milestone

Control-plane delivery now includes static route publication over the existing
peer view, and the embedded Linux and Windows runtime transition is validated
end to end in the dual-NAT acceptance lab.

## Completed

1. Local Go and protobuf toolchain enabled.
2. `managementd` implements bootstrap-token validation, in-memory device
   registry, overlay IPv4 allocation, and `SyncConfig` streaming.
3. `meshlinkd` implements config loading, registration, reconnect loop, and
   config stream handling.
4. `phase01-smoke.sh` verifies the server/client control-plane loop.
5. Project task files and dual Skills are in place.
6. Peer discovery now flows through `SyncConfig`, with stable revision ordering
   and per-device peer views.
7. The Rust client caches peer views and logs added, updated, and removed peers
   without touching WireGuard.
8. `phase02-smoke.sh` verifies two-client discovery over the live config stream.
9. A repeatable libvirt lab skeleton exists for `mgmt-1`, `client-a`, and
   `client-b`, with scripted create/destroy/acceptance commands.
10. Management and peer views propagate optional static direct endpoints for
    Linux peering.
11. The Rust client can register a direct endpoint, reconcile a Linux WireGuard
    interface, and keep peer changes idempotent without recreating the
    interface.
12. `run-phase03.sh` verifies VM-to-VM static WireGuard peering plus overlay
    ping.
13. `make build-server` and `make build-client` produce release artifacts under
    `dist/bin/linux-amd64/`.
14. `make package-deb` produces `meshlink-managementd`, `meshlink-signald`,
    `meshlink-relayd`, and `meshlink-client` Debian packages under `dist/deb/`.
15. Packaged Linux delivery assets now include standard install paths, default
    config files, and systemd units for `managementd` and `meshlinkd`.
16. `run-phase03-deb.sh` installs the deb artifacts inside the VM lab, starts
    services through systemd, and verifies Phase 03 overlay connectivity without
    copying raw host binaries into the guests.
17. `signald` authenticates `SignalHello`, keeps one active session per device,
    forwards candidate and punch messages, and exposes a minimal STUN responder
    on UDP `3479`.
18. The Rust client can collect LAN/public IPv4 candidates, exchange them over
    `SignalService`, and apply runtime endpoint overrides during WireGuard
    reconciliation.
19. `run-phase05.sh` validates the signaling and runtime endpoint-override path
    in the VM lab, with `dual-nat` as the primary acceptance topology.
20. `relayd` exposes authenticated relay reservation APIs and forwards raw
    WireGuard UDP for a reserved peer pair over a dedicated session socket.
21. The Rust client falls back to relay after punch failure, refreshes relay
    reservations while active, and releases relay once direct recovery succeeds.
22. `run-phase06.sh` validates initial direct NAT-WAN convergence, relay
    fallback under injected peer-to-peer packet loss, and recovery back to
    direct after the fault is cleared.
23. `managementd` now stores `advertised_routes`, validates static IPv4 CIDRs,
    rejects default-route and overlap conflicts, and derives `Peer.allowed_ips`
    through an internal hook chain.
24. `meshlinkd` now republishes static `advertised_routes` on every registration
    and reuses the existing AllowedIPs reconciliation path for routed subnets.
25. `run-phase08-routes.sh` now validates route advertisement, relay continuity
    for routed subnet traffic, and route withdrawal cleanup in the dual-NAT lab.
26. `netlink-linux` now vendors the official `wireguard-tools` embeddable
    library and uses it for both interface reconciliation and latest-handshake
    inspection.
27. The Linux client package no longer models `wireguard-tools` as a required
    runtime dependency.
28. `meshlinkd.exe` now supports `/service <config>` and `wintun-windows` can
    manage a `WireGuardTunnel$<interface>` service around package-local runtime
    assets.
29. `package-windows.sh` now auto-stages pinned `tunnel.dll`,
    `wireguard.dll`, and `wintun.dll` inputs on Linux, and the repository
    includes both Linux and Windows helper scripts for runtime staging.
30. The pinned `amd64` runtime set is staged locally, and the refreshed Windows
    zip package now includes `meshlinkd.exe`, `tunnel.dll`, `wireguard.dll`,
    and `wintun.dll`.
31. Manual Windows VM validation identified `wintun.dll` as a missing runtime
    dependency, after which a refreshed package booted the embedded
    `WireGuardTunnel$MeshLink` service successfully and restored the `MeshLink`
    overlay interface on reboot.
32. `run-phase05.sh` now provisions `relayd` plus `relay_addr` for Linux
    dual-NAT clients, and `run-phase08-validation.sh` provides a repeatable
    Windows VM acceptance path for package deployment, same-side direct
    peering, and opposite-side relay fallback.
33. `run-phase08-validation.sh` now completes the full Windows embedded-runtime
    acceptance path: package refresh and QGA deploy, direct connectivity, relay
    fallback, direct recovery, route advertisement, and route withdrawal.
34. `run-phase08-routes.sh` now records guest runtime state alongside the Linux
    route regression so the guest path proves it no longer depends on
    `wireguard-tools`, while the host harness may still use local `wg` helpers
    for test key generation and inspection.
35. `SyncConfig` now sends real peer patches on `INCREMENTAL` events through
    `peer_upserts` and `removed_peer_ids`, while clients reconcile WireGuard
    state from their cache's converged view instead of treating incremental
    payloads as full snapshots.

## Next

1. Add ACL, policy, and more granular route-distribution controls on top of the
   current route advertisement baseline.
2. Improve observability, upgrade flow, and long-running stability coverage for
   the embedded runtime path.
3. Add targeted integration coverage for rolling upgrades where older clients
   still emit or consume legacy full-view incremental events.

## Risks

1. Linux data-plane reconciliation still shells out to `ip`, so it must run
   with root privileges in the guest and still assumes kernel WireGuard
   support.
2. The Linux “no install” promise now applies to the guest runtime path; the
   host-side acceptance harness may still use local `wg` helpers for key
   generation and inspection.
3. VM lab acceptance depends on a local cloud image and SSH key being
   configured in `tests/nat-lab/libvirt.env`.
4. `make package-deb` requires `nfpm` or a working `go run` fallback path to
   fetch `nfpm` on demand.
5. The deb validation path assumes the guest image has a working `systemd`,
   `journalctl`, `dpkg`, `iproute2`, and kernel WireGuard support.
6. Dual-NAT acceptance now fails fast on host-to-VM reachability issues; a
   broken libvirt/network state still requires destroy/recreate before the
   end-to-end scripts will run.
7. `make package-windows` requires either a Windows Rust target plus linker on
   the Linux host, or a prebuilt `meshlinkd.exe` supplied through
   `MESHLINK_WINDOWS_BINARY`, plus staged runtime DLLs.
