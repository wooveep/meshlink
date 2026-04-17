# Windows Runtime Assets

This directory is reserved for the pinned WireGuard runtime consumed by
`scripts/package-windows.sh`.

Expected layout for the default `x86_64-pc-windows-gnu` target:

```text
deploy/packages/windows/runtime/v0.3.17/amd64/tunnel.dll
deploy/packages/windows/runtime/v0.3.17/amd64/wireguard.dll
deploy/packages/windows/runtime/v0.3.17/amd64/wintun.dll
```

Recommended provenance:

1. Run `scripts/build-wireguard-windows-runtime.sh` on a Linux host with Go,
   MinGW, `curl`, and `unzip` available.
2. Or, build `tunnel.dll` from the official `wireguard-windows` repository's
   `embeddable-dll-service` source at `v0.3.17` and pair it with the matching
   `wireguard.dll` from the official `wireguard-nt` SDK and `wintun.dll` from
   the official `wintun` release.

`make package-windows` now auto-stages the pinned runtime on Linux if the DLLs
are missing and the helper script is available.
