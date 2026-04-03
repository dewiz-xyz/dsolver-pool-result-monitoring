use anyhow::{Context, Result};
use axum::{
    Json,
    Router,
    extract::State,
    routing::get,
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::RwLock;

#[derive(Deserialize)]
struct RetryConfig {
    max_retries: u32,
    initial_backoff_ms: u64,
}

#[derive(Deserialize)]
struct Config {
    simulation_api_url: String,
    poll_interval_secs: u64,
    api_port: u16,
    retry: RetryConfig,
}

#[derive(Deserialize, Serialize, Clone)]
struct PoolResult {
    pool: String,
    pool_name: String,
    pool_address: String,
    amounts_out: Vec<String>,
    slippage: Vec<i64>,
    limit_max_in: String,
    gas_used: Vec<u64>,
    block_number: u64,
}

#[derive(Deserialize)]
struct SimulationRequest {
    amounts: Vec<String>,
}

#[derive(Deserialize)]
struct SimulationResponse {
    data: Vec<PoolResult>,
}

#[derive(Serialize, Clone)]
struct Winner {
    pool_name: String,
    pool_address: String,
    amount_in: String,
    amount_out: String,
    slippage: i64,
    final_amount_out: String,
    block_number: u64,
    has_lowest_slippage: bool,
    difference_to_lowest_slippage: String,
}

#[derive(Serialize, Clone)]
struct LowSlippagePool {
    pool_name: String,
    pool_address: String,
    amount_in: String,
    amount_out: String,
    slippage: i64,
    block_number: u64,
}

#[derive(Serialize, Clone)]
struct BestAmountOutPool {
    pool_name: String,
    pool_address: String,
    amount_in: String,
    amount_out: String,
    slippage: i64,
    final_amount_out: String,
    block_number: u64,
}

#[derive(Serialize)]
struct OutputResult {
    winners: Vec<Winner>,
    best_amount_out: Vec<BestAmountOutPool>,
    low_slippage: Vec<LowSlippagePool>,
    pools: Vec<PoolResult>,
    original_response: Value,
}

#[derive(Serialize)]
struct PoolWinnersResponse {
    #[serde(rename = "pool-winners")]
    pool_winners: Vec<Value>,
}

#[derive(Serialize)]
struct LatestResponse {
    winners: Vec<Winner>,
    best_amount_out: Vec<BestAmountOutPool>,
    low_slippage: Vec<LowSlippagePool>,
}

#[derive(Clone, Default)]
struct LatestData {
    winners: Vec<Winner>,
    best_amount_out: Vec<BestAmountOutPool>,
    low_slippage: Vec<LowSlippagePool>,
    pools: Vec<PoolResult>,
}

type LatestState = Arc<RwLock<LatestData>>;

async fn result_handler() -> Json<PoolWinnersResponse> {
    let mut pool_winners: Vec<Value> = Vec::new();

    let mut read_dir = match fs::read_dir("result-data").await {
        Ok(rd) => rd,
        Err(e) => {
            tracing::error!(error = %e, "failed to open result-data directory");
            return Json(PoolWinnersResponse { pool_winners });
        }
    };

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match fs::read_to_string(&path).await {
            Ok(raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(val) => {
                    if let Some(winners) = val.get("winners") {
                        if let Some(arr) = winners.as_array() {
                            pool_winners.extend(arr.iter().cloned());
                        }
                    }
                }
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "skipping malformed JSON"),
            },
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "failed to read file"),
        }
    }

    Json(PoolWinnersResponse { pool_winners })
}

async fn latest_handler(State(state): State<LatestState>) -> Json<LatestResponse> {
    let data = state.read().await.clone();
    Json(LatestResponse {
        winners: data.winners,
        best_amount_out: data.best_amount_out,
        low_slippage: data.low_slippage,
    })
}

async fn start_api_server(port: u16, latest: LatestState) {
    let app = Router::new()
        .route("/result", get(result_handler))
        .route("/result/latest", get(latest_handler))
        .with_state(latest);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr)
        .await
        .expect("failed to bind API listener");
    tracing::info!(%addr, "API server listening");
    axum::serve(listener, app)
        .await
        .expect("API server error");
}

async fn simulate_once(
    client: &reqwest::Client,
    config: &Config,
    request_payload: &Value,
    latest: &LatestState,
) -> Result<()> {
    let max_retries = config.retry.max_retries;
    let initial_backoff_ms = config.retry.initial_backoff_ms;

    let mut attempt = 0u32;
    let response_text = loop {
        attempt += 1;
        tracing::info!(attempt, url = %config.simulation_api_url, "posting simulation request");

        match client
            .post(&config.simulation_api_url)
            .json(request_payload)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                let body = resp
                    .text()
                    .await
                    .context("failed to read response body")?;

                if status.is_success() {
                    tracing::info!(%status, "received simulation response");
                    break body;
                }

                let msg = format!("simulation API returned {}: {}", status, body);
                if attempt > max_retries {
                    anyhow::bail!(msg);
                }
                tracing::warn!("{} — retrying ({}/{})", msg, attempt, max_retries);
            }
            Err(e) => {
                if attempt > max_retries {
                    return Err(e).context("failed to POST to simulation API");
                }
                tracing::warn!(error = %e, "request failed — retrying ({}/{})", attempt, max_retries);
            }
        }

        let backoff = Duration::from_millis(initial_backoff_ms * (1 << (attempt - 1).min(6)));
        tracing::info!(backoff_ms = backoff.as_millis(), "waiting before retry");
        tokio::time::sleep(backoff).await;
    };

    let original_response: Value =
        serde_json::from_str(&response_text).context("failed to parse response as JSON")?;

    let sim_response: SimulationResponse = serde_json::from_str(&response_text)
        .context("failed to deserialize simulation response into expected shape")?;

    anyhow::ensure!(!sim_response.data.is_empty(), "simulation returned no pools");

    let sim_request: SimulationRequest = serde_json::from_str(
        &serde_json::to_string(request_payload).unwrap_or_default(),
    )
    .context("failed to deserialize amounts from request payload")?;

    let num_amounts = sim_response.data[0].amounts_out.len();
    let mut winners: Vec<Winner> = Vec::with_capacity(num_amounts);

    for idx in 0..num_amounts {
        let best = sim_response
            .data
            .iter()
            .max_by_key(|pool| {
                let raw = pool.amounts_out
                    .get(idx)
                    .and_then(|a| a.parse::<i128>().ok())
                    .unwrap_or(0);
                let slip = pool.slippage.get(idx).copied().unwrap_or(0) as i128;
                (raw * (10000 + slip) / 10000).max(0) as u128
            })
            .expect("non-empty pool list");

        let amount_in = sim_request.amounts.get(idx).cloned().unwrap_or_default();
        let amount_out = best.amounts_out.get(idx).cloned().unwrap_or_default();
        let slippage = best.slippage.get(idx).copied().unwrap_or(0);
        let raw_out = amount_out.parse::<i128>().unwrap_or(0);
        let final_amount_out = (raw_out * (10000 + slippage as i128) / 10000).to_string();

        tracing::info!(
            index = idx,
            pool = %best.pool_name,
            amount_in = %amount_in,
            amount_out = %amount_out,
            final_amount_out = %final_amount_out,
            "winner for amounts_out[{}]", idx
        );

        winners.push(Winner {
            pool_name: best.pool_name.clone(),
            pool_address: best.pool_address.clone(),
            amount_in,
            amount_out,
            slippage,
            final_amount_out,
            block_number: best.block_number,
            has_lowest_slippage: false,         // filled in below
            difference_to_lowest_slippage: "0".to_string(), // filled in below
        });
    }

    let mut best_amount_out: Vec<BestAmountOutPool> = Vec::with_capacity(num_amounts);

    for idx in 0..num_amounts {
        let best_raw = sim_response
            .data
            .iter()
            .max_by_key(|pool| {
                pool.amounts_out
                    .get(idx)
                    .and_then(|a| a.parse::<u128>().ok())
                    .unwrap_or(0)
            })
            .expect("non-empty pool list");

        let amount_in = sim_request.amounts.get(idx).cloned().unwrap_or_default();
        let amount_out = best_raw.amounts_out.get(idx).cloned().unwrap_or_default();
        let slippage = best_raw.slippage.get(idx).copied().unwrap_or(0);
        let raw_out = amount_out.parse::<i128>().unwrap_or(0);
        let final_amount_out = (raw_out * (10000 + slippage as i128) / 10000).to_string();

        tracing::info!(
            index = idx,
            pool = %best_raw.pool_name,
            amount_out = %amount_out,
            "best raw amount_out for amounts[{}]", idx
        );

        best_amount_out.push(BestAmountOutPool {
            pool_name: best_raw.pool_name.clone(),
            pool_address: best_raw.pool_address.clone(),
            amount_in,
            amount_out,
            slippage,
            final_amount_out,
            block_number: best_raw.block_number,
        });
    }

    let mut low_slippage: Vec<LowSlippagePool> = Vec::with_capacity(num_amounts);

    for idx in 0..num_amounts {
        let best_low = sim_response
            .data
            .iter()
            .min_by_key(|pool| pool.slippage.get(idx).copied().unwrap_or(i64::MAX))
            .expect("non-empty pool list");

        let amount_in = sim_request.amounts.get(idx).cloned().unwrap_or_default();
        let amount_out = best_low.amounts_out.get(idx).cloned().unwrap_or_default();
        let slippage = best_low.slippage.get(idx).copied().unwrap_or(0);

        tracing::info!(
            index = idx,
            pool = %best_low.pool_name,
            slippage = %slippage,
            "lowest slippage pool for amounts[{}]", idx
        );

        low_slippage.push(LowSlippagePool {
            pool_name: best_low.pool_name.clone(),
            pool_address: best_low.pool_address.clone(),
            amount_in,
            amount_out,
            slippage,
            block_number: best_low.block_number,
        });
    }

    for (idx, w) in winners.iter_mut().enumerate() {
        if let Some(ls) = low_slippage.get(idx) {
            w.has_lowest_slippage = w.pool_address == ls.pool_address || w.slippage == ls.slippage;
            if w.has_lowest_slippage {
                w.difference_to_lowest_slippage = "0".to_string();
            } else {
                let winner_final = w.final_amount_out.parse::<i128>().unwrap_or(0);
                let ls_raw = ls.amount_out.parse::<i128>().unwrap_or(0);
                let ls_final = ls_raw * (10000 + ls.slippage as i128) / 10000;
                w.difference_to_lowest_slippage = (winner_final - ls_final).to_string();
            }
        }
    }

    let block_number = winners.first().map(|w| w.block_number).unwrap_or(0);
    let now = Local::now();
    // hhmmssyyyyoodd  (hh=hour24, mm=min, ss=sec, yyyy=year, oo=month, dd=day)
    let datetime_str = now.format("%H%M%S%Y%m%d").to_string();
    let filename = format!("sim-result-{}-{}.json", block_number, datetime_str);
    let output_path = Path::new("result-data").join(&filename);

    let pools = sim_response.data.clone();

    {
        let mut data = latest.write().await;
        data.winners = winners.clone();
        data.best_amount_out = best_amount_out.clone();
        data.low_slippage = low_slippage.clone();
        data.pools = pools.clone();
    }

    let output = OutputResult {
        winners,
        best_amount_out,
        low_slippage,
        pools,
        original_response,
    };

    let output_json =
        serde_json::to_string_pretty(&output).context("failed to serialize output")?;

    fs::write(&output_path, &output_json)
        .await
        .context("failed to write output file")?;

    tracing::info!(path = %output_path.display(), "result saved");
    println!("result saved → {}", output_path.display());

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_raw = fs::read_to_string("config.json")
        .await
        .context("failed to read config.json")?;

    let config: Arc<Config> = Arc::new(
        serde_json::from_str(&config_raw).context("failed to parse config.json")?,
    );

    let request_raw = fs::read_to_string("request-model.json")
        .await
        .context("failed to read request-model.json")?;

    let request_payload: Arc<Value> = Arc::new(
        serde_json::from_str(&request_raw).context("failed to parse request-model.json")?,
    );

    let client = reqwest::Client::new();
    let poll_interval = Duration::from_secs(config.poll_interval_secs);

    tracing::info!(
        poll_interval_secs = config.poll_interval_secs,
        max_retries = config.retry.max_retries,
        initial_backoff_ms = config.retry.initial_backoff_ms,
        api_port = config.api_port,
        "starting — press Ctrl+C to stop"
    );

    let latest: LatestState = Arc::new(RwLock::new(LatestData::default()));

    let api_port = config.api_port;
    let api_task = tokio::spawn(start_api_server(api_port, Arc::clone(&latest)));

    let poll_task = async {
        loop {
            if let Err(e) = simulate_once(&client, &config, &request_payload, &latest).await {
                tracing::error!(error = %e, "simulation cycle failed");
            }

            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {}
                _ = signal::ctrl_c() => {
                    tracing::info!("received Ctrl+C, shutting down");
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = poll_task => {}
        res = api_task => {
            if let Err(e) = res {
                tracing::error!(error = %e, "API server task panicked");
            }
        }
    }

    Ok(())
}
