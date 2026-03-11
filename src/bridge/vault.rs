use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::chains::Chain;

/// Vault manager — tracks locked liquidity across chains
/// Lock-and-Unlock model: tokens are locked in source vault,
/// unlocked from target vault. No minting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultManager {
    /// Current locked balance per chain
    balances: HashMap<String, u64>,
    /// Total locked across all chains
    pub total_locked: u64,
    /// Historical transfers count
    pub transfer_count: u64,
}

impl VaultManager {
    pub fn new() -> Self {
        Self {
            balances: HashMap::from([
                (Chain::TON.to_string(), 0),
                (Chain::Solana.to_string(), 0),
                (Chain::XRPL.to_string(), 0),
            ]),
            total_locked: 0,
            transfer_count: 0,
        }
    }

    /// Record a lock (deposit received on source chain)
    pub fn lock(&mut self, chain: Chain, amount: u64) {
        let balance = self.balances.entry(chain.to_string()).or_insert(0);
        *balance += amount;
        self.total_locked += amount;
        tracing::info!("🔒 Locked {amount} GSTD on {chain} (vault: {})", *balance);
    }

    /// Record an unlock (withdrawal sent from target chain)
    /// Returns false if insufficient liquidity
    pub fn unlock(&mut self, chain: Chain, amount: u64) -> bool {
        let balance = self.balances.entry(chain.to_string()).or_insert(0);
        if *balance < amount {
            tracing::warn!(
                "⚠️ Insufficient liquidity on {chain}: need {amount}, have {}",
                *balance
            );
            return false;
        }
        *balance -= amount;
        self.transfer_count += 1;
        tracing::info!(
            "🔓 Unlocked {amount} GSTD on {chain} (vault: {}, total txs: {})",
            *balance,
            self.transfer_count
        );
        true
    }

    /// Get balance for a specific chain
    pub fn balance(&self, chain: Chain) -> u64 {
        self.balances.get(&chain.to_string()).copied().unwrap_or(0)
    }

    /// Get all balances
    pub fn all_balances(&self) -> &HashMap<String, u64> {
        &self.balances
    }

    /// Sync balances from chain monitors
    pub fn sync_balance(&mut self, chain: Chain, actual_balance: u64) {
        self.balances.insert(chain.to_string(), actual_balance);
    }
}

impl Default for VaultManager {
    fn default() -> Self {
        Self::new()
    }
}
