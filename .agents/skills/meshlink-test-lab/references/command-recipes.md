# Command Recipes

## Phase 01 local smoke

```bash
./tests/e2e/phase01-smoke.sh
```

## Manual server start

```bash
cd /home/cloudyi/code/meshlink/server
go run ./cmd/managementd -listen 127.0.0.1:33073
```

## Manual client start

```bash
cargo run --manifest-path /home/cloudyi/code/meshlink/client/Cargo.toml --bin meshlinkd -- \
  --config /home/cloudyi/code/meshlink/deploy/examples/client-config.toml
```

## VM lab bring-up

Read `references/vm-lab-usage.md` before changing the lab environment. The validated workflow is:

```bash
./tests/nat-lab/create-lab.sh
./tests/nat-lab/run-phase01-02.sh
./tests/nat-lab/destroy-lab.sh
```

## VM lab inspection

```bash
virsh list --all
virsh net-dhcp-leases default
```
