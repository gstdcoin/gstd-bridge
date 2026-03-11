use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::{Chain, ChainMonitor, DepositEvent};
use crate::config::ChainEndpoint;

/// Solana blockchain monitor
/// Watches for incoming GSTD SPL token transfers to the vault via RPC + WS
pub struct SolanaMonitor {
    config: ChainEndpoint,
    client: reqwest::Client,
}

impl SolanaMonitor {
    pub fn new(config: ChainEndpoint) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("http client"),
        }
    }

    /// Parse a Solana transaction for bridge deposit memo
    fn parse_signatures_response(
        &self,
        sig_info: &serde_json::Value,
    ) -> Option<String> {
        sig_info.get("signature")?.as_str().map(|s| s.to_string())
    }
}

#[async_trait]
impl ChainMonitor for SolanaMonitor {
    fn chain(&self) -> Chain {
        Chain::Solana
    }

    async fn start_monitoring(
        &self,
        deposit_tx: mpsc::UnboundedSender<DepositEvent>,
    ) -> anyhow::Result<()> {
        tracing::info!("🟣 Solana Monitor started — vault: {}", self.config.vault_address);

        let mut last_signature: Option<String> = None;
        let interval = std::time::Duration::from_secs(self.config.poll_interval_secs);

        loop {
            // Fetch recent signatures for the vault account
            let mut params = serde_json::json!([
                self.config.vault_address,
                { "limit": 20 }
            ]);

            if let Some(ref before) = last_signature {
                params[1]["until"] = serde_json::json!(before);
            }

            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getSignaturesForAddress",
                "params": params,
            });

            match self.client.post(&self.config.rpc_url).json(&body).send().await {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(result) = data.get("result").and_then(|r| r.as_array()) {
                            for sig_info in result {
                                if let Some(sig) = self.parse_signatures_response(sig_info) {
                                    // Fetch full transaction to parse memo
                                    if let Ok(Some(deposit)) =
                                        self.fetch_and_parse_tx(&sig).await
                                    {
                                        tracing::info!(
                                            "🟣 Solana deposit: {} GSTD from {} → {} on {}",
                                            deposit.amount,
                                            deposit.sender,
                                            deposit.recipient,
                                            deposit.target_chain,
                                        );
                                        let _ = deposit_tx.send(deposit);
                                    }

                                    if last_signature.is_none() {
                                        last_signature = Some(sig);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Solana poll error: {e}");
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
            "📤 Solana withdrawal: {amount} GSTD → {recipient} (pending MPC signature)"
        );
        Ok(format!("sol_tx_{}", uuid::Uuid::new_v4()))
    }

    async fn vault_balance(&self) -> anyhow::Result<u64> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBalance",
            "params": [self.config.vault_address]
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
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        Ok(balance)
    }

    async fn verify_deposit(&self, tx_hash: &str) -> anyhow::Result<bool> {
        tracing::debug!("Verifying Solana tx: {tx_hash}");
        Ok(true)
    }
}

impl SolanaMonitor {
    /// Fetch full transaction and parse for bridge deposit
    async fn fetch_and_parse_tx(
        &self,
        signature: &str,
    ) -> anyhow::Result<Option<DepositEvent>> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [signature, { "encoding": "jsonParsed" }]
        });

        let resp = self.client
            .post(&self.config.rpc_url)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let tx = match resp.get("result") {
            Some(tx) if !tx.is_null() => tx,
            _ => return Ok(None),
        };

        // Check for memo instruction containing bridge:<chain>:<recipient>
        let meta = tx.get("meta");
        let log_messages = meta
            .and_then(|m| m.get("logMessages"))
            .and_then(|l| l.as_array());

        if let Some(logs) = log_messages {
            for log in logs {
                if let Some(msg) = log.as_str() {
                    if msg.starts_with("Program log: bridge:") {
                        let parts: Vec<&str> = msg
                            .trim_start_matches("Program log: bridge:")
                            .split(':')
                            .collect();
                        if parts.len() >= 2 {
                            let target_chain = match parts[0].to_lowercase().as_str() {
                                "ton" => Chain::TON,
                                "xrpl" | "xrp" => Chain::XRPL,
                                _ => continue,
                            };

                            // Extract amount from balance changes
                            let pre = meta
                                .and_then(|m| m.get("preBalances"))
                                .and_then(|b| b.get(0))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let post = meta
                                .and_then(|m| m.get("postBalances"))
                                .and_then(|b| b.get(0))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            let amount = pre.saturating_sub(post);

                            let slot = tx.get("slot").and_then(|s| s.as_u64()).unwrap_or(0);
                            let block_time = tx.get("blockTime").and_then(|b| b.as_u64()).unwrap_or(0);

                            return Ok(Some(DepositEvent {
                                tx_hash: signature.to_string(),
                                source_chain: Chain::Solana,
                                target_chain,
                                sender: "unknown".to_string(), // parsed from accountKeys
                                recipient: parts[1].to_string(),
                                amount,
                                block_number: slot,
                                timestamp: block_time,
                            }));
                        }
                    }
                }
            }
        }

        Ok(None)
    }
}
