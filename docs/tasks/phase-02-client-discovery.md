# Phase 02: Client Discovery

## Goal

Propagate peer visibility through `SyncConfig` so two clients can discover each other without creating tunnels.

## Tasks

### TASK-005 Build client discovery over SyncConfig

Status: `done`

Required behavior:

1. Server derives peer views from the device registry.
2. Clients receive full and incremental peer updates.
3. Two clients see each other within a bounded delay.
4. No WireGuard interface changes happen in this phase.

Verification:

1. `cd server && go test ./...`
2. `cargo test --manifest-path client/Cargo.toml --workspace`
3. `./tests/e2e/phase02-smoke.sh`
