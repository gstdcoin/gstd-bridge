use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::chains::Chain;

/// Shared state table replicated across P2P network
/// Tracks vault balances, active transfers, and node state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedState {
    /// Current vault balances per chain (as reported by monitors)
    pub vault_balances: HashMap<String, u64>,
    /// Total locked across all chains
    pub total_locked: u64,
    /// Total unlocked (withdrawn)
    pub total_unlocked: u64,
    /// Epoch number (increments on each state update)
    pub epoch: u64,
    /// Last state hash (for integrity verification)
    pub state_hash: String,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            vault_balances: HashMap::from([
                (Chain::TON.to_string(), 0),
                (Chain::Solana.to_string(), 0),
                (Chain::XRPL.to_string(), 0),
            ]),
            total_locked: 0,
            total_unlocked: 0,
            epoch: 0,
            state_hash: String::new(),
        }
    }

    /// Update vault balance for a chain
    pub fn update_balance(&mut self, chain: Chain, balance: u64) {
        self.vault_balances.insert(chain.to_string(), balance);
        self.epoch += 1;
        self.recompute_hash();
    }

    /// Record a lock (deposit received)
    pub fn record_lock(&mut self, chain: Chain, amount: u64) {
        let current = self.vault_balances.get(&chain.to_string()).copied().unwrap_or(0);
        self.vault_balances.insert(chain.to_string(), current + amount);
        self.total_locked += amount;
        self.epoch += 1;
        self.recompute_hash();
    }

    /// Record an unlock (withdrawal sent)
    pub fn record_unlock(&mut self, chain: Chain, amount: u64) {
        let current = self.vault_balances.get(&chain.to_string()).copied().unwrap_or(0);
        self.vault_balances.insert(chain.to_string(), current.saturating_sub(amount));
        self.total_unlocked += amount;
        self.epoch += 1;
        self.recompute_hash();
    }

    /// Compute integrity hash
    fn recompute_hash(&mut self) {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.epoch.to_le_bytes());
        hasher.update(self.total_locked.to_le_bytes());
        hasher.update(self.total_unlocked.to_le_bytes());
        for (chain, balance) in &self.vault_balances {
            hasher.update(chain.as_bytes());
            hasher.update(balance.to_le_bytes());
        }
        self.state_hash = hex::encode(hasher.finalize());
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}
