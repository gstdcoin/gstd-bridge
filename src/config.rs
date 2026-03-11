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

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            node: NodeConfig {
                name: "gstd-bridge-node".to_string(),
                identity_path: PathBuf::from("./data/identity.key"),
                data_dir: PathBuf::from("./data"),
                heartbeat_url: "https://api.gstdtoken.com/api/v1/nodes/heartbeat".to_string(),
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
                    vault_address: "EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO".to_string(),
                    token_address: "EQDv6cYW9nNiKjN3Nwl8D6ABjUiH1gYfWVGZhfP7-9tZskTO".to_string(),
                    poll_interval_secs: 10,
                },
                solana: ChainEndpoint {
                    rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
                    ws_url: Some("wss://api.mainnet-beta.solana.com".to_string()),
                    vault_address: "AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby".to_string(),
                    token_address: "AzN7uPhQZgThxsRvhNGHPUPRjdEjScTbqQdf5gt6Fqby".to_string(),
                    poll_interval_secs: 5,
                },
                xrpl: ChainEndpoint {
                    rpc_url: "https://s1.ripple.com:51234".to_string(),
                    ws_url: Some("wss://s1.ripple.com".to_string()),
                    vault_address: "ryHSvxUqpcTjoESHbCkMJoqzenjFgPQSf".to_string(),
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
        }
    }
}

impl BridgeConfig {
    /// Load from TOML file, falling back to defaults
    pub fn load(path: &str) -> anyhow::Result<Self> {
        if std::path::Path::new(path).exists() {
            let content = std::fs::read_to_string(path)?;
            let config: BridgeConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            tracing::warn!("Config file not found at {path}, using defaults");
            Ok(Self::default())
        }
    }

    /// Save current config to TOML
    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
