pub mod gossip;
pub mod discovery;

use anyhow::Result;
use libp2p::{
    gossipsub, identify, kad, noise, tcp, yamux,
    swarm::NetworkBehaviour, Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::bridge::BridgeMessage;
use crate::config::P2PConfig;

/// Composed libp2p behaviour: Gossipsub + Kademlia + Identify
#[derive(NetworkBehaviour)]
pub struct BridgeBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
}

/// Wrapper around libp2p Swarm for the bridge
pub struct P2PNode {
    pub peer_id: PeerId,
    pub swarm: Swarm<BridgeBehaviour>,
    pub topic: gossipsub::IdentTopic,
    /// Channel to send received messages to the bridge engine
    pub msg_tx: mpsc::UnboundedSender<(PeerId, BridgeMessage)>,
}

impl P2PNode {
    /// Create and configure the P2P node
    pub async fn new(
        config: &P2PConfig,
        msg_tx: mpsc::UnboundedSender<(PeerId, BridgeMessage)>,
    ) -> Result<Self> {
        // Build the swarm
        let swarm = SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                // Gossipsub with message deduplication
                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(5))
                    .validation_mode(gossipsub::ValidationMode::Strict)
                    .max_transmit_size(65536)
                    .build()
                    .expect("gossipsub config");

                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )
                .expect("gossipsub behaviour");

                // Kademlia DHT for peer discovery
                let store = kad::store::MemoryStore::new(key.public().to_peer_id());
                let mut kademlia = kad::Behaviour::new(
                    key.public().to_peer_id(),
                    store,
                );
                kademlia.set_mode(Some(kad::Mode::Server));

                // Identify protocol for exchanging peer info
                let identify = identify::Behaviour::new(
                    identify::Config::new(
                        "/gstd-bridge/1.0.0".to_string(),
                        key.public(),
                    )
                    .with_push_listen_addr_updates(true),
                );

                Ok(BridgeBehaviour {
                    gossipsub,
                    kademlia,
                    identify,
                })
            })?
            .with_swarm_config(|c| {
                c.with_idle_connection_timeout(Duration::from_secs(120))
            })
            .build();

        let peer_id = *swarm.local_peer_id();
        let topic = gossipsub::IdentTopic::new(&config.topic);

        let mut node = Self {
            peer_id,
            swarm,
            topic,
            msg_tx,
        };

        // Subscribe to bridge topic
        node.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&node.topic)?;

        // Listen on configured address
        let listen_addr: Multiaddr = config.listen_addr.parse()?;
        node.swarm.listen_on(listen_addr)?;

        // Connect to bootstrap peers
        for peer_addr in &config.bootstrap_peers {
            if let Ok(addr) = peer_addr.parse::<Multiaddr>() {
                node.swarm.dial(addr.clone())?;
                tracing::info!("Dialing bootstrap peer: {}", peer_addr);
            }
        }

        tracing::info!("🌐 P2P Node started: {peer_id}");
        Ok(node)
    }

    /// Publish a message to the gossipsub topic
    pub fn broadcast(&mut self, message: &BridgeMessage) -> Result<()> {
        let data = bincode::serialize(message)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic.clone(), data)?;
        Ok(())
    }

    /// Get current number of connected peers
    pub fn peer_count(&self) -> usize {
        self.swarm.connected_peers().count()
    }

    /// Get list of connected peer IDs
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.swarm.connected_peers().cloned().collect()
    }
}
