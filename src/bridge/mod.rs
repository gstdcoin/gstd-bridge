pub mod vault;

use serde::{Deserialize, Serialize};
use crate::chains::DepositEvent;
use crate::consensus::{Vote, TransferId};

/// Messages exchanged between bridge nodes via Gossipsub
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeMessage {
    /// A node detected a deposit and proposes a transfer
    ProposeTransfer {
        deposit: DepositEvent,
        proposer: String,
    },
    /// A validator votes on a proposed transfer
    CastVote(Vote),
    /// A transfer was executed (withdrawal sent)
    TransferExecuted {
        transfer_id: TransferId,
        tx_hash: String,
        executor: String,
    },
    /// State sync: share current state hash
    StateSync {
        epoch: u64,
        state_hash: String,
        vault_balances: std::collections::HashMap<String, u64>,
    },
    /// Heartbeat: node is alive and in the validator set
    ValidatorHeartbeat {
        peer_id: String,
        version: String,
        uptime_secs: u64,
        chains_monitoring: Vec<String>,
    },
}

impl BridgeMessage {
    /// Get a human-readable kind for logging
    pub fn kind(&self) -> &str {
        match self {
            BridgeMessage::ProposeTransfer { .. } => "ProposeTransfer",
            BridgeMessage::CastVote(_) => "CastVote",
            BridgeMessage::TransferExecuted { .. } => "TransferExecuted",
            BridgeMessage::StateSync { .. } => "StateSync",
            BridgeMessage::ValidatorHeartbeat { .. } => "ValidatorHeartbeat",
        }
    }
}
