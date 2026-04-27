MeshLink Windows package

Contents:
- meshlinkd.exe
- client.example.toml
- client.state.json (generated on first start)
- run-meshlinkd.ps1
- tunnel.dll
- wireguard.dll
- wintun.dll

Prerequisites:
1. Run MeshLink from an elevated PowerShell session so the embedded tunnel-service path can install and update the WireGuard service.
2. Keep `meshlinkd.exe`, `tunnel.dll`, `wireguard.dll`, and `wintun.dll` in the same extracted directory.
3. Copy `client.example.toml` to `client.toml` before first start.
4. At minimum, set `management_addr` in `client.toml`.
5. `private_key` and `public_key` will be generated into `client.state.json` automatically on first start.

Recommended first run:
  powershell.exe -ExecutionPolicy Bypass -File .\run-meshlinkd.ps1

For a libvirt validation flow, see:
  tests/windows-vm/README.md
