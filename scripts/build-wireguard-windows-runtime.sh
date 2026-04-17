#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

MESHLINK_WINDOWS_RUNTIME_VERSION="${MESHLINK_WINDOWS_RUNTIME_VERSION:-v0.3.17}"
MESHLINK_WINDOWS_WIREGUARD_WINDOWS_REF="${MESHLINK_WINDOWS_WIREGUARD_WINDOWS_REF:-v0.3.17}"
MESHLINK_WINDOWS_WIREGUARD_NT_VERSION="${MESHLINK_WINDOWS_WIREGUARD_NT_VERSION:-0.11}"
MESHLINK_WINDOWS_WINTUN_VERSION="${MESHLINK_WINDOWS_WINTUN_VERSION:-0.13}"
MESHLINK_WINDOWS_WINTUN_SHA256="${MESHLINK_WINDOWS_WINTUN_SHA256:-34afe7d0de1fdb781af3defc0a75fd8c97daa756279b42dd6be6a1bd8ccdc7f0}"
MESHLINK_WINDOWS_RUNTIME_ARCH="${MESHLINK_WINDOWS_RUNTIME_ARCH:-amd64}"
MESHLINK_WINDOWS_RUNTIME_WORK_ROOT="${MESHLINK_WINDOWS_RUNTIME_WORK_ROOT:-$ROOT_DIR/dist/windows-runtime-src}"
MESHLINK_WINDOWS_RUNTIME_STAGE_ROOT="${MESHLINK_WINDOWS_RUNTIME_STAGE_ROOT:-$ROOT_DIR/deploy/packages/windows/runtime}"

require_commands() {
  local missing=0
  for cmd in "$@"; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      echo "missing required command: $cmd" >&2
      missing=1
    fi
  done
  if (( missing )); then
    exit 1
  fi
}

wireguard_windows_repo_dir() {
  printf '%s\n' "$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wireguard-windows-$MESHLINK_WINDOWS_WIREGUARD_WINDOWS_REF"
}

runtime_stage_dir() {
  printf '%s\n' "$MESHLINK_WINDOWS_RUNTIME_STAGE_ROOT/$MESHLINK_WINDOWS_RUNTIME_VERSION/$MESHLINK_WINDOWS_RUNTIME_ARCH"
}

download_wireguard_windows() {
  local repo_dir
  repo_dir="$(wireguard_windows_repo_dir)"
  if [[ -d "$repo_dir/.git" ]]; then
    return 0
  fi

  rm -rf "$repo_dir"
  git clone --branch "$MESHLINK_WINDOWS_WIREGUARD_WINDOWS_REF" --depth 1 \
    https://git.zx2c4.com/wireguard-windows "$repo_dir"
}

build_tunnel_dll() {
  local repo_dir build_dir compiler goarch dll_path
  repo_dir="$(wireguard_windows_repo_dir)"
  build_dir="$repo_dir/embeddable-dll-service"

  case "$MESHLINK_WINDOWS_RUNTIME_ARCH" in
    amd64)
      compiler="x86_64-w64-mingw32-gcc"
      goarch="amd64"
      ;;
    x86)
      compiler="i686-w64-mingw32-gcc"
      goarch="386"
      ;;
    arm64)
      compiler="aarch64-w64-mingw32-gcc"
      goarch="arm64"
      ;;
    *)
      echo "unsupported runtime arch: $MESHLINK_WINDOWS_RUNTIME_ARCH" >&2
      exit 1
      ;;
  esac

  require_commands "$compiler"
  mkdir -p "$build_dir/$MESHLINK_WINDOWS_RUNTIME_ARCH"

  (
    cd "$build_dir"
    GOOS=windows \
    GOARCH="$goarch" \
    CC="$compiler" \
    CGO_ENABLED=1 \
    go build -buildmode c-shared -ldflags="-w -s" -trimpath -v \
      -o "$MESHLINK_WINDOWS_RUNTIME_ARCH/tunnel.dll"
  )

  dll_path="$build_dir/$MESHLINK_WINDOWS_RUNTIME_ARCH/tunnel.dll"
  if [[ ! -f "$dll_path" ]]; then
    echo "tunnel.dll was not produced: $dll_path" >&2
    exit 1
  fi
  printf '%s\n' "$dll_path"
}

download_wireguard_nt_sdk() {
  local zip_path
  zip_path="$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wireguard-nt-$MESHLINK_WINDOWS_WIREGUARD_NT_VERSION.zip"
  if [[ ! -f "$zip_path" ]]; then
    curl -L --fail \
      "https://download.wireguard.com/wireguard-nt/wireguard-nt-$MESHLINK_WINDOWS_WIREGUARD_NT_VERSION.zip" \
      -o "$zip_path"
  fi
  printf '%s\n' "$zip_path"
}

extract_wireguard_dll() {
  local zip_path dll_in_zip out_dir out_path
  zip_path="$(download_wireguard_nt_sdk)"
  out_dir="$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wireguard-nt-$MESHLINK_WINDOWS_WIREGUARD_NT_VERSION"
  out_path="$out_dir/bin/$MESHLINK_WINDOWS_RUNTIME_ARCH/wireguard.dll"

  if [[ -f "$out_path" ]]; then
    printf '%s\n' "$out_path"
    return 0
  fi

  mkdir -p "$out_dir"
  dll_in_zip="wireguard-nt/bin/$MESHLINK_WINDOWS_RUNTIME_ARCH/wireguard.dll"
  unzip -o "$zip_path" "$dll_in_zip" "wireguard-nt/LICENSE.txt" "wireguard-nt/README.md" -d "$out_dir.tmp" >/dev/null
  rm -rf "$out_dir"
  mv "$out_dir.tmp/wireguard-nt" "$out_dir"
  rmdir "$out_dir.tmp"
  printf '%s\n' "$out_path"
}

download_wintun_zip() {
  local zip_path actual_sha
  zip_path="$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wintun-$MESHLINK_WINDOWS_WINTUN_VERSION.zip"
  if [[ ! -f "$zip_path" ]]; then
    curl -L --fail \
      "https://www.wintun.net/builds/wintun-$MESHLINK_WINDOWS_WINTUN_VERSION.zip" \
      -o "$zip_path"
  fi

  actual_sha="$(sha256sum "$zip_path" | awk '{print $1}')"
  if [[ "$actual_sha" != "$MESHLINK_WINDOWS_WINTUN_SHA256" ]]; then
    echo "unexpected SHA256 for $(basename "$zip_path"): $actual_sha" >&2
    echo "expected: $MESHLINK_WINDOWS_WINTUN_SHA256" >&2
    exit 1
  fi

  printf '%s\n' "$zip_path"
}

extract_wintun_dll() {
  local zip_path dll_in_zip out_dir out_path
  zip_path="$(download_wintun_zip)"
  out_dir="$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wintun-$MESHLINK_WINDOWS_WINTUN_VERSION"
  out_path="$out_dir/bin/$MESHLINK_WINDOWS_RUNTIME_ARCH/wintun.dll"

  if [[ -f "$out_path" ]]; then
    printf '%s\n' "$out_path"
    return 0
  fi

  mkdir -p "$out_dir"
  dll_in_zip="wintun/bin/$MESHLINK_WINDOWS_RUNTIME_ARCH/wintun.dll"
  unzip -o "$zip_path" "$dll_in_zip" "wintun/LICENSE.txt" "wintun/README.md" -d "$out_dir.tmp" >/dev/null
  rm -rf "$out_dir"
  mv "$out_dir.tmp/wintun" "$out_dir"
  rmdir "$out_dir.tmp"
  printf '%s\n' "$out_path"
}

write_manifest() {
  local stage_dir tunnel_path wireguard_path wintun_path
  stage_dir="$1"
  tunnel_path="$2"
  wireguard_path="$3"
  wintun_path="$4"

  cat >"$stage_dir/MESHLINK_RUNTIME_MANIFEST.txt" <<EOF
runtime_version=$MESHLINK_WINDOWS_RUNTIME_VERSION
arch=$MESHLINK_WINDOWS_RUNTIME_ARCH
wireguard_windows_ref=$MESHLINK_WINDOWS_WIREGUARD_WINDOWS_REF
wireguard_nt_version=$MESHLINK_WINDOWS_WIREGUARD_NT_VERSION
wintun_version=$MESHLINK_WINDOWS_WINTUN_VERSION
tunnel_dll_sha256=$(sha256sum "$stage_dir/tunnel.dll" | awk '{print $1}')
wireguard_dll_sha256=$(sha256sum "$stage_dir/wireguard.dll" | awk '{print $1}')
wintun_dll_sha256=$(sha256sum "$stage_dir/wintun.dll" | awk '{print $1}')
tunnel_dll_source=$tunnel_path
wireguard_dll_source=$wireguard_path
wintun_dll_source=$wintun_path
generated_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF
}

main() {
  local stage_dir tunnel_path wireguard_path wintun_path

  require_commands git curl unzip go sha256sum

  mkdir -p "$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT"
  stage_dir="$(runtime_stage_dir)"
  mkdir -p "$stage_dir"

  download_wireguard_windows
  tunnel_path="$(build_tunnel_dll)"
  wireguard_path="$(extract_wireguard_dll)"
  wintun_path="$(extract_wintun_dll)"

  install -m 0644 "$tunnel_path" "$stage_dir/tunnel.dll"
  install -m 0644 "$wireguard_path" "$stage_dir/wireguard.dll"
  install -m 0644 "$wintun_path" "$stage_dir/wintun.dll"
  install -m 0644 "$(wireguard_windows_repo_dir)/embeddable-dll-service/README.md" "$stage_dir/tunnel.README.md"
  install -m 0644 "$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wireguard-nt-$MESHLINK_WINDOWS_WIREGUARD_NT_VERSION/LICENSE.txt" "$stage_dir/wireguard-nt.LICENSE.txt"
  install -m 0644 "$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wireguard-nt-$MESHLINK_WINDOWS_WIREGUARD_NT_VERSION/README.md" "$stage_dir/wireguard-nt.README.md"
  install -m 0644 "$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wintun-$MESHLINK_WINDOWS_WINTUN_VERSION/LICENSE.txt" "$stage_dir/wintun.LICENSE.txt"
  install -m 0644 "$MESHLINK_WINDOWS_RUNTIME_WORK_ROOT/wintun-$MESHLINK_WINDOWS_WINTUN_VERSION/README.md" "$stage_dir/wintun.README.md"
  write_manifest "$stage_dir" "$tunnel_path" "$wireguard_path" "$wintun_path"

  echo "Windows runtime assets staged into $stage_dir"
}

main "$@"
