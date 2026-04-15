MeshLink Windows package

Contents:
- meshlinkd.exe
- client.toml
- run-meshlinkd.ps1
- tunnel.dll
- wireguard.dll

Prerequisites:
1. Run MeshLink from an elevated PowerShell session so the embedded tunnel-service path can install and update the WireGuard service.
2. Keep `meshlinkd.exe`, `tunnel.dll`, and `wireguard.dll` in the same extracted directory.
3. Edit client.toml before first start.

Recommended first run:
  powershell.exe -ExecutionPolicy Bypass -File .\run-meshlinkd.ps1

For a libvirt validation flow, see:
  tests/windows-vm/README.md
