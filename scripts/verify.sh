#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "[1/2] cargo clippy --workspace -- -D warnings..."
cargo clippy --workspace -- -D warnings

echo "[2/2] cargo test --workspace..."
cargo test --workspace

echo "OK: PrinterTUI verificado."
