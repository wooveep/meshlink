# Protocol Checklist

Use this checklist when changing `proto/` or any RPC behavior.

1. Update `.proto` files first.
2. Regenerate Go stubs with `./scripts/gen-proto.sh`.
3. Ensure Rust still builds via `tonic_build`.
4. Update `docs/api/service-contracts.md`.
5. Update any architecture docs affected by semantic changes.
6. If the change affects task sequencing, update `docs/tasks/`.
