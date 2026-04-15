# Remaining Work

## Goal

Keep the remaining delivery sequence decision-complete so the project finishes
the embedded WireGuard runtime transition without reopening protocol or route
distribution design.

## Tasks

### TASK-011 Create the ordered execution file and refresh stale roadmap docs

Status: `done`

Deliverables:

1. `docs/tasks/remaining-work.md`
2. Refreshed `docs/tasks/index.yaml`, `docs/tasks/progress.md`,
   `docs/roadmap/execution-plan.md`, `README.md`, and phase task docs
3. Remaining work explicitly ordered after route publication landed

Verification:

1. `test -f docs/tasks/remaining-work.md`
2. `rg -n "TASK-011|TASK-017|embedded runtime|Windows" docs/tasks docs/roadmap README.md`

### TASK-012 Add managementd hook chain and route advertisement metadata

Status: `done`

Deliverables:

1. `Device.advertised_routes` and `RegisterDeviceRequest.advertised_routes`
2. Route normalization and overlap validation in `managementd`
3. `peer` allowed-IPs hook chain with `static_route_advertiser`

Verification:

1. `cd server && go test ./...`

### TASK-013 Add client static route publication

Status: `done`

Deliverables:

1. `client.toml` supports `advertised_routes`
2. `meshlinkd` sends static routes on every registration
3. Existing reconciliation keeps using `Peer.allowed_ips`

Verification:

1. `cargo test --manifest-path client/Cargo.toml --workspace`

### TASK-014 Add route publication acceptance and regression path

Status: `done`

Deliverables:

1. `tests/nat-lab/run-phase08-routes.sh`
2. Route publication, relay fallback, recovery, and withdrawal checks on
   dual-NAT
3. `make vm-lab-phase08`

Verification:

1. `bash -n tests/nat-lab/common.sh tests/nat-lab/run-phase05.sh tests/nat-lab/run-phase06.sh tests/nat-lab/run-phase08-routes.sh`
2. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase08-routes.sh`

### TASK-015 Preserve the external-WireGuard Windows minimum path as a transition milestone

Status: `done`

Deliverables:

1. Shared route semantics across Linux and Windows
2. A first `wintun-windows` backend using the external WireGuard tunnel-service
   path
3. Baseline Windows + Linux validation notes for the embedded-runtime follow-up

Verification:

1. `cargo test --manifest-path client/Cargo.toml --workspace`

### TASK-016 Add Windows VM validation path and zip packaging

Status: `done`

Deliverables:

1. `scripts/package-windows.sh` produces a Windows client zip payload under
   `dist/windows/`
2. `deploy/packages/windows/` carries the packaged config template, runner, and
   README assets
3. `tests/windows-vm/` documents how to create a Windows VM on libvirt and
   prepare dual-NAT validation

Verification:

1. `bash -n scripts/package-windows.sh tests/windows-vm/create-vm.sh tests/windows-vm/prepare-dual-nat.sh`
2. `rg -n "package-windows|windows-vm" README.md docs/tasks tests/windows-vm`

### TASK-017 Reframe planning and task files around the embedded runtime phase

Status: `done`

Deliverables:

1. `docs/tasks/phase-08-embedded-wireguard-runtime.md`
2. Updated task index, roadmap, progress, README, and phase docs
3. Phase 07 marked as a transition milestone rather than the final Windows
   runtime shape

Verification:

1. `rg -n "TASK-017|embedded runtime|Phase 08" docs/tasks docs/roadmap README.md`

### TASK-018 Replace Linux `wg(8)` shell-outs with the embedded WireGuard UAPI library

Status: `done`

Deliverables:

1. Vendored official `wireguard-tools` embeddable library in `netlink-linux`
2. Linux reconciliation through FFI instead of `wg syncconf`
3. Linux latest-handshake lookup through the same embedded UAPI path
4. Debian package metadata without `wireguard-tools`

Verification:

1. `cargo test --manifest-path client/Cargo.toml --workspace`
2. `rg -n "wireguard-tools|latest_handshake_timestamp" client deploy`

### TASK-019 Add the embedded Windows tunnel-service runtime path

Status: `in_progress`

Deliverables:

1. `meshlinkd.exe /service <config>` entrypoint for `WireGuardTunnelService`
2. `wintun-windows` service install/update flow using `WireGuardTunnel$<interface>`
3. Package-local runtime asset checks for `tunnel.dll` and `wireguard.dll`

Verification:

1. `cargo test --manifest-path client/Cargo.toml --workspace`
2. Manual Windows validation with staged runtime DLLs

### TASK-020 Pin and stage Windows runtime assets in the packaging flow

Status: `in_progress`

Deliverables:

1. Versioned runtime asset layout under `deploy/packages/windows/runtime/`
2. `scripts/package-windows.sh` support for staged DLLs and explicit overrides
3. A Windows build/staging helper for source-built `tunnel.dll`

Verification:

1. `bash -n scripts/package-windows.sh`
2. `pwsh -NoProfile -File scripts/build-wireguard-windows-runtime.ps1 -?`

### TASK-021 Validate Linux and Windows embedded-runtime regression paths

Status: `pending`

Deliverables:

1. Linux regression pass without `wireguard-tools`
2. Windows VM validation against one Linux peer using package-local runtime DLLs
3. Direct, relay, route advertisement, and route withdrawal checks captured for
   the embedded-runtime path

Verification:

1. `MESHLINK_LAB_TOPOLOGY=dual-nat ./tests/nat-lab/run-phase08-routes.sh`
2. Manual Windows VM validation against one Linux peer
