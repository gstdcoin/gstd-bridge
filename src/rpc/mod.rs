use std::sync::Arc;
use tokio::sync::RwLock;
use warp::Filter;
use serde::{Deserialize, Serialize};
use tracing;

use crate::bridge::vault::VaultManager;
use crate::consensus::{ConsensusEngine, ConsensusStats, TransferStatus};
use crate::config::RpcConfig;

/// Shared state accessible by RPC handlers
pub struct RpcState {
    pub consensus: Arc<RwLock<ConsensusEngine>>,
    pub vault: Arc<RwLock<VaultManager>>,
    pub peer_count: Arc<RwLock<usize>>,
    pub start_time: std::time::Instant,
}

/// Bridge status response for frontend
#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeStatus {
    pub node_id: String,
    pub version: String,
    pub uptime_secs: u64,
    pub peer_count: usize,
    pub consensus: ConsensusStats,
    pub vaults: std::collections::HashMap<String, u64>,
}

/// Transfer info response
#[derive(Debug, Serialize, Deserialize)]
pub struct TransferInfo {
    pub id: String,
    pub source_chain: String,
    pub target_chain: String,
    pub amount: u64,
    pub sender: String,
    pub recipient: String,
    pub status: String,
    pub votes: usize,
    pub executed_tx: Option<String>,
    pub created_at: u64,
}

/// Start the local RPC server for frontend integration
pub async fn start_rpc_server(
    config: RpcConfig,
    state: Arc<RpcState>,
) -> anyhow::Result<()> {
    let state_filter = warp::any().map(move || state.clone());

    // GET /status — overall bridge status
    let status = warp::path("status")
        .and(warp::get())
        .and(state_filter.clone())
        .and_then(handle_status);

    // GET /transfers — list recent transfers
    let transfers = warp::path("transfers")
        .and(warp::get())
        .and(state_filter.clone())
        .and_then(handle_transfers);

    // GET /transfer/:id — single transfer details
    let transfer = warp::path!("transfer" / String)
        .and(warp::get())
        .and(state_filter.clone())
        .and_then(handle_transfer);

    // GET /vaults — vault balances
    let vaults = warp::path("vaults")
        .and(warp::get())
        .and(state_filter.clone())
        .and_then(handle_vaults);

    // CORS
    let cors = if config.cors_enabled {
        warp::cors()
            .allow_any_origin()
            .allow_methods(vec!["GET", "POST", "OPTIONS"])
            .allow_headers(vec!["Content-Type"])
    } else {
        warp::cors()
    };

    let routes = warp::path("api")
        .and(warp::path("bridge"))
        .and(status.or(transfers).or(transfer).or(vaults))
        .with(cors);

    let addr: std::net::SocketAddr = config.listen_addr.parse()?;
    tracing::info!("🌐 Bridge RPC server: http://{addr}/api/bridge/status");

    warp::serve(routes).run(addr).await;
    Ok(())
}

async fn handle_status(
    state: Arc<RpcState>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let consensus = state.consensus.read().await;
    let vault = state.vault.read().await;
    let peers = *state.peer_count.read().await;

    let status = BridgeStatus {
        node_id: String::new(), // filled by caller
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        peer_count: peers,
        consensus: consensus.stats(),
        vaults: vault.all_balances().clone(),
    };

    Ok(warp::reply::json(&status))
}

async fn handle_transfers(
    state: Arc<RpcState>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let consensus = state.consensus.read().await;

    let mut transfers: Vec<TransferInfo> = consensus
        .transfers
        .values()
        .map(|t| TransferInfo {
            id: t.id.clone(),
            source_chain: t.deposit.source_chain.to_string(),
            target_chain: t.deposit.target_chain.to_string(),
            amount: t.deposit.amount,
            sender: t.deposit.sender.clone(),
            recipient: t.deposit.recipient.clone(),
            status: format!("{:?}", t.status),
            votes: t.votes.len(),
            executed_tx: t.executed_tx.clone(),
            created_at: t.created_at,
        })
        .collect();

    // Sort by created_at descending
    transfers.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(warp::reply::json(&transfers))
}

async fn handle_transfer(
    id: String,
    state: Arc<RpcState>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let consensus = state.consensus.read().await;

    match consensus.transfers.get(&id) {
        Some(t) => {
            let info = TransferInfo {
                id: t.id.clone(),
                source_chain: t.deposit.source_chain.to_string(),
                target_chain: t.deposit.target_chain.to_string(),
                amount: t.deposit.amount,
                sender: t.deposit.sender.clone(),
                recipient: t.deposit.recipient.clone(),
                status: format!("{:?}", t.status),
                votes: t.votes.len(),
                executed_tx: t.executed_tx.clone(),
                created_at: t.created_at,
            };
            Ok(warp::reply::json(&info))
        }
        None => Err(warp::reject::not_found()),
    }
}

async fn handle_vaults(
    state: Arc<RpcState>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let vault = state.vault.read().await;
    Ok(warp::reply::json(&vault.all_balances()))
}
