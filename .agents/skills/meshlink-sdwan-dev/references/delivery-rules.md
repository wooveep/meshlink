# Delivery Rules

## Mandatory Rules

1. Protocol changes require synchronized updates to `proto/` and `docs/api/`.
2. Module boundary changes require synchronized updates to `docs/architecture/` and `docs/ai/module-boundaries.md`.
3. Keep Linux as the first fully supported platform until the core path is stable.
4. Keep Relay as fallback only.
5. Prefer small vertical slices that can be verified.

## Definition of Done

1. The change has a clear owner module.
2. The contract surface is explicit.
3. The verification path is stated.
4. The surrounding docs are not left stale.
