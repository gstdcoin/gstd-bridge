use libp2p::gossipsub::Event as GossipsubEvent;
use libp2p::PeerId;
use tracing;

use crate::bridge::BridgeMessage;

/// Process incoming gossipsub event
pub fn handle_gossipsub_event(
    event: GossipsubEvent,
    local_peer_id: &PeerId,
    msg_tx: &tokio::sync::mpsc::UnboundedSender<(PeerId, BridgeMessage)>,
) {
    match event {
        GossipsubEvent::Message {
            propagation_source,
            message,
            message_id,
            ..
        } => {
            // Skip messages from ourselves
            if let Some(source) = &message.source {
                if source == local_peer_id {
                    return;
                }
            }

            // Deserialize the bridge message
            match bincode::deserialize::<BridgeMessage>(&message.data) {
                Ok(bridge_msg) => {
                    tracing::debug!(
                        "📨 Gossip from {}: {:?} (id={message_id})",
                        propagation_source,
                        bridge_msg.kind()
                    );
                    let _ = msg_tx.send((propagation_source, bridge_msg));
                }
                Err(e) => {
                    tracing::warn!("Failed to deserialize gossip message: {e}");
                }
            }
        }
        GossipsubEvent::Subscribed { peer_id, topic } => {
            tracing::info!("Peer {peer_id} subscribed to {topic}");
        }
        GossipsubEvent::Unsubscribed { peer_id, topic } => {
            tracing::info!("Peer {peer_id} unsubscribed from {topic}");
        }
        _ => {}
    }
}
