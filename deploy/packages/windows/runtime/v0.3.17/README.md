# Windows Runtime Assets

This directory is reserved for the pinned WireGuard runtime consumed by
`scripts/package-windows.sh`.

Expected layout for the default `x86_64-pc-windows-gnu` target:

```text
deploy/packages/windows/runtime/v0.3.17/amd64/tunnel.dll
deploy/packages/windows/runtime/v0.3.17/amd64/wireguard.dll
```

Recommended provenance:

1. Build `tunnel.dll` from the official `wireguard-windows` repository's
   `embeddable-dll-service` source at `v0.3.17`.
2. Stage the matching `wireguard.dll` for the same runtime set under the same
   architecture directory.

`make package-windows` will refuse to build if these pinned assets are missing.
