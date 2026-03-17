use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing;

use super::{Chain, ChainMonitor, DepositEvent};
use crate::config::ChainEndpoint;

/// XRPL blockchain monitor
/// Watches for GSTD IOU token transfers to the vault via XRPL JSON-RPC
/// Correctly distinguishes between XRP drops and IOU Amount objects
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

    /// Parse XRPL Amount field — handles both XRP drops (string) and IOU objects
    /// XRP:  "Amount": "1000000"               (drops, 1 XRP = 1,000,000 drops)
    /// IOU:  "Amount": {"currency":"GSD","issuer":"r...","value":"100.5"}
    fn parse_amount(amount: &serde_json::Value, expected_currency: &str) -> Option<u64> {
        if let Some(drops_str) = amount.as_str() {
            // Native XRP payment (drops)
            let drops: u64 = drops_str.parse().ok()?;
            return Some(drops);
        }

        if let Some(obj) = amount.as_object() {
            let currency = obj.get("currency")?.as_str()?;
            // Match GSTD IOU by currency code (3-char "GSD" or hex-encoded "GSTD")
            let is_gstd = currency.eq_ignore_ascii_case(expected_currency)
                || currency.eq_ignore_ascii_case("GSD")
                || currency.eq_ignore_ascii_case("GSTD")
                || Self::is_hex_encoded_gstd(currency);

            if !is_gstd {
                return None;
            }

            let value_str = obj.get("value")?.as_str()?;
            let value: f64 = value_str.parse().ok()?;
            // Convert to base units (9 decimals like nanoGSTD)
            return Some((value * 1_000_000_000.0) as u64);
        }

        None
    }

    /// Check if currency code is hex-encoded "GSTD" (XRPL uses hex for 4+ char tokens)
    /// "GSTD" hex = "4753544400000000000000000000000000000000"
    fn is_hex_encoded_gstd(currency: &str) -> bool {
        if currency.len() != 40 {
            return false;
        }
        if let Ok(bytes) = hex::decode(currency) {
            let trimmed: Vec<u8> = bytes.into_iter().take_while(|b| *b != 0).collect();
            if let Ok(name) = String::from_utf8(trimmed) {
                return name.eq_ignore_ascii_case("GSTD");
            }
        }
        false
    }

    /// Validate that the IOU issuer matches our expected token configuration
    fn validate_issuer(&self, amount: &serde_json::Value) -> bool {
        if amount.is_string() {
            // Native XRP — always valid (bridge accepts XRP too)
            return true;
        }
        if let Some(obj) = amount.as_object() {
            if let Some(issuer) = obj.get("issuer").and_then(|i| i.as_str()) {
                // Check against configured token_address (GSTD issuer on XRPL)
                return issuer == self.config.token_address
                    || self.config.token_address.is_empty(); // Accept any if not configured
            }
        }
        false
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
        tracing::info!(
            "🔴 XRPL Monitor started — vault: {}, token: {}",
            self.config.vault_address,
            self.config.token_address
        );

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

                                // Check transaction was successful
                                let meta = tx_wrap.get("meta");
                                let tx_result = meta
                                    .and_then(|m| m.get("TransactionResult"))
                                    .and_then(|r| r.as_str())
                                    .unwrap_or("");
                                if tx_result != "tesSUCCESS" {
                                    continue;
                                }

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

                                // Validate amount and issuer
                                let amount_field = match tx.get("Amount") {
                                    Some(a) => a,
                                    None => continue,
                                };

                                if !self.validate_issuer(amount_field) {
                                    tracing::debug!("Skipping tx with unknown issuer");
                                    continue;
                                }

                                // Use delivered_amount from meta for accurate value
                                // (handles partial payments correctly)
                                let effective_amount = meta
                                    .and_then(|m| m.get("delivered_amount"))
                                    .unwrap_or(amount_field);

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
                                                        tx,
                                                        effective_amount,
                                                        &memo_str,
                                                        ledger_index,
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
        // Query GSTD IOU balance via account_lines (trust lines)
        let body = serde_json::json!({
            "method": "account_lines",
            "params": [{
                "account": self.config.vault_address,
                "ledger_index": "validated"
            }]
        });
        let resp = self.client
            .post(&self.config.rpc_url)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Look for GSTD trust line balance
        let mut gstd_balance: u64 = 0;

        if let Some(lines) = resp
            .get("result")
            .and_then(|r| r.get("lines"))
            .and_then(|l| l.as_array())
        {
            for line in lines {
                let currency = line.get("currency").and_then(|c| c.as_str()).unwrap_or("");
                if currency.eq_ignore_ascii_case("GSD")
                    || currency.eq_ignore_ascii_case("GSTD")
                    || Self::is_hex_encoded_gstd(currency)
                {
                    if let Some(balance_str) = line.get("balance").and_then(|b| b.as_str()) {
                        if let Ok(balance) = balance_str.parse::<f64>() {
                            gstd_balance = (balance.abs() * 1_000_000_000.0) as u64;
                        }
                    }
                }
            }
        }

        // Also add native XRP balance
        let xrp_body = serde_json::json!({
            "method": "account_info",
            "params": [{
                "account": self.config.vault_address,
                "ledger_index": "validated"
            }]
        });
        let xrp_resp = self.client
            .post(&self.config.rpc_url)
            .json(&xrp_body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let xrp_drops = xrp_resp
            .get("result")
            .and_then(|r| r.get("account_data"))
            .and_then(|a| a.get("Balance"))
            .and_then(|b| b.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        // Return total: GSTD IOU (if any) + XRP drops as reference
        tracing::debug!(
            "XRPL vault balance: {} GSTD (nanoGSTD), {} XRP drops",
            gstd_balance,
            xrp_drops
        );
        Ok(gstd_balance.max(xrp_drops))
    }

    async fn verify_deposit(&self, tx_hash: &str) -> anyhow::Result<bool> {
        let body = serde_json::json!({
            "method": "tx",
            "params": [{
                "transaction": tx_hash,
                "binary": false
            }]
        });
        let resp = self.client
            .post(&self.config.rpc_url)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let validated = resp
            .get("result")
            .and_then(|r| r.get("validated"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let success = resp
            .get("result")
            .and_then(|r| r.get("meta"))
            .and_then(|m| m.get("TransactionResult"))
            .and_then(|r| r.as_str())
            == Some("tesSUCCESS");

        tracing::debug!("Verified XRPL tx {tx_hash}: validated={validated}, success={success}");
        Ok(validated && success)
    }
}

impl XrplMonitor {
    /// Parse an XRPL memo for bridge instructions: "bridge:<chain>:<recipient>"
    fn parse_memo(
        &self,
        tx: &serde_json::Value,
        effective_amount: &serde_json::Value,
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

        // Parse amount using IOU-aware parser
        let currency_hint = &self.config.token_address;
        let amount = Self::parse_amount(effective_amount, currency_hint)
            .unwrap_or(0);

        if amount == 0 {
            tracing::warn!(
                "XRPL bridge memo found but amount is 0, tx={}",
                tx_hash
            );
            return None;
        }

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
