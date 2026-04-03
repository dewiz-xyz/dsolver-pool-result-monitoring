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

echo "Stopping Prometheus and Grafana..."
sudo systemctl stop prometheus || true
sudo systemctl stop grafana-server || true

echo "Copying Prometheus configuration..."
sudo cp "$SCRIPT_DIR/prometheus.yml" /etc/prometheus/prometheus.yml

echo "Configuring Grafana HTTP port to 3001..."
sudo sed -i 's/^;*\s*http_port\s*=.*/http_port = 3001/' /etc/grafana/grafana.ini
# If the key doesn't exist yet, add it under the [server] section
if ! sudo grep -q '^http_port = 3001' /etc/grafana/grafana.ini; then
    sudo sed -i '/^\[server\]/a http_port = 3001' /etc/grafana/grafana.ini
fi

echo "Copying Grafana datasource configuration..."
sudo mkdir -p /etc/grafana/provisioning/datasources
sudo cp "$SCRIPT_DIR/grafana-datasource.yml" /etc/grafana/provisioning/datasources/dsolver.yml

echo "Copying Grafana dashboard provisioning configuration..."
sudo mkdir -p /etc/grafana/provisioning/dashboards
sudo cp "$SCRIPT_DIR/grafana-dashboard-provisioning.yml" /etc/grafana/provisioning/dashboards/dsolver.yml

echo "Copying Grafana dashboard JSON..."
sudo cp "$SCRIPT_DIR/grafana-dashboard.json" /etc/grafana/provisioning/dashboards/dsolver-dashboard.json

echo "Starting Prometheus and Grafana..."
sudo systemctl start prometheus
sudo systemctl start grafana-server

echo "Prometheus status:"
sudo systemctl is-active prometheus

echo "Grafana status:"
sudo systemctl is-active grafana-server

echo "Update complete."