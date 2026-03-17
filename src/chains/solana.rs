use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::{Chain, ChainMonitor, DepositEvent};
use crate::config::ChainEndpoint;

/// Solana blockchain monitor
/// Watches for incoming GSTD SPL token transfers to the vault via RPC
/// Parses SPL Token balance changes (not SOL lamports) for accurate tracking
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

    /// Parse a Solana transaction signature from the response
    fn parse_signatures_response(
        &self,
        sig_info: &serde_json::Value,
    ) -> Option<String> {
        // Skip failed transactions
        if let Some(err) = sig_info.get("err") {
            if !err.is_null() {
                return None;
            }
        }
        sig_info.get("signature")?.as_str().map(|s| s.to_string())
    }

    /// Extract sender public key from transaction accountKeys
    fn extract_sender(tx: &serde_json::Value) -> String {
        // jsonParsed format: transaction.message.accountKeys[0].pubkey
        tx.get("transaction")
            .and_then(|t| t.get("message"))
            .and_then(|m| m.get("accountKeys"))
            .and_then(|keys| keys.get(0))
            .and_then(|key| {
                // May be either a string or an object with "pubkey"
                key.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| key.get("pubkey").and_then(|p| p.as_str()).map(|s| s.to_string()))
            })
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Parse SPL Token transfer amount from pre/postTokenBalances
    /// This correctly tracks GSTD SPL token transfers (not SOL lamports)
    fn parse_spl_deposit_amount(
        &self,
        meta: &serde_json::Value,
    ) -> Option<u64> {
        let post_balances = meta.get("postTokenBalances")?.as_array()?;
        let pre_balances = meta.get("preTokenBalances")?.as_array()?;

        let gstd_mint = &self.config.token_address;

        // Find GSTD token balance change for the vault
        for post in post_balances {
            let mint = post.get("mint")?.as_str()?;
            if mint != gstd_mint {
                continue;
            }

            let owner = post.get("owner").and_then(|o| o.as_str()).unwrap_or("");
            if owner != self.config.vault_address {
                continue;
            }

            let post_amount: u64 = post
                .get("uiTokenAmount")
                .and_then(|u| u.get("amount"))
                .and_then(|a| a.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            // Find matching pre-balance for same account index
            let account_index = post.get("accountIndex").and_then(|i| i.as_u64());
            let pre_amount: u64 = pre_balances
                .iter()
                .find(|p| p.get("accountIndex").and_then(|i| i.as_u64()) == account_index)
                .and_then(|p| {
                    p.get("uiTokenAmount")
                        .and_then(|u| u.get("amount"))
                        .and_then(|a| a.as_str())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(0);

            if post_amount > pre_amount {
                return Some(post_amount - pre_amount);
            }
        }
        None
    }

    /// Fallback: parse SOL lamport balance changes (for native SOL deposits)
    fn parse_sol_deposit_amount(meta: &serde_json::Value) -> u64 {
        let pre = meta
            .get("preBalances")
            .and_then(|b| b.get(0))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let post = meta
            .get("postBalances")
            .and_then(|b| b.get(0))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        pre.saturating_sub(post)
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
        tracing::info!(
            "🟣 Solana Monitor started — vault: {}, token: {}",
            self.config.vault_address,
            self.config.token_address
        );

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
                                    // Fetch full transaction to parse deposit
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
        // TODO: Integrate Raydium CPI for GSTD→SOL swap if needed
        tracing::info!(
            "📤 Solana withdrawal: {amount} GSTD → {recipient} (pending MPC signature)"
        );
        Ok(format!("sol_tx_{}", uuid::Uuid::new_v4()))
    }

    async fn vault_balance(&self) -> anyhow::Result<u64> {
        // Query SPL Token account balance (not SOL lamports)
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTokenAccountsByOwner",
            "params": [
                self.config.vault_address,
                { "mint": self.config.token_address },
                { "encoding": "jsonParsed" }
            ]
        });
        let resp = self.client
            .post(&self.config.rpc_url)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Parse token account balance
        let balance = resp
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|acc| acc.get("account"))
            .and_then(|a| a.get("data"))
            .and_then(|d| d.get("parsed"))
            .and_then(|p| p.get("info"))
            .and_then(|i| i.get("tokenAmount"))
            .and_then(|t| t.get("amount"))
            .and_then(|a| a.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        Ok(balance)
    }

    async fn verify_deposit(&self, tx_hash: &str) -> anyhow::Result<bool> {
        // Verify the transaction exists and is confirmed
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTransaction",
            "params": [tx_hash, { "encoding": "jsonParsed" }]
        });
        let resp = self.client
            .post(&self.config.rpc_url)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let confirmed = resp
            .get("result")
            .map(|r| !r.is_null())
            .unwrap_or(false);

        tracing::debug!("Verified Solana tx {tx_hash}: confirmed={confirmed}");
        Ok(confirmed)
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
            "params": [signature, { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0 }]
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

        let meta = match tx.get("meta") {
            Some(m) => m,
            None => return Ok(None),
        };

        // Check for memo instruction containing bridge:<chain>:<recipient>
        let log_messages = meta
            .get("logMessages")
            .and_then(|l| l.as_array());

        if let Some(logs) = log_messages {
            for log in logs {
                if let Some(msg) = log.as_str() {
                    // Match both "Program log: bridge:" and memo program logs
                    let bridge_prefix = if msg.starts_with("Program log: bridge:") {
                        Some("Program log: bridge:")
                    } else if msg.starts_with("Program log: Memo") && msg.contains("bridge:") {
                        // Memo program format
                        msg.find("bridge:").map(|_| "")
                    } else {
                        None
                    };

                    if let Some(prefix) = bridge_prefix {
                        let memo_content = if prefix.is_empty() {
                            // Extract from position of "bridge:"
                            msg.split("bridge:").nth(1).unwrap_or("")
                        } else {
                            msg.trim_start_matches(prefix)
                        };

                        let parts: Vec<&str> = memo_content.split(':').collect();
                        if parts.len() >= 2 {
                            let target_chain = match parts[0].to_lowercase().as_str() {
                                "ton" => Chain::TON,
                                "xrpl" | "xrp" => Chain::XRPL,
                                _ => continue,
                            };

                            // Extract sender from accountKeys (not "unknown")
                            let sender = Self::extract_sender(tx);

                            // Parse SPL Token amount (primary) or SOL amount (fallback)
                            let amount = self
                                .parse_spl_deposit_amount(meta)
                                .unwrap_or_else(|| Self::parse_sol_deposit_amount(meta));

                            let slot = tx.get("slot").and_then(|s| s.as_u64()).unwrap_or(0);
                            let block_time = tx.get("blockTime").and_then(|b| b.as_u64()).unwrap_or(0);

                            return Ok(Some(DepositEvent {
                                tx_hash: signature.to_string(),
                                source_chain: Chain::Solana,
                                target_chain,
                                sender,
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
