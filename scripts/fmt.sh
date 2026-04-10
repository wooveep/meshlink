#!/usr/bin/env bash
set -euo pipefail

echo "Formatting scaffold"
if command -v gofmt >/dev/null 2>&1; then
  find server -name '*.go' -print0 | xargs -0 -r gofmt -w
fi
if command -v cargo >/dev/null 2>&1; then
  cargo fmt --manifest-path client/Cargo.toml --all || true
fi
