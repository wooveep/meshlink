# Phase 04: VM Lab and Tests

## Goal

Build a repeatable QEMU/libvirt-based test lab for phase 01-03 acceptance and future NAT experiments.

## Tasks

### TASK-007 Build QEMU libvirt lab and smoke scenarios

Status: `done`

Required behavior:

1. A basic topology exists for `mgmt-1`, `client-a`, and `client-b`.
2. Acceptance commands are scripted.
3. Test-lab docs explain how to repeat phase 01-03 validation.

Deliverables:

1. `create-lab.sh` creates the three-node libvirt topology from a cloud image.
2. `destroy-lab.sh` tears down the topology and overlay disks.
3. `run-phase01-02.sh` copies host-built binaries into the VMs and verifies registration plus peer discovery.
4. `run-phase03.sh` validates static Linux WireGuard peering and overlay ping.
5. `tests/nat-lab/README.md` documents prerequisites, environment variables, and repeat steps.

Verification:

1. `bash -n tests/nat-lab/common.sh tests/nat-lab/create-lab.sh tests/nat-lab/destroy-lab.sh tests/nat-lab/run-phase01-02.sh tests/nat-lab/run-phase03.sh`
2. `./tests/nat-lab/run-phase01-02.sh` after configuring `tests/nat-lab/libvirt.env`
3. `./tests/nat-lab/run-phase03.sh` after configuring `tests/nat-lab/libvirt.env`
