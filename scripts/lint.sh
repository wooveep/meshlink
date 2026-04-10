#!/usr/bin/env bash
set -euo pipefail

echo "Lint scaffold"
if command -v go >/dev/null 2>&1; then
  (cd server && go test ./...)
fi
if command -v cargo >/dev/null 2>&1; then
  cargo test --manifest-path client/Cargo.toml --workspace
fi
