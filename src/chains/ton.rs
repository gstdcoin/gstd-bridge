use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::{Chain, ChainMonitor, DepositEvent};
use crate::config::ChainEndpoint;

/// TON blockchain monitor
/// Watches for incoming GSTD transfers to the vault address via TonCenter API
pub struct TonMonitor {
    config: ChainEndpoint,
    client: reqwest::Client,
}

impl TonMonitor {
    pub fn new(config: ChainEndpoint) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("http client"),
        }
    }

    /// Parse TON transaction for GSTD transfer to vault
    fn parse_deposit(&self, tx: &serde_json::Value) -> Option<DepositEvent> {
        // Extract transfer fields from TON transaction
        let hash = tx.get("transaction_id")?.get("hash")?.as_str()?;
        let in_msg = tx.get("in_msg")?;
        let source = in_msg.get("source")?.as_str()?;

        // Parse message body for bridge memo:  "bridge:<target_chain>:<recipient>"
        let msg_body = in_msg.get("message")?.as_str().unwrap_or("");
        let parts: Vec<&str> = msg_body.split(':').collect();
        if parts.len() < 3 || parts[0] != "bridge" {
            return None;
        }

        let target_chain = match parts[1].to_lowercase().as_str() {
            "solana" | "sol" => Chain::Solana,
            "xrpl" | "xrp" => Chain::XRPL,
            _ => return None,
        };
        let recipient = parts[2].to_string();

        let amount = in_msg.get("value")?.as_str()?.parse::<u64>().ok()?;
        let utime = tx.get("utime")?.as_u64()?;

        Some(DepositEvent {
            tx_hash: hash.to_string(),
            source_chain: Chain::TON,
            target_chain,
            sender: source.to_string(),
            recipient,
            amount,
            block_number: 0, // TON uses logical time
            timestamp: utime,
        })
    }
}

#[async_trait]
impl ChainMonitor for TonMonitor {
    fn chain(&self) -> Chain {
        Chain::TON
    }

    async fn start_monitoring(
        &self,
        deposit_tx: mpsc::UnboundedSender<DepositEvent>,
    ) -> anyhow::Result<()> {
        tracing::info!("🔵 TON Monitor started — vault: {}", self.config.vault_address);
        let mut last_lt = String::new();
        let interval = std::time::Duration::from_secs(self.config.poll_interval_secs);

        loop {
            // Fetch recent transactions for vault address
            let url = format!(
                "{}/getTransactions?address={}&limit=20",
                self.config.rpc_url, self.config.vault_address
            );

            match self.client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        if let Some(result) = body.get("result").and_then(|r| r.as_array()) {
                            for tx in result {
                                // Skip already processed
                                if let Some(lt) = tx.get("transaction_id")
                                    .and_then(|t| t.get("lt"))
                                    .and_then(|l| l.as_str())
                                {
                                    if lt <= last_lt.as_str() {
                                        continue;
                                    }
                                    last_lt = lt.to_string();
                                }

                                if let Some(deposit) = self.parse_deposit(tx) {
                                    tracing::info!(
                                        "💎 TON deposit detected: {} GSTD from {} → {} on {}",
                                        deposit.amount,
                                        deposit.sender,
                                        deposit.recipient,
                                        deposit.target_chain,
                                    );
                                    let _ = deposit_tx.send(deposit);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("TON poll error: {e}");
                }
            }

            tokio::time::sleep(interval).await;
        }
    }

    async fn execute_withdrawal(
        &self,
        recipient: &str,
        amount: u64,
        _signature_shares: Vec<Vec<u8>>,
    ) -> anyhow::Result<String> {
        // In production: construct TON transfer with MPC-aggregated signature
        // For now: log and return placeholder
        tracing::info!(
            "📤 TON withdrawal: {amount} GSTD → {recipient} (pending MPC signature)"
        );
        Ok(format!("ton_tx_{}", uuid::Uuid::new_v4()))
    }

    async fn vault_balance(&self) -> anyhow::Result<u64> {
        let url = format!(
            "{}/getAddressBalance?address={}",
            self.config.rpc_url, self.config.vault_address
        );
        let resp = self.client.get(&url).send().await?.json::<serde_json::Value>().await?;
        let balance = resp.get("result")
            .and_then(|r| r.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        Ok(balance)
    }

    async fn verify_deposit(&self, tx_hash: &str) -> anyhow::Result<bool> {
        // Verify transaction exists and has sufficient confirmations
        tracing::debug!("Verifying TON tx: {tx_hash}");
        // In production: query TonCenter and verify confirmations
        Ok(true)
    }
}
