# Current Progress

## Current milestone

Control-plane delivery now includes static route publication over the existing
peer view, and the client runtime is being moved from external WireGuard tools
to embedded Linux and Windows integration paths.

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
29. `package-windows.sh` now expects pinned `tunnel.dll` and `wireguard.dll`
    inputs, and the repository includes a helper script to stage a source-built
    `tunnel.dll`.

## Next

1. Stage pinned Windows runtime DLLs under
   `deploy/packages/windows/runtime/v0.3.17/amd64/`.
2. Use the Windows VM package-first path to verify one Windows node against one
   Linux node for direct, relay fallback, route advertisement, and route
   withdrawal behavior.
3. Re-run the dual-NAT Linux regression path on a host that does not have
   `wireguard-tools` installed.
4. Move beyond “latest full view” `INCREMENTAL` events when the control-plane
   patch model is ready.

## Risks

1. `INCREMENTAL` events still carry the latest full peer set rather than true
   patches.
2. Linux data-plane reconciliation still shells out to `ip`, so it must run
   with root privileges in the guest and still assumes kernel WireGuard
   support.
3. The embedded Windows path is wired up in code and packaging, but real DLLs
   still need to be staged and validated on a Windows host or CI runner.
4. VM lab acceptance depends on a local cloud image and SSH key being
   configured in `tests/nat-lab/libvirt.env`.
5. `make package-deb` requires `nfpm` or a working `go run` fallback path to
   fetch `nfpm` on demand.
6. The deb validation path assumes the guest image has a working `systemd`,
   `journalctl`, `dpkg`, `iproute2`, and kernel WireGuard support.
7. Dual-NAT acceptance now fails fast on host-to-VM reachability issues; a
   broken libvirt/network state still requires destroy/recreate before the
   end-to-end scripts will run.
8. `make package-windows` requires either a Windows Rust target plus linker on
   the Linux host, or a prebuilt `meshlinkd.exe` supplied through
   `MESHLINK_WINDOWS_BINARY`, plus staged runtime DLLs.
