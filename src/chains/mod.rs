pub mod ton;
pub mod solana;
pub mod xrpl;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported blockchain networks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Chain {
    TON,
    Solana,
    XRPL,
}

impl fmt::Display for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Chain::TON => write!(f, "TON"),
            Chain::Solana => write!(f, "Solana"),
            Chain::XRPL => write!(f, "XRPL"),
        }
    }
}

/// Detected deposit on a chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositEvent {
    /// Unique transaction hash on source chain
    pub tx_hash: String,
    /// Source chain
    pub source_chain: Chain,
    /// Target chain for the bridge
    pub target_chain: Chain,
    /// Sender address on source chain
    pub sender: String,
    /// Recipient address on target chain
    pub recipient: String,
    /// Amount of GSTD tokens (base units)
    pub amount: u64,
    /// Block number / ledger index where detected
    pub block_number: u64,
    /// Unix timestamp
    pub timestamp: u64,
}

/// Trait for chain monitoring workers
#[async_trait]
pub trait ChainMonitor: Send + Sync {
    /// Which chain this monitor watches
    fn chain(&self) -> Chain;

    /// Start monitoring for deposits to the vault address
    /// Sends detected deposits through the channel
    async fn start_monitoring(
        &self,
        deposit_tx: tokio::sync::mpsc::UnboundedSender<DepositEvent>,
    ) -> anyhow::Result<()>;

    /// Execute a withdrawal (unlock) from the vault
    /// Returns the transaction hash
    async fn execute_withdrawal(
        &self,
        recipient: &str,
        amount: u64,
        signature_shares: Vec<Vec<u8>>,
    ) -> anyhow::Result<String>;

    /// Check current vault balance
    async fn vault_balance(&self) -> anyhow::Result<u64>;

    /// Verify that a deposit transaction is valid and confirmed
    async fn verify_deposit(&self, tx_hash: &str) -> anyhow::Result<bool>;
}
