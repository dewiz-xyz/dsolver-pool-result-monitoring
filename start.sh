#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BINARY="$SCRIPT_DIR/target/release/dsolver-pool-result-monitoring"
LOG_FILE="$SCRIPT_DIR/system-monitoring.log"

if [[ ! -d "$SCRIPT_DIR/result-data" ]]; then
    echo "Creating result-data directory..."
    mkdir -p "$SCRIPT_DIR/result-data"
fi

if [[ ! -x "$BINARY" ]]; then
    echo "Binary not found: $BINARY" >&2
    echo "Run: RUSTFLAGS=\"-C target-cpu=native -C link-arg=-s\" cargo build --release" >&2
    exit 1
fi

nohup "$BINARY" >> "$LOG_FILE" 2>&1 &
PID=$!

echo "Started dsolver-pool-result-monitoring"
echo "PID: $PID"
echo "Log: $LOG_FILE"

tail -f "$LOG_FILE"
