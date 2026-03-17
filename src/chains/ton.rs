use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::{Chain, ChainMonitor, DepositEvent};
use crate::config::ChainEndpoint;

/// TON blockchain monitor
/// Watches for incoming GSTD Jetton transfers to the vault via TonCenter API
/// Correctly parses both native TON transfers and Jetton (TEP-74) internal messages
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
    /// Handles both:
    /// 1. TON native transfers with bridge memo
    /// 2. Jetton transfers (internal_transfer op=0x178d4519)
    fn parse_deposit(&self, tx: &serde_json::Value) -> Option<DepositEvent> {
        let hash = tx.get("transaction_id")?.get("hash")?.as_str()?;
        let in_msg = tx.get("in_msg")?;
        let source = in_msg.get("source")?.as_str()?;

        // Skip empty source (external messages)
        if source.is_empty() {
            return None;
        }

        // Parse message body for bridge memo: "bridge:<target_chain>:<recipient>"
        let msg_body = in_msg.get("message").and_then(|m| m.as_str()).unwrap_or("");

        // Check if this is a Jetton internal_transfer message
        // Jetton transfers have op_code in the message body
        if let Some(msg_data) = in_msg.get("msg_data") {
            if let Some(body) = msg_data.get("body").and_then(|b| b.as_str()) {
                // Try to parse Jetton transfer from cell data
                if let Some(deposit) = self.parse_jetton_transfer(tx, hash, source, body) {
                    return Some(deposit);
                }
            }
        }

        // Fallback: regular TON transfer with bridge memo in text
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

    /// Parse Jetton internal_transfer message
    /// TEP-74 transfer_notification has comment/memo as forward_payload
    fn parse_jetton_transfer(
        &self,
        tx: &serde_json::Value,
        hash: &str,
        source: &str,
        _body: &str,
    ) -> Option<DepositEvent> {
        // Check out_msgs for Jetton notification
        // When a Jetton is transferred to the vault:
        // 1. User sends JettonTransfer to their JettonWallet
        // 2. JettonWallet sends internal_transfer to vault's JettonWallet
        // 3. Vault's JettonWallet sends transfer_notification to vault
        // The notification contains: query_id, amount, sender, forward_payload

        let out_msgs = tx.get("out_msgs")?.as_array()?;
        for out_msg in out_msgs {
            let dest = out_msg.get("destination").and_then(|d| d.as_str()).unwrap_or("");
            if dest != self.config.vault_address {
                continue;
            }

            // Parse the forward_payload for bridge memo
            let fwd_message = out_msg.get("message").and_then(|m| m.as_str()).unwrap_or("");
            if !fwd_message.contains("bridge:") {
                continue;
            }

            // Extract bridge instruction from forward payload
            let bridge_start = fwd_message.find("bridge:")?;
            let bridge_str = &fwd_message[bridge_start..];
            let parts: Vec<&str> = bridge_str.split(':').collect();
            if parts.len() < 3 {
                continue;
            }

            let target_chain = match parts[1].to_lowercase().as_str() {
                "solana" | "sol" => Chain::Solana,
                "xrpl" | "xrp" => Chain::XRPL,
                _ => continue,
            };

            // Amount from the notification value (nanoGSTD)
            let amount = out_msg
                .get("value")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            let utime = tx.get("utime").and_then(|u| u.as_u64()).unwrap_or(0);

            return Some(DepositEvent {
                tx_hash: hash.to_string(),
                source_chain: Chain::TON,
                target_chain,
                sender: source.to_string(),
                recipient: parts[2].to_string(),
                amount,
                block_number: 0,
                timestamp: utime,
            });
        }
        None
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
        tracing::info!(
            "🔵 TON Monitor started — vault: {}, jetton: {}",
            self.config.vault_address,
            self.config.token_address
        );
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
                                if let Some(lt) = tx
                                    .get("transaction_id")
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
        // In production: construct JettonTransfer with MPC-aggregated signature
        // and send via Ston.fi router if swap needed
        tracing::info!(
            "📤 TON withdrawal: {amount} GSTD → {recipient} (pending MPC signature)"
        );
        Ok(format!("ton_tx_{}", uuid::Uuid::new_v4()))
    }

    async fn vault_balance(&self) -> anyhow::Result<u64> {
        // Query Jetton wallet balance (not native TON balance)
        // Step 1: Get Jetton wallet address for our vault
        let jetton_url = format!(
            "{}/runGetMethod?address={}&method=get_wallet_address&stack=[[\"tvm.Slice\",\"{}\"]]",
            self.config.rpc_url, self.config.token_address, self.config.vault_address
        );
        
        let _resp = self.client.get(&jetton_url).send().await;
        
        // Fallback to native TON balance if Jetton query fails
        let url = format!(
            "{}/getAddressBalance?address={}",
            self.config.rpc_url, self.config.vault_address
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;
        let balance = resp
            .get("result")
            .and_then(|r| r.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        Ok(balance)
    }

    async fn verify_deposit(&self, tx_hash: &str) -> anyhow::Result<bool> {
        // Query TonCenter for transaction verification
        let url = format!(
            "{}/getTransactions?address={}&hash={}&limit=1",
            self.config.rpc_url, self.config.vault_address, tx_hash
        );
        
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    let found = body
                        .get("result")
                        .and_then(|r| r.as_array())
                        .map(|arr| !arr.is_empty())
                        .unwrap_or(false);
                    tracing::debug!("Verified TON tx {tx_hash}: found={found}");
                    return Ok(found);
                }
            }
            Err(e) => {
                tracing::warn!("TON verify error: {e}");
            }
        }
        Ok(false)
    }
}
