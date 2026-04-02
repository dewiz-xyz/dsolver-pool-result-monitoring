#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "$SCRIPT_DIR"

echo "Pulling latest changes..."
git pull

echo "Building release binary..."
RUSTFLAGS="-C target-cpu=native -C link-arg=-s" cargo build --release

echo "Build complete: $SCRIPT_DIR/target/release/dsolver-pool-result-monitoring"
