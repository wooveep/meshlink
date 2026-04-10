# Phase 03: Linux Direct Tunnel

## Goal

Establish a Linux-to-Linux WireGuard tunnel using a static reachable endpoint, without NAT traversal.

## Tasks

### TASK-006 Establish Linux direct tunnel without NAT

Status: `done`

Required behavior:

1. `wg-manager` can create the interface and upsert a peer.
2. `netlink-linux` owns platform-specific setup.
3. Two Linux clients can exchange overlay traffic.
4. Peer updates do not restart the entire interface.

Verification:

1. `cd server && go test ./...`
2. `cargo test --manifest-path client/Cargo.toml --workspace`
3. `./scripts/test-e2e.sh`
4. `./tests/nat-lab/run-phase03.sh`
