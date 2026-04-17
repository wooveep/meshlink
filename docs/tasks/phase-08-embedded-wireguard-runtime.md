# Phase 08: Embedded WireGuard Runtime

## Goal

Remove the client's dependency on externally installed WireGuard tooling by
switching Linux to the official embedded kernel-control library and moving the
Windows path to an embedded tunnel-service runtime packaged with MeshLink.

## Tasks

### TASK-017 Reframe planning and task files around the embedded runtime phase

Status: `done`

Required behavior:

1. Add a dedicated Phase 08 task file for embedded runtime work.
2. Refresh `docs/tasks/index.yaml`, `docs/tasks/remaining-work.md`,
   `docs/tasks/progress.md`, `docs/roadmap/execution-plan.md`, and `README.md`
   so they no longer treat the external WireGuard Windows path as the final
   target.
3. Keep protocol and service-contract ownership unchanged; this phase is a
   runtime and packaging evolution, not a control-plane contract change.

Verification:

1. `rg -n "TASK-017|embedded runtime|Phase 08" docs/tasks docs/roadmap README.md`

### TASK-018 Replace Linux `wg(8)` shell-outs with the embedded WireGuard UAPI library

Status: `done`

Required behavior:

1. `netlink-linux` vendors the official `wireguard-tools` embeddable library
   from a pinned upstream release.
2. Linux interface reconciliation still uses `ip` for link, address, and route
   management, but no longer shells out to `wg syncconf`.
3. Linux handshake inspection no longer depends on `wg show latest-handshakes`.
4. The Debian client package no longer declares `wireguard-tools` as a runtime
   dependency.

Verification:

1. `cargo test --manifest-path client/Cargo.toml --workspace`
2. `rg -n "wireguard-tools" client/crates/netlink-linux deploy/packages/nfpm/meshlink-client.yaml`

### TASK-019 Add the embedded Windows tunnel-service runtime path

Status: `done`

Required behavior:

1. `wintun-windows` writes the stable tunnel config, validates package-local
   runtime assets, and manages a `WireGuardTunnel$<interface>` service instead
   of invoking `wireguard.exe`.
2. `meshlinkd.exe` supports `/service <config>` so the embedded tunnel service
   can call into `WireGuardTunnelService`.
3. The Windows runtime assets are treated as pinned package inputs rather than a
   host prerequisite.

Verification:

1. `cargo test --manifest-path client/Cargo.toml --workspace`
2. Manual Windows validation with staged `tunnel.dll`, `wireguard.dll`, and
   `wintun.dll`

### TASK-020 Pin and stage Windows runtime assets in the packaging flow

Status: `done`

Required behavior:

1. `scripts/package-windows.sh` requires staged `tunnel.dll` and
   `wireguard.dll` and `wintun.dll` from a fixed versioned directory or
   explicit env overrides.
2. The repository documents the pinned runtime layout and the Linux/Windows
   build-staging flow for the runtime DLL set.
3. Package docs describe runtime provenance, not a preinstalled WireGuard host
   dependency.
4. The pinned `amd64` runtime set is staged and the refreshed package now ships
   all three DLLs.

Verification:

1. `bash -n scripts/package-windows.sh`
2. `rg -n "tunnel.dll|wireguard.dll|wintun.dll|runtime/v0.3.17" scripts deploy README.md tests/windows-vm`

### TASK-021 Validate Linux and Windows embedded-runtime regression paths

Status: `done`

Required behavior:

1. Reuse the existing dual-NAT lab to re-check Linux direct, relay fallback,
   recovery, and routed-subnet behavior without `wireguard-tools`.
2. Re-run the Windows VM path with package-local runtime DLLs rather than a
   preinstalled WireGuard application, using `run-phase08-validation.sh` as the
   canonical scripted acceptance entrypoint.
3. Confirm route advertisement and route withdrawal continue to behave the same
   across Linux and Windows.
4. Record the Linux guest/runtime distinction clearly: the guest path no longer
   depends on `wireguard-tools`, but the host acceptance harness may still use
   local `wg` helpers for test key generation and inspection.

Verification:

1. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase08-routes.sh`
2. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/windows-vm/run-phase08-validation.sh`

## Notes

1. Linux still assumes kernel WireGuard support and root privileges; only the
   userspace `wireguard-tools` dependency is being removed.
2. Windows now targets the official `embeddable-dll-service` integration path.
3. The pinned Windows runtime set now includes `wintun.dll` in addition to
   `tunnel.dll` and `wireguard.dll`.
4. Route advertisement remains control-plane driven and ACL-free in this phase;
   later policy work can narrow visibility without changing the client wire
   format.
5. The Windows validation script may pick an alternate guest listen port inside
   the dual-NAT lab to avoid same-side NAT port collisions with Linux guests.
6. Manual Windows validation remains useful for deeper debugging, but scripted
   acceptance now covers direct, relay fallback, recovery, route advertisement,
   and route withdrawal.
