# Phase 00: Foundation

## Goal

Create a stable repository, tooling, task tracking, and AI workflow base so feature work can proceed without rewriting process artifacts.

## Tasks

### TASK-000 Bootstrap Go and Protobuf toolchain

Status: `done`

Deliverables:

1. Local `go`, `protoc`, `protoc-gen-go`, `protoc-gen-go-grpc`
2. Working `scripts/gen-proto.sh`
3. Go module dependencies resolved

### TASK-003 Add project task files and AI progress sources

Status: `done`

Deliverables:

1. `docs/tasks/index.yaml`
2. Phase task documents
3. `docs/tasks/progress.md`

### TASK-004 Create dual Skills for development and VM test lab

Status: `done`

Deliverables:

1. `meshlink-sdwan-dev`
2. `meshlink-test-lab`
3. Agent metadata and references for both

### TASK-008 Produce Debian build artifacts for current deliverables

Status: `done`

Deliverables:

1. `make build-server`, `make build-client`, and `make package-deb`
2. `dist/bin/linux-amd64/` and `dist/deb/` artifact layout
3. `nfpm` package definitions for `meshlink-managementd`, `meshlink-signald`, `meshlink-relayd`, and `meshlink-client`
4. Packaged systemd assets and default config files for current Linux delivery targets
