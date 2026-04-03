#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "$SCRIPT_DIR"

echo "Pulling latest changes..."
git pull

echo "Stopping running instance (if any)..."
BINARY_NAME="dsolver-pool-result-monitoring"
PIDS=$(pgrep -f "$BINARY_NAME" 2>/dev/null || true)
if [[ -n "$PIDS" ]]; then
    kill $PIDS
    echo "Stopped $BINARY_NAME (PID: $PIDS)."
else
    echo "No running instance found."
fi

echo "Building release binary..."
RUSTFLAGS="-C target-cpu=native -C link-arg=-s" cargo build --release

echo "Build complete: $SCRIPT_DIR/target/release/dsolver-pool-result-monitoring"
if [[ ! -d "$SCRIPT_DIR/result-data" ]]; then
    echo "Creating result-data directory..."
    mkdir -p "$SCRIPT_DIR/result-data"
else
    echo "Clearing result-data JSON files..."
    find "$SCRIPT_DIR/result-data" -maxdepth 1 -name "*.json" -delete
    echo "result-data cleared."
fi