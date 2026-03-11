//! GSTD Cross-Chain Bridge Validator Node
//!
//! Architecture:
//!   ┌─────────────────────────────────────────────────────┐
//!   │                   GSTD Bridge Node                  │
//!   │                                                     │
//!   │  ┌──────────┐  ┌──────────┐  ┌──────────┐          │
//!   │  │TON Watch │  │SOL Watch │  │XRP Watch │ Monitors  │
//!   │  └────┬─────┘  └────┬─────┘  └────┬─────┘          │
//!   │       │              │              │               │
//!   │       └──────────────┼──────────────┘               │
//!   │                      ▼                              │
//!   │            ┌─────────────────┐                      │
//!   │            │ Consensus Engine │◄──── P2P Gossipsub  │
//!   │            │   (67% Quorum)  │                      │
//!   │            └────────┬────────┘                      │
//!   │                     │                               │
//!   │            ┌────────▼────────┐                      │
//!   │            │   MPC Signer    │ Threshold Signatures │
//!   │            └────────┬────────┘                      │
//!   │                     │                               │
//!   │            ┌────────▼────────┐                      │
//!   │            │  Vault Manager  │ Lock-and-Unlock      │
//!   │            └────────┬────────┘                      │
//!   │                     │                               │
//!   │            ┌────────▼────────┐                      │
//!   │            │   RPC Server    │ → Frontend           │
//!   │            └─────────────────┘                      │
//!   └─────────────────────────────────────────────────────┘

mod config;
mod p2p;
mod chains;
mod consensus;
mod mpc;
mod bridge;
mod rpc;

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use clap::Parser;
use futures::StreamExt;
use libp2p::swarm::SwarmEvent;

use config::BridgeConfig;
use chains::{Chain, DepositEvent, ChainMonitor};
use chains::ton::TonMonitor;
use chains::solana::SolanaMonitor;
use chains::xrpl::XrplMonitor;
use consensus::ConsensusEngine;
use bridge::BridgeMessage;
use bridge::vault::VaultManager;
use mpc::ThresholdSigner;
use p2p::P2PNode;

/// GSTD Cross-Chain Bridge Validator
#[derive(Parser)]
#[command(name = "gstd-bridge", version, about)]
struct Cli {
    /// Path to config file
    #[arg(short, long, default_value = "bridge.toml")]
    config: String,

    /// Generate default config and exit
    #[arg(long)]
    init: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,libp2p=warn".to_string()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    // Generate default config if requested
    if cli.init {
        let config = BridgeConfig::default();
        config.save(&cli.config)?;
        tracing::info!("📝 Default config written to {}", cli.config);
        return Ok(());
    }

    // Load configuration
    let config = BridgeConfig::load(&cli.config)?;
    tracing::info!("🌉 GSTD Bridge Node v{}", env!("CARGO_PKG_VERSION"));
    tracing::info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    tracing::info!("TON vault:    {}", config.chains.ton.vault_address);
    tracing::info!("Solana vault: {}", config.chains.solana.vault_address);
    tracing::info!("XRPL vault:   {}", config.chains.xrpl.vault_address);
    tracing::info!("Quorum:       {}%", (config.consensus.quorum_threshold * 100.0) as u32);
    tracing::info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Ensure data directory exists
    std::fs::create_dir_all(&config.node.data_dir)?;

    // ═══════════════════════════════════════════════════════════
    // Initialize Core Components
    // ═══════════════════════════════════════════════════════════

    // Consensus engine (shared state)
    let consensus = Arc::new(RwLock::new(ConsensusEngine::new(config.consensus.clone())));

    // Vault manager
    let vault = Arc::new(RwLock::new(VaultManager::new()));

    // MPC threshold signer (threshold = 67% of validators)
    let signer = Arc::new(ThresholdSigner::new(
        2, // threshold (will be dynamic based on validator count)
        3, // total parties (initial)
        0, // share index (assigned during DKG)
    ));

    // ═══════════════════════════════════════════════════════════
    // Channels
    // ═══════════════════════════════════════════════════════════

    // Chain monitors → Main loop: deposit events
    let (deposit_tx, mut deposit_rx) = mpsc::unbounded_channel::<DepositEvent>();

    // P2P → Main loop: incoming bridge messages
    let (p2p_msg_tx, mut p2p_msg_rx) = mpsc::unbounded_channel();

    // ═══════════════════════════════════════════════════════════
    // Start P2P Network
    // ═══════════════════════════════════════════════════════════

    let mut p2p_node = P2PNode::new(&config.p2p, p2p_msg_tx.clone()).await?;
    let local_peer_id = p2p_node.peer_id;

    // ═══════════════════════════════════════════════════════════
    // Start Chain Monitors (async workers)
    // ═══════════════════════════════════════════════════════════

    let ton_monitor = TonMonitor::new(config.chains.ton.clone());
    let sol_monitor = SolanaMonitor::new(config.chains.solana.clone());
    let xrpl_monitor = XrplMonitor::new(config.chains.xrpl.clone());

    // Spawn TON watcher
    let ton_tx = deposit_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = ton_monitor.start_monitoring(ton_tx).await {
            tracing::error!("TON monitor failed: {e}");
        }
    });

    // Spawn Solana watcher
    let sol_tx = deposit_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = sol_monitor.start_monitoring(sol_tx).await {
            tracing::error!("Solana monitor failed: {e}");
        }
    });

    // Spawn XRPL watcher
    let xrpl_tx = deposit_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = xrpl_monitor.start_monitoring(xrpl_tx).await {
            tracing::error!("XRPL monitor failed: {e}");
        }
    });

    // ═══════════════════════════════════════════════════════════
    // Start RPC Server (for frontend integration)
    // ═══════════════════════════════════════════════════════════

    let rpc_state = Arc::new(rpc::RpcState {
        consensus: consensus.clone(),
        vault: vault.clone(),
        peer_count: Arc::new(RwLock::new(0)),
        start_time: std::time::Instant::now(),
    });
    let rpc_config = config.rpc.clone();
    let rpc_peer_count = rpc_state.peer_count.clone();
    tokio::spawn(async move {
        if let Err(e) = rpc::start_rpc_server(rpc_config, rpc_state).await {
            tracing::error!("RPC server failed: {e}");
        }
    });

    // ═══════════════════════════════════════════════════════════
    // Periodic Tasks
    // ═══════════════════════════════════════════════════════════

    // Cleanup expired transfers every 60s
    let consensus_cleanup = consensus.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            consensus_cleanup.write().await.cleanup_expired();
        }
    });

    // Heartbeat: broadcast alive status every 30s
    let heartbeat_peer = local_peer_id.to_string();
    // (heartbeat is handled in main loop via P2P broadcast)

    // ═══════════════════════════════════════════════════════════
    // MAIN EVENT LOOP
    // Ties together: P2P events, Deposit events, Consensus
    // ═══════════════════════════════════════════════════════════

    tracing::info!("🚀 Bridge node main loop started (peer: {local_peer_id})");

    let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            // ─── P2P Swarm Events ────────────────────────────
            event = p2p_node.swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(p2p::BridgeBehaviourEvent::Gossipsub(e)) => {
                        p2p::gossip::handle_gossipsub_event(
                            e,
                            &local_peer_id,
                            &p2p_msg_tx,
                        );
                    }
                    SwarmEvent::Behaviour(p2p::BridgeBehaviourEvent::Kademlia(e)) => {
                        p2p::discovery::handle_kademlia_event(e);
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        tracing::info!("📡 Listening on: {address}");
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        tracing::info!("🤝 Connected: {peer_id}");
                        consensus.write().await.add_validator(&peer_id);
                        *rpc_peer_count.write().await = p2p_node.peer_count();
                    }
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        tracing::info!("👋 Disconnected: {peer_id}");
                        consensus.write().await.remove_validator(&peer_id);
                        *rpc_peer_count.write().await = p2p_node.peer_count();
                    }
                    _ => {}
                }
            }

            // ─── Deposit from chain monitors ─────────────────
            Some(deposit) = deposit_rx.recv() => {
                tracing::info!(
                    "💰 Deposit detected on {}: {} GSTD (tx: {})",
                    deposit.source_chain,
                    deposit.amount,
                    deposit.tx_hash
                );

                // Lock in vault
                vault.write().await.lock(deposit.source_chain, deposit.amount);

                // Propose to consensus
                let transfer_id = {
                    let mut cs = consensus.write().await;
                    cs.propose_transfer(deposit.clone())
                };

                if let Some(id) = transfer_id {
                    // Broadcast proposal to all validators
                    let msg = BridgeMessage::ProposeTransfer {
                        deposit: deposit.clone(),
                        proposer: local_peer_id.to_string(),
                    };
                    if let Err(e) = p2p_node.broadcast(&msg) {
                        tracing::error!("Failed to broadcast proposal: {e}");
                    }

                    // Cast our own vote (approve + sign)
                    let tx_bytes = bincode::serialize(&deposit).unwrap_or_default();
                    let signing_result = signer.sign_share(&tx_bytes);

                    let vote = consensus::Vote {
                        transfer_id: id.clone(),
                        voter: local_peer_id.to_string(),
                        approved: true,
                        signature_share: signing_result.signature_share,
                        timestamp: chrono::Utc::now().timestamp() as u64,
                    };

                    // Record own vote
                    let new_status = consensus.write().await.record_vote(vote.clone());

                    // Broadcast vote
                    let vote_msg = BridgeMessage::CastVote(vote);
                    let _ = p2p_node.broadcast(&vote_msg);

                    // If immediately approved (e.g., single node), execute
                    if new_status == Some(consensus::TransferStatus::Approved) {
                        handle_approved_transfer(
                            &id,
                            &consensus,
                            &vault,
                            &signer,
                            &mut p2p_node,
                            &local_peer_id,
                        ).await;
                    }
                }
            }

            // ─── Incoming P2P messages ───────────────────────
            Some((sender, msg)) = p2p_msg_rx.recv() => {
                match msg {
                    BridgeMessage::ProposeTransfer { deposit, proposer } => {
                        tracing::info!(
                            "📋 Transfer proposal from {proposer}: {} GSTD {} → {}",
                            deposit.amount,
                            deposit.source_chain,
                            deposit.target_chain,
                        );

                        // Verify the deposit on-chain before voting
                        // (simplified: we trust the proposer for now)
                        let transfer_id = {
                            let mut cs = consensus.write().await;
                            cs.propose_transfer(deposit.clone())
                        };

                        if let Some(id) = transfer_id {
                            // Sign and vote
                            let tx_bytes = bincode::serialize(&deposit).unwrap_or_default();
                            let signing_result = signer.sign_share(&tx_bytes);

                            let vote = consensus::Vote {
                                transfer_id: id.clone(),
                                voter: local_peer_id.to_string(),
                                approved: true,
                                signature_share: signing_result.signature_share,
                                timestamp: chrono::Utc::now().timestamp() as u64,
                            };

                            let new_status = consensus.write().await.record_vote(vote.clone());
                            let vote_msg = BridgeMessage::CastVote(vote);
                            let _ = p2p_node.broadcast(&vote_msg);

                            if new_status == Some(consensus::TransferStatus::Approved) {
                                handle_approved_transfer(
                                    &id,
                                    &consensus,
                                    &vault,
                                    &signer,
                                    &mut p2p_node,
                                    &local_peer_id,
                                ).await;
                            }
                        }
                    }

                    BridgeMessage::CastVote(vote) => {
                        tracing::debug!("🗳️ Vote from {} for {}", vote.voter, vote.transfer_id);
                        let new_status = consensus.write().await.record_vote(vote.clone());

                        if new_status == Some(consensus::TransferStatus::Approved) {
                            handle_approved_transfer(
                                &vote.transfer_id,
                                &consensus,
                                &vault,
                                &signer,
                                &mut p2p_node,
                                &local_peer_id,
                            ).await;
                        }
                    }

                    BridgeMessage::TransferExecuted { transfer_id, tx_hash, executor } => {
                        tracing::info!(
                            "✅ Transfer {transfer_id} executed by {executor}: {tx_hash}"
                        );
                        consensus.write().await.mark_executed(&transfer_id, tx_hash);
                    }

                    BridgeMessage::StateSync { epoch, state_hash, vault_balances } => {
                        tracing::debug!(
                            "📊 State sync from {sender}: epoch={epoch}, hash={state_hash}"
                        );
                        // Reconcile state (production: verify hash, merge)
                    }

                    BridgeMessage::ValidatorHeartbeat { peer_id, version, uptime_secs, .. } => {
                        tracing::debug!(
                            "💓 Heartbeat from {peer_id}: v{version}, up {uptime_secs}s"
                        );
                    }
                }
            }

            // ─── Periodic heartbeat ──────────────────────────
            _ = heartbeat_interval.tick() => {
                let uptime = std::time::Instant::now().elapsed().as_secs();
                let hb = BridgeMessage::ValidatorHeartbeat {
                    peer_id: local_peer_id.to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    uptime_secs: uptime,
                    chains_monitoring: vec![
                        "TON".to_string(),
                        "Solana".to_string(),
                        "XRPL".to_string(),
                    ],
                };
                let _ = p2p_node.broadcast(&hb);

                let stats = consensus.read().await.stats();
                tracing::info!(
                    "📊 Bridge: {} peers, {} validators, {} pending, {} executed",
                    p2p_node.peer_count(),
                    stats.validators,
                    stats.pending,
                    stats.executed,
                );
            }
        }
    }
}

/// Handle an approved transfer: aggregate signatures and execute withdrawal
async fn handle_approved_transfer(
    transfer_id: &str,
    consensus: &Arc<RwLock<ConsensusEngine>>,
    vault: &Arc<RwLock<VaultManager>>,
    signer: &Arc<ThresholdSigner>,
    p2p_node: &mut P2PNode,
    local_peer_id: &libp2p::PeerId,
) {
    let cs = consensus.read().await;
    let transfer = match cs.transfers.get(transfer_id) {
        Some(t) => t.clone(),
        None => return,
    };
    drop(cs);

    // Collect signature shares
    let shares = consensus.read().await.collect_signature_shares(transfer_id);

    // Attempt to aggregate
    if let Some(_aggregated_sig) = signer.try_aggregate(&shares) {
        // Check vault liquidity on target chain
        let can_unlock = vault.write().await.unlock(
            transfer.deposit.target_chain,
            transfer.deposit.amount,
        );

        if !can_unlock {
            tracing::error!(
                "❌ Insufficient liquidity on {} for transfer {transfer_id}",
                transfer.deposit.target_chain
            );
            return;
        }

        // Execute withdrawal on target chain
        // In production: use the aggregated MPC signature
        let tx_hash = format!(
            "{}_bridge_{}",
            transfer.deposit.target_chain,
            &transfer_id[..8.min(transfer_id.len())]
        );

        tracing::info!(
            "🎯 Executing withdrawal: {} GSTD on {} → {} (tx: {tx_hash})",
            transfer.deposit.amount,
            transfer.deposit.target_chain,
            transfer.deposit.recipient,
        );

        // Mark as executed
        consensus.write().await.mark_executed(transfer_id, tx_hash.clone());

        // Broadcast execution to network
        let msg = BridgeMessage::TransferExecuted {
            transfer_id: transfer_id.to_string(),
            tx_hash,
            executor: local_peer_id.to_string(),
        };
        let _ = p2p_node.broadcast(&msg);
    }
}
