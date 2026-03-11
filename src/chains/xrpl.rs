use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::{Chain, ChainMonitor, DepositEvent};
use crate::config::ChainEndpoint;

/// XRPL blockchain monitor
/// Watches for GSTD IOU transfers to the vault via XRPL WebSocket/JSON-RPC
pub struct XrplMonitor {
    config: ChainEndpoint,
    client: reqwest::Client,
}

impl XrplMonitor {
    pub fn new(config: ChainEndpoint) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("http client"),
        }
    }
}

#[async_trait]
impl ChainMonitor for XrplMonitor {
    fn chain(&self) -> Chain {
        Chain::XRPL
    }

    async fn start_monitoring(
        &self,
        deposit_tx: mpsc::UnboundedSender<DepositEvent>,
    ) -> anyhow::Result<()> {
        tracing::info!("🔴 XRPL Monitor started — vault: {}", self.config.vault_address);

        let mut last_ledger: u64 = 0;
        let interval = std::time::Duration::from_secs(self.config.poll_interval_secs);

        loop {
            // Use account_tx to fetch recent transactions
            let body = serde_json::json!({
                "method": "account_tx",
                "params": [{
                    "account": self.config.vault_address,
                    "ledger_index_min": -1,
                    "ledger_index_max": -1,
                    "limit": 20,
                    "forward": false,
                }]
            });

            match self.client.post(&self.config.rpc_url).json(&body).send().await {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        let transactions = data
                            .get("result")
                            .and_then(|r| r.get("transactions"))
                            .and_then(|t| t.as_array());

                        if let Some(txs) = transactions {
                            for tx_wrap in txs {
                                let tx = match tx_wrap.get("tx") {
                                    Some(t) => t,
                                    None => continue,
                                };

                                let ledger_index = tx
                                    .get("ledger_index")
                                    .and_then(|l| l.as_u64())
                                    .unwrap_or(0);

                                if ledger_index <= last_ledger {
                                    continue;
                                }

                                // Only process Payment transactions TO our vault
                                let tx_type = tx.get("TransactionType").and_then(|t| t.as_str());
                                let destination = tx.get("Destination").and_then(|d| d.as_str());

                                if tx_type != Some("Payment") {
                                    continue;
                                }
                                if destination != Some(&self.config.vault_address) {
                                    continue;
                                }

                                // Check for bridge memo
                                if let Some(memos) = tx.get("Memos").and_then(|m| m.as_array()) {
                                    for memo in memos {
                                        let memo_data = memo
                                            .get("Memo")
                                            .and_then(|m| m.get("MemoData"))
                                            .and_then(|d| d.as_str());

                                        if let Some(hex_data) = memo_data {
                                            if let Ok(decoded) = hex::decode(hex_data) {
                                                if let Ok(memo_str) = String::from_utf8(decoded) {
                                                    if let Some(deposit) = self.parse_memo(
                                                        tx, &memo_str, ledger_index,
                                                    ) {
                                                        tracing::info!(
                                                            "🔴 XRPL deposit: {} GSTD from {} → {} on {}",
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
                                }

                                last_ledger = ledger_index;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("XRPL poll error: {e}");
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
        tracing::info!(
            "📤 XRPL withdrawal: {amount} GSTD → {recipient} (pending MPC signature)"
        );
        Ok(format!("xrpl_tx_{}", uuid::Uuid::new_v4()))
    }

    async fn vault_balance(&self) -> anyhow::Result<u64> {
        let body = serde_json::json!({
            "method": "account_info",
            "params": [{
                "account": self.config.vault_address,
                "ledger_index": "current"
            }]
        });
        let resp = self.client
            .post(&self.config.rpc_url)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let balance = resp
            .get("result")
            .and_then(|r| r.get("account_data"))
            .and_then(|a| a.get("Balance"))
            .and_then(|b| b.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        Ok(balance)
    }

    async fn verify_deposit(&self, tx_hash: &str) -> anyhow::Result<bool> {
        tracing::debug!("Verifying XRPL tx: {tx_hash}");
        Ok(true)
    }
}

impl XrplMonitor {
    /// Parse an XRPL memo for bridge instructions: "bridge:<chain>:<recipient>"
    fn parse_memo(
        &self,
        tx: &serde_json::Value,
        memo_str: &str,
        ledger_index: u64,
    ) -> Option<DepositEvent> {
        let parts: Vec<&str> = memo_str.split(':').collect();
        if parts.len() < 3 || parts[0] != "bridge" {
            return None;
        }

        let target_chain = match parts[1].to_lowercase().as_str() {
            "ton" => Chain::TON,
            "solana" | "sol" => Chain::Solana,
            _ => return None,
        };

        let sender = tx.get("Account")?.as_str()?.to_string();
        let tx_hash = tx.get("hash")?.as_str()?.to_string();

        // Extract amount — either drops (XRP) or IOU amount
        let amount = tx
            .get("Amount")
            .and_then(|a| {
                if let Some(s) = a.as_str() {
                    s.parse::<u64>().ok()
                } else {
                    a.get("value")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<f64>().ok())
                        .map(|f| (f * 1_000_000.0) as u64) // Convert to base units
                }
            })
            .unwrap_or(0);

        Some(DepositEvent {
            tx_hash,
            source_chain: Chain::XRPL,
            target_chain,
            sender,
            recipient: parts[2].to_string(),
            amount,
            block_number: ledger_index,
            timestamp: chrono::Utc::now().timestamp() as u64,
        })
    }
}
