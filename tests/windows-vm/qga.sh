#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WINDOWS_ENV_FILE="${MESHLINK_WINDOWS_ENV:-$ROOT_DIR/tests/windows-vm/windows.env}"
ACTION="${1:-ping}"
shift || true

load_windows_env() {
  if [[ -f "$WINDOWS_ENV_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$WINDOWS_ENV_FILE"
  fi
}

require_tools() {
  command -v virsh >/dev/null 2>&1 || {
    echo "virsh is required" >&2
    exit 1
  }
  command -v jq >/dev/null 2>&1 || {
    echo "jq is required" >&2
    exit 1
  }
}

qga_raw() {
  local payload="${1:-}"
  if [[ -z "$payload" ]]; then
    payload="$(cat)"
  fi
  virsh qemu-agent-command "$MESHLINK_WINDOWS_VM_NAME" "$payload"
}

qga_wait() {
  local timeout="${1:-120}"
  local start
  start="$(date +%s)"

  while true; do
    if qga_raw '{"execute":"guest-ping"}' >/dev/null 2>&1; then
      echo "qemu guest agent is reachable on $MESHLINK_WINDOWS_VM_NAME"
      return 0
    fi

    if (( "$(date +%s)" - start >= timeout )); then
      echo "timed out waiting for qemu guest agent on $MESHLINK_WINDOWS_VM_NAME" >&2
      return 1
    fi
    sleep 2
  done
}

json_array_from_args() {
  if (($# == 0)); then
    printf '%s\n' '[]'
    return 0
  fi

  printf '%s\n' "$@" | jq -R . | jq -s .
}

guest_exec() {
  local program="$1"
  shift
  local args_json pid status stdout_b64 stderr_b64

  args_json="$(json_array_from_args "$@")"
  pid="$(
    jq -cn \
      --arg path "$program" \
      --argjson arg "$args_json" \
      '{execute:"guest-exec",arguments:{path:$path,arg:$arg,"capture-output":true}}' \
      | qga_raw \
      | jq -r '.return.pid'
  )"

  while true; do
    status="$(
      jq -cn --argjson pid "$pid" '{execute:"guest-exec-status",arguments:{pid:$pid}}' \
      | qga_raw
    )"
    if [[ "$(jq -r '.return.exited' <<<"$status")" == "true" ]]; then
      stdout_b64="$(jq -r '.return["out-data"] // empty' <<<"$status")"
      stderr_b64="$(jq -r '.return["err-data"] // empty' <<<"$status")"
      if [[ -n "$stdout_b64" ]]; then
        printf '%s' "$stdout_b64" | base64 -d
      fi
      if [[ -n "$stderr_b64" ]]; then
        printf '%s' "$stderr_b64" | base64 -d >&2
      fi
      return "$(jq -r '.return["exitcode"] // 1' <<<"$status")"
    fi
    sleep 1
  done
}

guest_file_read() {
  local guest_path="$1"
  local handle chunk eof result

  handle="$(
    jq -cn --arg path "$guest_path" '{execute:"guest-file-open",arguments:{path:$path,mode:"r"}}' \
      | qga_raw \
      | jq -r '.return'
  )"

  trap 'jq -cn --argjson handle "$handle" '"'"'{execute:"guest-file-close",arguments:{handle:$handle}}'"'"' | qga_raw >/dev/null 2>&1 || true' RETURN

  while true; do
    result="$(
      jq -cn --argjson handle "$handle" '{execute:"guest-file-read",arguments:{handle:$handle,"count":4096}}' \
        | qga_raw
    )"
    chunk="$(jq -r '.return["buf-b64"] // empty' <<<"$result")"
    eof="$(jq -r '.return.eof // false' <<<"$result")"
    if [[ -n "$chunk" ]]; then
      printf '%s' "$chunk" | base64 -d
    fi
    if [[ "$eof" == "true" ]]; then
      break
    fi
  done
}

guest_file_write() {
  local host_path="$1"
  local guest_path="$2"
  local handle index chunk_b64

  if [[ ! -f "$host_path" ]]; then
    echo "host file does not exist: $host_path" >&2
    return 1
  fi

  handle="$(
    jq -cn --arg path "$guest_path" '{execute:"guest-file-open",arguments:{path:$path,mode:"w"}}' \
      | qga_raw \
      | jq -r '.return'
  )"

  trap 'jq -cn --argjson handle "$handle" '"'"'{execute:"guest-file-close",arguments:{handle:$handle}}'"'"' | qga_raw >/dev/null 2>&1 || true' RETURN

  index=0
  while true; do
    chunk_b64="$(dd if="$host_path" bs=49152 skip="$index" count=1 iflag=fullblock status=none | base64 -w0)"
    if [[ -z "$chunk_b64" ]]; then
      break
    fi
    jq -cn --argjson handle "$handle" --arg data "$chunk_b64" \
      '{execute:"guest-file-write",arguments:{handle:$handle,"buf-b64":$data}}' \
      | qga_raw >/dev/null
    index=$((index + 1))
  done
}

load_windows_env
require_tools

MESHLINK_WINDOWS_VM_NAME="${MESHLINK_WINDOWS_VM_NAME:-meshlink-win-a}"

case "$ACTION" in
  ping)
    qga_raw '{"execute":"guest-ping"}'
    ;;
  wait)
    qga_wait "${1:-120}"
    ;;
  info)
    qga_raw '{"execute":"guest-info"}'
    ;;
  net)
    qga_raw '{"execute":"guest-network-get-interfaces"}'
    ;;
  exec)
    if (($# < 1)); then
      echo "usage: $0 exec <program> [args...]" >&2
      exit 1
    fi
    guest_exec "$@"
    ;;
  powershell)
    if (($# != 1)); then
      echo "usage: $0 powershell <command>" >&2
      exit 1
    fi
    guest_exec "powershell.exe" "-NoProfile" "-NonInteractive" "-ExecutionPolicy" "Bypass" "-Command" "$1"
    ;;
  read)
    if (($# != 1)); then
      echo "usage: $0 read <guest-path>" >&2
      exit 1
    fi
    guest_file_read "$1"
    ;;
  write)
    if (($# != 2)); then
      echo "usage: $0 write <host-path> <guest-path>" >&2
      exit 1
    fi
    guest_file_write "$1" "$2"
    ;;
  *)
    cat >&2 <<EOF
usage: $0 [ping|wait|info|net|exec|powershell|read|write] ...

examples:
  $0 wait 180
  $0 info
  $0 powershell "Get-Service qemu-ga"
  $0 read 'C:\\ProgramData\\MeshLink\\MeshLink.conf'
  $0 write /tmp/MESHLINK.ZIP 'C:\\MESHLINK.ZIP'
EOF
    exit 1
    ;;
esac
