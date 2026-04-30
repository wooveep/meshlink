#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNS="${MESHLINK_STABILITY_RUNS:-3}"
ARTIFACT_ROOT="${MESHLINK_STABILITY_ARTIFACT_DIR:-${MESHLINK_LAB_STATE_DIR:-/tmp/meshlink-nat-lab}/stability-gate}"

if ! [[ "$RUNS" =~ ^[0-9]+$ ]] || (( RUNS < 1 )); then
  echo "MESHLINK_STABILITY_RUNS must be a positive integer" >&2
  exit 1
fi

mkdir -p "$ARTIFACT_ROOT"
SUMMARY="$ARTIFACT_ROOT/summary.tsv"
printf 'run\tphase\tstatus\tartifact\n' >"$SUMMARY"

stop_windows_meshlinkd_best_effort() {
  "$ROOT_DIR/tests/windows-vm/qga.sh" exec powershell.exe \
    -NoProfile \
    -NonInteractive \
    -ExecutionPolicy Bypass \
    -Command 'Get-Process meshlinkd -ErrorAction SilentlyContinue | Stop-Process -Force; Get-Service "WireGuardTunnel$MeshLink" -ErrorAction SilentlyContinue | Stop-Service -Force' \
    >/dev/null 2>&1 || true
}

run_phase() {
  local run="$1"
  local phase="$2"
  shift 2

  local artifact_dir="$ARTIFACT_ROOT/run-${run}/${phase}"
  mkdir -p "$artifact_dir"

  echo "== stability run ${run}/${RUNS}: ${phase} =="
  if MESHLINK_VERIFY_ENDPOINT_APPLY="${MESHLINK_VERIFY_ENDPOINT_APPLY:-1}" \
    MESHLINK_LINUX_IPERF_ARTIFACT_DIR="$artifact_dir/linux-iperf3" \
    MESHLINK_WINDOWS_VALIDATION_STATE_DIR="$artifact_dir/windows-phase08" \
    "$@" >"$artifact_dir/stdout.log" 2>"$artifact_dir/stderr.log"; then
    printf '%s\t%s\tpass\t%s\n' "$run" "$phase" "$artifact_dir" | tee -a "$SUMMARY"
    return 0
  fi

  printf '%s\t%s\tfail\t%s\n' "$run" "$phase" "$artifact_dir" | tee -a "$SUMMARY"
  echo "phase failed: ${phase}; artifact=${artifact_dir}" >&2
  return 1
}

for run in $(seq 1 "$RUNS"); do
  stop_windows_meshlinkd_best_effort
  run_phase "$run" linux-phase05 "$ROOT_DIR/tests/nat-lab/run-phase05.sh"
  stop_windows_meshlinkd_best_effort
  run_phase "$run" linux-iperf3 env MESHLINK_LINUX_IPERF_SKIP_PHASE05=1 "$ROOT_DIR/tests/nat-lab/run-linux-iperf3.sh"
  stop_windows_meshlinkd_best_effort
  run_phase "$run" linux-phase06 "$ROOT_DIR/tests/nat-lab/run-phase06.sh"
  stop_windows_meshlinkd_best_effort
  run_phase "$run" linux-phase08-routes "$ROOT_DIR/tests/nat-lab/run-phase08-routes.sh"
  run_phase "$run" windows-phase08 "$ROOT_DIR/tests/windows-vm/run-phase08-validation.sh"
done

echo "stability gate passed: runs=${RUNS} summary=${SUMMARY}"
