#!/usr/bin/env bash
set -euo pipefail

MESHLINK_PHASE01_ADDR=127.0.0.1:33173 ./tests/e2e/phase01-smoke.sh
sleep 1
MESHLINK_PHASE02_ADDR=127.0.0.1:33174 ./tests/e2e/phase02-smoke.sh
