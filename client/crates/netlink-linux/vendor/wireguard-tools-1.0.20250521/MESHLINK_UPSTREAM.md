# Vendored Source

- Upstream project: `wireguard-tools`
- Version: `1.0.20250521`
- Source tarball: `https://download.wireguard.com/qemu-test/distfiles/wireguard-tools-1.0.20250521.tar.xz`
- Included files:
  - `COPYING`
  - `contrib/embeddable-wg-library/README`
  - `contrib/embeddable-wg-library/wireguard.c`
  - `contrib/embeddable-wg-library/wireguard.h`

MeshLink vendors only the Linux embeddable WireGuard UAPI shim from the official
release tarball so the client can configure kernel WireGuard interfaces without a
runtime dependency on `wg(8)` / `wireguard-tools`.
