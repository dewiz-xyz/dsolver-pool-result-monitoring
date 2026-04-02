#!/usr/bin/env bash
set -euo pipefail

BINARY_NAME="dsolver-pool-result-monitoring"

PIDS=$(pgrep -f "$BINARY_NAME" 2>/dev/null || true)

if [[ -z "$PIDS" ]]; then
    echo "No running instance of $BINARY_NAME found."
    exit 0
fi

echo "Stopping $BINARY_NAME (PID: $PIDS)..."
kill $PIDS
echo "Done."
