# dsolver-pool-result-monitoring

A Rust service that continuously polls a DEX simulation API to find the best-paying liquidity pool for a set of swap amounts. It runs an embedded HTTP API to expose results, and persists every cycle's output to disk.

---

## What it does

Every `poll_interval_secs` seconds the service:

1. Reads `request-model.json` and POSTs it to `simulation_api_url`.
2. Receives a list of pool simulation results, each containing an `amounts_out` and `slippage` array that correspond 1-to-1 with the `amounts` array in the request.
3. For each index position, finds:
   - The pool that returns the highest `amount_out` — the **winner** for that input amount.
   - The pool that returns the lowest `slippage` — the **low-slippage pool** for that input amount.
4. Marks each winner with `has_lowest_slippage: true` when it is also the lowest-slippage pool for that index.
5. Writes winners, low-slippage pools, and the raw server response to `result-data/sim-result-{blocknumber}-{hhmmssyyyyoodd}.json`.
6. Keeps the latest results in memory for instant retrieval via the API.

Failed requests are retried with exponential backoff up to `max_retries` times before the cycle is logged as an error and skipped (the service keeps running).

---

## Winner object

Each winner entry represents the best pool (highest `amount_out`) for a specific input amount:

```json
{
  "pool_name": "pancakeswap_v3::WETH/USDC",
  "pool_address": "0x72ab388e2e2f6facef59e3c3fa2c4e29011c2d38",
  "amount_in": "1000000000000000000",
  "amount_out": "2057033206",
  "slippage": 2,
  "block_number": 44179002,
  "has_lowest_slippage": false
}
```

`has_lowest_slippage` is `true` when the winning pool for that index is also the pool with the lowest slippage for the same input amount.

## LowSlippagePool object

Each low-slippage entry represents the pool with the lowest `slippage` for a specific input amount:

```json
{
  "pool_name": "uniswap_v3::WETH/USDC",
  "pool_address": "0xabcdef1234567890abcdef1234567890abcdef12",
  "amount_in": "1000000000000000000",
  "amount_out": "2050000000",
  "slippage": 0,
  "block_number": 44179002
}
```

---

## Configuration

A template is provided at `config.json.example`. Copy it and fill in your values:

```bash
cp config.json.example config.json
```

Then edit `config.json`:

```json
{
  "database_url": "postgres://<user>:<password>@<host>:<port>/<database>",
  "simulation_api_url": "http://<host>:<port>/simulate",
  "poll_interval_secs": 2,
  "api_port": 3500,
  "retry": {
    "max_retries": 3,
    "initial_backoff_ms": 500
  }
}
```

| Field | Description |
|---|---|
| `database_url` | PostgreSQL connection string (`postgres://user:pass@host:port/db`) |
| `simulation_api_url` | Full URL of the simulation endpoint that receives the POST request |
| `poll_interval_secs` | Seconds to wait between polling cycles |
| `api_port` | Port the embedded HTTP server listens on |
| `retry.max_retries` | Maximum retry attempts per failed request before skipping the cycle |
| `retry.initial_backoff_ms` | Base backoff in ms; doubles on each retry attempt (capped at 64×) |

> `config.json` is git-ignored. Never commit credentials to the repository.

### Swap request

Edit `request-model.json` to change the swap parameters sent on every cycle:

```json
{
  "request_id": "req-1234",
  "token_in": "0x4200000000000000000000000000000000000006",
  "token_out": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
  "amounts": ["1000000000000000000", "2000000000000000000", "2500000000000000000", "5000000000000000000"],
  "pool_type": "blue_chip"
}
```

The `amounts` array drives how many winners are produced per cycle (one winner per entry).

---

## Build

Requires Rust toolchain (edition 2021).

```bash
RUSTFLAGS="-C target-cpu=native -C link-arg=-s" cargo build --release
```

---

## Start / Stop

### Start

```bash
./start.sh
```

Starts the service in the background and prints the PID and log file path. The `result-data/` directory is created automatically if it does not exist.

To follow the log output, copy and run the hint printed by the script:

```bash
tail -f /path/to/system-monitoring.log
```

### Stop

```bash
./stop.sh
```

Sends `SIGTERM` to the running process and exits. If no instance is found it reports so and exits cleanly.

---

## HTTP API

Base URL: `http://localhost:<api_port>`

### `GET /result`

Returns the aggregated winner entries from **all** persisted result files in `result-data/`.

```bash
curl http://localhost:3500/result
```

Response:

```json
{
  "pool-winners": [
    {
      "pool_name": "pancakeswap_v3::WETH/USDC",
      "pool_address": "0x72ab388e...",
      "amount_in": "1000000000000000000",
      "amount_out": "2057033206",
      "slippage": 2,
      "block_number": 44179002
    },
    ...
  ]
}
```

### `GET /result/latest`

Returns the results from the **most recent completed cycle**, served directly from memory (no disk I/O). Both arrays are index-aligned with the `amounts` array in `request-model.json`.

```bash
curl http://localhost:3500/result/latest
```

Response:

```json
{
  "winners": [
    {
      "pool_name": "pancakeswap_v3::WETH/USDC",
      "pool_address": "0x72ab388e...",
      "amount_in": "1000000000000000000",
      "amount_out": "2057033206",
      "slippage": 2,
      "block_number": 44179002,
      "has_lowest_slippage": false
    },
    ...
  ],
  "low_slippage": [
    {
      "pool_name": "uniswap_v3::WETH/USDC",
      "pool_address": "0xabcdef12...",
      "amount_in": "1000000000000000000",
      "amount_out": "2050000000",
      "slippage": 0,
      "block_number": 44179002
    },
    ...
  ]
}
```

---

## Result files

Each cycle writes a file to `result-data/` following this naming pattern:

```
sim-result-{blocknumber}-{hhmmssyyyyoodd}.json
```

Example: `sim-result-44179002-18283220260402.json`

File structure:

```json
{
  "winners": [ ... ],
  "low_slippage": [ ... ],
  "original_response": { ... }
}
```

`winners` and `low_slippage` are both arrays indexed by input amount position. `original_response` contains the full unmodified payload returned by the simulation API.

---

## Update

```bash
./update.sh
```

Pulls the latest changes from the repository, rebuilds the release binary, and clears all JSON files inside `result-data/`. If `result-data/` does not exist it is created automatically.

---

## Logs

All structured logs are written to `system-monitoring.log` when started via `start.sh`.  
Log verbosity is controlled by the `RUST_LOG` environment variable (default: `info`).

```bash
RUST_LOG=debug ./start.sh
```
