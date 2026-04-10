# VM Lab Usage

This reference captures the VM lab workflow that was actually validated in this repository on 2026-04-10.

Use it when you need to bring up or debug the three-node libvirt lab under `tests/nat-lab`.

## Validated Host Setup

1. Base image: `/home/cloudyi/Downloads/noble-server-cloudimg-amd64.img`
2. Storage pool: `Downloads`
3. Libvirt network: `default`
4. Guest SSH user: `ubuntu`
5. SSH public key: `/home/cloudyi/.ssh/id_ed25519.pub`

Why this shape:

1. The standard Ubuntu server cloud image consumed the NoCloud seed reliably.
2. The `Downloads` pool maps to a user-writable directory and avoids the permission issues we hit when trying to use media-mounted images directly as backing files.
3. The current scripts discover DHCP-assigned guest IPs dynamically, so do not assume the configured `192.168.122.x` placeholders will be the live guest addresses.

## Expected Environment File

The validated `tests/nat-lab/libvirt.env` shape is:

```bash
MESHLINK_BASE_IMAGE=/home/cloudyi/Downloads/noble-server-cloudimg-amd64.img
MESHLINK_LIBVIRT_NETWORK=default
MESHLINK_LIBVIRT_POOL=Downloads
MESHLINK_OS_VARIANT=ubuntu24.04
MESHLINK_SSH_USER=ubuntu
MESHLINK_SSH_PUBLIC_KEY_FILE=/home/cloudyi/.ssh/id_ed25519.pub
MESHLINK_MGMT_IP=192.168.122.201
MESHLINK_CLIENT_A_IP=192.168.122.202
MESHLINK_CLIENT_B_IP=192.168.122.203
MESHLINK_GATEWAY=192.168.122.1
MESHLINK_MANAGEMENT_PORT=33073
MESHLINK_SYNC_INTERVAL=1s
```

## Recommended Bring-Up Flow

From the repo root:

```bash
./tests/nat-lab/create-lab.sh
./tests/nat-lab/run-phase01-02.sh
./tests/nat-lab/destroy-lab.sh
```

What each command does:

1. `create-lab.sh`
Creates `meshlink-mgmt-1`, `meshlink-client-a`, and `meshlink-client-b` from the cloud image and a generated NoCloud seed ISO.
2. `run-phase01-02.sh`
Builds host artifacts, copies them into the guests, starts `managementd` and two `meshlinkd` instances, then verifies registration and peer discovery.
3. `destroy-lab.sh`
Removes the domains, overlay disks, and temporary lab state.

## Useful Verification Commands

Check VM state:

```bash
virsh list --all
```

Check DHCP leases on the libvirt network:

```bash
virsh net-dhcp-leases default
```

Run the VM acceptance script repeatedly:

```bash
./tests/nat-lab/run-phase01-02.sh
```

The current script supports repeated runs on the same VMs. It stops old processes, replaces binaries atomically, and downloads logs through the same SSH helper path it uses for deployment.

## Logs and Artifacts

Local collected logs:

1. `/tmp/meshlink-nat-lab/runtime/mgmt-1-managementd.log`
2. `/tmp/meshlink-nat-lab/runtime/client-a-meshlinkd.log`
3. `/tmp/meshlink-nat-lab/runtime/client-b-meshlinkd.log`

Guest-side working directory:

1. `/home/ubuntu/meshlink/bin`
2. `/home/ubuntu/meshlink/config`
3. `/home/ubuntu/meshlink/logs`

## Troubleshooting Notes

1. Prefer the standard Ubuntu server cloud image over the `minimal` cloud image. The minimal image did not reliably apply the injected NoCloud config in this environment.
2. If the source image lives under `/media/...`, copy it into `/home/cloudyi/Downloads/` before using it as a backing image for libvirt.
3. If you need SSH credentials from scratch, generate them with:

```bash
mkdir -p ~/.ssh
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519 -N ""
```

4. If the acceptance script fails, inspect the collected logs in `/tmp/meshlink-nat-lab/runtime/` before changing the topology.
5. If a task only needs Phase 01/02 validation, do not expand the lab into WireGuard or NAT traversal experiments yet. Keep the environment minimal and reproducible.
