pub mod state;

use std::collections::{HashMap, HashSet};

use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tracing;

use crate::chains::DepositEvent;
use crate::config::ConsensusConfig;

/// A unique identifier for a bridge transfer request
pub type TransferId = String;

/// Vote from a validator node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub transfer_id: TransferId,
    pub voter: String, // PeerId as string
    pub approved: bool,
    pub signature_share: Vec<u8>, // MPC partial signature
    pub timestamp: u64,
}

/// Current status of a bridge transfer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    /// Deposit detected, waiting for votes
    Pending,
    /// Quorum reached, executing withdrawal
    Approved,
    /// Withdrawal executed on target chain
    Executed,
    /// Voting failed (timeout or rejection)
    Rejected,
    /// Already processed (double-spend protection)
    Duplicate,
}

/// A bridge transfer being tracked by the consensus engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeTransfer {
    pub id: TransferId,
    pub deposit: DepositEvent,
    pub status: TransferStatus,
    pub votes: HashMap<String, Vote>,
    pub created_at: u64,
    pub executed_tx: Option<String>,
}

impl BridgeTransfer {
    pub fn new(deposit: DepositEvent) -> Self {
        let id = format!(
            "{}:{}:{}",
            deposit.source_chain, deposit.tx_hash,
            deposit.timestamp
        );
        Self {
            id,
            deposit,
            status: TransferStatus::Pending,
            votes: HashMap::new(),
            created_at: chrono::Utc::now().timestamp() as u64,
            executed_tx: None,
        }
    }
}

/// Consensus engine — manages voting rounds for bridge transfers
pub struct ConsensusEngine {
    config: ConsensusConfig,
    /// Active transfers being voted on
    pub transfers: HashMap<TransferId, BridgeTransfer>,
    /// Set of known tx_hashes to prevent double-spending
    processed_txs: HashSet<String>,
    /// Known active validators (PeerIds)
    pub validators: HashSet<String>,
}

impl ConsensusEngine {
    pub fn new(config: ConsensusConfig) -> Self {
        Self {
            config,
            transfers: HashMap::new(),
            processed_txs: HashSet::new(),
            validators: HashSet::new(),
        }
    }

    /// Register a new validator (peer connected)
    pub fn add_validator(&mut self, peer_id: &PeerId) {
        self.validators.insert(peer_id.to_string());
        tracing::info!(
            "✅ Validator added: {peer_id} (total: {})",
            self.validators.len()
        );
    }

    /// Remove a validator (peer disconnected)
    pub fn remove_validator(&mut self, peer_id: &PeerId) {
        self.validators.remove(&peer_id.to_string());
        tracing::info!(
            "❌ Validator removed: {peer_id} (total: {})",
            self.validators.len()
        );
    }

    /// Propose a new bridge transfer for voting
    /// Returns None if already processed (double-spend protection)
    pub fn propose_transfer(&mut self, deposit: DepositEvent) -> Option<TransferId> {
        // Double-spend protection
        let tx_key = format!("{}:{}", deposit.source_chain, deposit.tx_hash);
        if self.processed_txs.contains(&tx_key) {
            tracing::warn!("⛔ Duplicate deposit detected: {tx_key}");
            return None;
        }

        let transfer = BridgeTransfer::new(deposit);
        let id = transfer.id.clone();

        tracing::info!(
            "📋 Transfer proposed: {} ({} → {} for {} GSTD)",
            id,
            transfer.deposit.source_chain,
            transfer.deposit.target_chain,
            transfer.deposit.amount,
        );

        self.transfers.insert(id.clone(), transfer);
        Some(id)
    }

    /// Record a vote for a transfer
    /// Returns the new status if quorum is reached
    pub fn record_vote(&mut self, vote: Vote) -> Option<TransferStatus> {
        let transfer = self.transfers.get_mut(&vote.transfer_id)?;

        if transfer.status != TransferStatus::Pending {
            return Some(transfer.status.clone());
        }

        let transfer_id = vote.transfer_id.clone();
        
        // Record the vote
        transfer.votes.insert(vote.voter.clone(), vote);

        // Check quorum
        let total_validators = self.validators.len().max(1);
        let approve_count = transfer.votes.values().filter(|v| v.approved).count();
        let reject_count = transfer.votes.values().filter(|v| !v.approved).count();

        let approve_ratio = approve_count as f64 / total_validators as f64;
        let reject_ratio = reject_count as f64 / total_validators as f64;

        if approve_ratio >= self.config.quorum_threshold
            && total_validators >= self.config.min_validators
        {
            transfer.status = TransferStatus::Approved;
            let tx_key = format!(
                "{}:{}",
                transfer.deposit.source_chain, transfer.deposit.tx_hash
            );
            self.processed_txs.insert(tx_key);

            tracing::info!(
                "🏆 Transfer APPROVED: {} ({approve_count}/{total_validators} votes)",
                transfer_id,
            );
            return Some(TransferStatus::Approved);
        }

        if reject_ratio > (1.0 - self.config.quorum_threshold) {
            transfer.status = TransferStatus::Rejected;
            tracing::info!(
                "🚫 Transfer REJECTED: {} ({reject_count}/{total_validators} rejections)",
                transfer_id,
            );
            return Some(TransferStatus::Rejected);
        }

        None // Voting still in progress
    }

    /// Mark a transfer as executed
    pub fn mark_executed(&mut self, transfer_id: &str, tx_hash: String) {
        if let Some(transfer) = self.transfers.get_mut(transfer_id) {
            transfer.status = TransferStatus::Executed;
            transfer.executed_tx = Some(tx_hash);
        }
    }

    /// Collect MPC signature shares from votes for an approved transfer
    pub fn collect_signature_shares(&self, transfer_id: &str) -> Vec<Vec<u8>> {
        self.transfers
            .get(transfer_id)
            .map(|t| {
                t.votes
                    .values()
                    .filter(|v| v.approved)
                    .map(|v| v.signature_share.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Clean up expired pending transfers
    pub fn cleanup_expired(&mut self) {
        let now = chrono::Utc::now().timestamp() as u64;
        let timeout = self.config.vote_timeout_secs;

        let expired: Vec<String> = self
            .transfers
            .iter()
            .filter(|(_, t)| {
                t.status == TransferStatus::Pending
                    && (now - t.created_at) > timeout
            })
            .map(|(id, _)| id.clone())
            .collect();

        for id in expired {
            if let Some(t) = self.transfers.get_mut(&id) {
                t.status = TransferStatus::Rejected;
                tracing::warn!("⏰ Transfer expired (timeout): {id}");
            }
        }
    }

    /// Get summary stats
    pub fn stats(&self) -> ConsensusStats {
        let pending = self.transfers.values().filter(|t| t.status == TransferStatus::Pending).count();
        let approved = self.transfers.values().filter(|t| t.status == TransferStatus::Approved).count();
        let executed = self.transfers.values().filter(|t| t.status == TransferStatus::Executed).count();
        let rejected = self.transfers.values().filter(|t| t.status == TransferStatus::Rejected).count();

        ConsensusStats {
            validators: self.validators.len(),
            pending,
            approved,
            executed,
            rejected,
            total_processed: self.processed_txs.len(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsensusStats {
    pub validators: usize,
    pub pending: usize,
    pub approved: usize,
    pub executed: usize,
    pub rejected: usize,
    pub total_processed: usize,
}
