use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Global bridge configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub node: NodeConfig,
    pub p2p: P2PConfig,
    pub chains: ChainsConfig,
    pub consensus: ConsensusConfig,
    pub rpc: RpcConfig,
    #[serde(default)]
    pub mpc: MpcConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique node identity (derived from keypair)
    pub name: String,
    /// Path to persistent peer identity keypair
    pub identity_path: PathBuf,
    /// Data directory for state persistence
    pub data_dir: PathBuf,
    /// Platform heartbeat URL
    pub heartbeat_url: String,
    /// Node operator wallet address (TON)
    pub operator_wallet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PConfig {
    /// Listen address for libp2p
    pub listen_addr: String,
    /// Bootstrap peers (multiaddrs)
    pub bootstrap_peers: Vec<String>,
    /// Gossipsub topic for bridge messages
    pub topic: String,
    /// Max peers
    pub max_peers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainsConfig {
    pub ton: ChainEndpoint,
    pub solana: ChainEndpoint,
    pub xrpl: ChainEndpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainEndpoint {
    /// RPC / HTTP endpoint
    pub rpc_url: String,
    /// WebSocket endpoint for live monitoring
    pub ws_url: Option<String>,
    /// Vault / custody address
    pub vault_address: String,
    /// Token contract address
    pub token_address: String,
    /// Polling interval in seconds (fallback for WS)
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    /// Quorum threshold (0.0 - 1.0), default 0.67
    pub quorum_threshold: f64,
    /// Timeout for voting round in seconds
    pub vote_timeout_secs: u64,
    /// Min validators for consensus to proceed
    pub min_validators: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcConfig {
    /// Local RPC listen address
    pub listen_addr: String,
    /// Enable CORS for frontend
    pub cors_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MpcConfig {
    /// This node's share index (1-based, assigned during validator onboarding)
    pub share_index: usize,
    /// Minimum shares required to produce a valid threshold signature
    pub threshold: usize,
    /// Total number of MPC parties (all active validators)
    pub total_parties: usize,
    /// Path where the key share is persisted — NEVER share or commit this file
    pub key_share_path: PathBuf,
}

impl Default for MpcConfig {
    fn default() -> Self {
        Self {
            share_index: 1,
            threshold: 2,
            total_parties: 3,
            key_share_path: PathBuf::from("./data/key_share.bin"),
        }
    }
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            node: NodeConfig {
                name: "gstd-bridge-node".to_string(),
                identity_path: PathBuf::from("./data/identity.key"),
                data_dir: PathBuf::from("./data"),
                heartbeat_url: "https://app.gstdtoken.com/api/v1/nodes/heartbeat".to_string(),
                operator_wallet: String::new(),
            },
            p2p: P2PConfig {
                listen_addr: "/ip4/0.0.0.0/tcp/4001".to_string(),
                bootstrap_peers: vec![],
                topic: "gstd-bridge/v1".to_string(),
                max_peers: 50,
            },
            chains: ChainsConfig {
                ton: ChainEndpoint {
                    rpc_url: "https://toncenter.com/api/v2".to_string(),
                    ws_url: None,
                    // IMPORTANT: vault_address must be the bridge-controlled custodian wallet (NOT the jetton master).
                    // Deploy a dedicated TON wallet for the bridge and set both addresses correctly in bridge.toml.
                    // token_address = GSTD jetton master: EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO
                    vault_address: "EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO".to_string(), // PLACEHOLDER — replace with bridge custodian wallet
                    token_address: "EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO".to_string(),
                    poll_interval_secs: 10,
                },
                solana: ChainEndpoint {
                    rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
                    ws_url: Some("wss://api.mainnet-beta.solana.com".to_string()),
                    // IMPORTANT: vault_address must be a bridge-controlled ATA, not the token mint address.
                    vault_address: "AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby".to_string(), // PLACEHOLDER — replace with bridge ATA
                    token_address: "AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby".to_string(),
                    poll_interval_secs: 5,
                },
                xrpl: ChainEndpoint {
                    rpc_url: "https://s1.ripple.com:51234".to_string(),
                    ws_url: Some("wss://s1.ripple.com".to_string()),
                    // IMPORTANT: vault_address must be a bridge-controlled issuer account, not the token issuer.
                    vault_address: "ryHSvxUqpcTjoESHbCkMJoqzenjFgPQSf".to_string(), // PLACEHOLDER — replace with bridge issuer account
                    token_address: "ryHSvxUqpcTjoESHbCkMJoqzenjFgPQSf".to_string(),
                    poll_interval_secs: 5,
                },
            },
            consensus: ConsensusConfig {
                quorum_threshold: 0.67,
                vote_timeout_secs: 30,
                min_validators: 3,
            },
            rpc: RpcConfig {
                listen_addr: "127.0.0.1:9090".to_string(),
                cors_enabled: true,
            },
            mpc: MpcConfig::default(),
        }
    }
}

impl BridgeConfig {
    /// Load from TOML file. Requires the file to exist and every chain's
    /// vault_address to be a real, distinct custodian wallet -- never falls
    /// back to a default config, since the only defaults available for
    /// vault_address are either empty or equal to token_address, both of
    /// which would misroute funds to the token mint/issuer instead of a
    /// bridge-controlled wallet.
    pub fn load(path: &str) -> anyhow::Result<Self> {
        if !std::path::Path::new(path).exists() {
            anyhow::bail!(
                "Config file not found at {path}. Copy the bridge.toml template from this repo \
                 and set vault_address for each chain to a real bridge-controlled custodian \
                 wallet before starting the bridge."
            );
        }
        let content = std::fs::read_to_string(path)?;
        let config: BridgeConfig = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Reject a config whose vault_address is missing or equal to the
    /// token's own mint/issuer address on any chain -- either would route
    /// bridged funds to the token contract itself instead of a real
    /// bridge-controlled custodian wallet.
    fn validate(&self) -> anyhow::Result<()> {
        for (chain_name, endpoint) in [
            ("ton", &self.chains.ton),
            ("solana", &self.chains.solana),
            ("xrpl", &self.chains.xrpl),
        ] {
            if endpoint.vault_address.is_empty() {
                anyhow::bail!(
                    "chains.{chain_name}.vault_address is not set. It must be a real \
                     bridge-controlled custodian wallet, distinct from token_address."
                );
            }
            if endpoint.vault_address == endpoint.token_address {
                anyhow::bail!(
                    "chains.{chain_name}.vault_address is set to the same address as \
                     token_address ({}) -- this would route bridged funds to the token's own \
                     mint/issuer instead of a bridge-controlled custodian wallet. Set \
                     vault_address to a real, distinct custodian wallet.",
                    endpoint.token_address
                );
            }
        }
        Ok(())
    }

    /// Save current config to TOML
    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
