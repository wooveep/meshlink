# Phase 01: Bootstrap Connectivity

## Goal

Run the minimum control-plane loop: server listen, client connect, bootstrap registration, overlay allocation, and long-lived config stream.

## Tasks

### TASK-001 Implement ManagementService RegisterDevice and SyncConfig

Status: `done`

Acceptance:

1. The server listens on a TCP address.
2. Registration requires a bootstrap token.
3. The same public key gets the same device identity and overlay IPv4.
4. `SyncConfig` sends an initial full event and heartbeat-style incremental events.

### TASK-002 Implement Rust client registration and sync loop

Status: `done`

Acceptance:

1. The client loads a local config file.
2. The client registers through gRPC.
3. The client opens and maintains the config stream.
4. The client reconnects after stream failure or server restart.

## Verification

1. `cd server && go test ./...`
2. `cargo test --manifest-path client/Cargo.toml --workspace`
3. `./tests/e2e/phase01-smoke.sh`
