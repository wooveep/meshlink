#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/windows"
ASSET_DIR="$ROOT_DIR/deploy/packages/windows"
CLIENT_MANIFEST="$ROOT_DIR/client/Cargo.toml"
DEFAULT_TARGET="x86_64-pc-windows-gnu"
DEFAULT_RUNTIME_VERSION="v0.3.17"

derive_version() {
  awk '
    /^\[workspace.package\]$/ { in_workspace = 1; next }
    /^\[/ { in_workspace = 0 }
    in_workspace && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "$CLIENT_MANIFEST"
}

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "required file does not exist: $path" >&2
    exit 1
  fi
}

require_windows_target() {
  local target="$1"
  if ! rustup target list --installed | grep -qx "$target"; then
    echo "rust target '$target' is not installed" >&2
    echo "run: rustup target add $target" >&2
    exit 1
  fi
}

require_windows_linker() {
  local target="$1"
  case "$target" in
    x86_64-pc-windows-gnu)
      if [[ -n "${CC_x86_64_pc_windows_gnu:-}" ]]; then
        return
      fi
      if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
        echo "missing linker for $target" >&2
        echo "install x86_64-w64-mingw32-gcc or export CC_x86_64_pc_windows_gnu" >&2
        exit 1
      fi
      ;;
  esac
}

build_windows_binary() {
  local target="$1"
  require_windows_target "$target"
  require_windows_linker "$target"

  echo "Building meshlinkd for $target" >&2
  cargo build \
    --manifest-path "$CLIENT_MANIFEST" \
    --release \
    --bin meshlinkd \
    --target "$target"

  printf '%s\n' "$ROOT_DIR/client/target/$target/release/meshlinkd.exe"
}

runtime_arch_dir() {
  local target="$1"
  case "$target" in
    x86_64-pc-windows-gnu) printf '%s\n' "amd64" ;;
    i686-pc-windows-gnu) printf '%s\n' "x86" ;;
    aarch64-pc-windows-gnu) printf '%s\n' "arm64" ;;
    *)
      echo "unsupported windows runtime target: $target" >&2
      exit 1
      ;;
  esac
}

MESHLINK_WINDOWS_TARGET="${MESHLINK_WINDOWS_TARGET:-$DEFAULT_TARGET}"
MESHLINK_WINDOWS_BINARY="${MESHLINK_WINDOWS_BINARY:-}"
MESHLINK_WINDOWS_PACKAGE_VERSION="${MESHLINK_WINDOWS_PACKAGE_VERSION:-$(derive_version)}"
MESHLINK_WINDOWS_PACKAGE_NAME="${MESHLINK_WINDOWS_PACKAGE_NAME:-meshlink-client}"
MESHLINK_WINDOWS_RUNTIME_VERSION="${MESHLINK_WINDOWS_RUNTIME_VERSION:-$DEFAULT_RUNTIME_VERSION}"

RUNTIME_ARCH="$(runtime_arch_dir "$MESHLINK_WINDOWS_TARGET")"
DEFAULT_RUNTIME_DIR="$ASSET_DIR/runtime/$MESHLINK_WINDOWS_RUNTIME_VERSION/$RUNTIME_ARCH"
MESHLINK_WINDOWS_TUNNEL_DLL="${MESHLINK_WINDOWS_TUNNEL_DLL:-$DEFAULT_RUNTIME_DIR/tunnel.dll}"
MESHLINK_WINDOWS_WIREGUARD_DLL="${MESHLINK_WINDOWS_WIREGUARD_DLL:-$DEFAULT_RUNTIME_DIR/wireguard.dll}"

require_file "$ASSET_DIR/client.toml"
require_file "$ASSET_DIR/README.txt"
require_file "$ASSET_DIR/run-meshlinkd.ps1"
require_file "$MESHLINK_WINDOWS_TUNNEL_DLL"
require_file "$MESHLINK_WINDOWS_WIREGUARD_DLL"

if [[ -z "$MESHLINK_WINDOWS_BINARY" ]]; then
  MESHLINK_WINDOWS_BINARY="$(build_windows_binary "$MESHLINK_WINDOWS_TARGET")"
fi

require_file "$MESHLINK_WINDOWS_BINARY"

STAGE_ROOT="$DIST_DIR/$MESHLINK_WINDOWS_TARGET"
STAGE_DIR="$STAGE_ROOT/$MESHLINK_WINDOWS_PACKAGE_NAME-$MESHLINK_WINDOWS_PACKAGE_VERSION"
ZIP_NAME="${MESHLINK_WINDOWS_PACKAGE_NAME}_${MESHLINK_WINDOWS_PACKAGE_VERSION}_${MESHLINK_WINDOWS_TARGET}.zip"
ZIP_PATH="$STAGE_ROOT/$ZIP_NAME"

mkdir -p "$STAGE_ROOT"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

install -m 0755 "$MESHLINK_WINDOWS_BINARY" "$STAGE_DIR/meshlinkd.exe"
install -m 0644 "$ASSET_DIR/client.toml" "$STAGE_DIR/client.toml"
install -m 0644 "$ASSET_DIR/README.txt" "$STAGE_DIR/README.txt"
install -m 0644 "$ASSET_DIR/run-meshlinkd.ps1" "$STAGE_DIR/run-meshlinkd.ps1"
install -m 0644 "$MESHLINK_WINDOWS_TUNNEL_DLL" "$STAGE_DIR/tunnel.dll"
install -m 0644 "$MESHLINK_WINDOWS_WIREGUARD_DLL" "$STAGE_DIR/wireguard.dll"

rm -f "$ZIP_PATH"
(
  cd "$STAGE_ROOT"
  zip -qr "$ZIP_NAME" "$(basename "$STAGE_DIR")"
)

echo "Windows package written to $ZIP_PATH"
