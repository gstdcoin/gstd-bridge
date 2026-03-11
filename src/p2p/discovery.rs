use libp2p::kad::{self, Event as KademliaEvent};
use tracing;

/// Process Kademlia DHT events for peer discovery
pub fn handle_kademlia_event(event: KademliaEvent) {
    match event {
        KademliaEvent::RoutingUpdated {
            peer, addresses, ..
        } => {
            tracing::debug!(
                "🔍 DHT routing updated: peer={peer}, addrs={}",
                addresses.len()
            );
        }
        KademliaEvent::OutboundQueryProgressed { result, .. } => {
            match result {
                kad::QueryResult::GetClosestPeers(Ok(ok)) => {
                    tracing::debug!("DHT closest peers: found {} peers", ok.peers.len());
                }
                kad::QueryResult::Bootstrap(Ok(ok)) => {
                    tracing::info!(
                        "🔄 DHT bootstrap step complete ({} remaining)",
                        ok.num_remaining
                    );
                }
                _ => {}
            }
        }
        _ => {}
    }
}
