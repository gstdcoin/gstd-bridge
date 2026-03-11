pub mod keygen;

use serde::{Deserialize, Serialize};
use tracing;

/// Threshold Signature Scheme (TSS) manager
/// Manages the MPC protocol for generating shared signatures
/// without any single node holding the full private key.
///
/// Key concept:
/// - N validators each hold a "share" of the signing key
/// - T of N shares are needed to produce a valid signature (threshold)
/// - The private key is NEVER reconstructed on any single machine
pub struct ThresholdSigner {
    /// Threshold (minimum shares to sign)
    pub threshold: usize,
    /// Total number of participants
    pub total_parties: usize,
    /// This node's share index
    pub share_index: usize,
    /// This node's key share (private)
    pub key_share: Vec<u8>,
}

/// Result of an MPC signing round
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningResult {
    pub transfer_id: String,
    pub signature_share: Vec<u8>,
    pub share_index: usize,
    pub is_final: bool,
    /// Aggregated signature (only set when threshold reached)
    pub aggregated_signature: Option<Vec<u8>>,
}

impl ThresholdSigner {
    pub fn new(threshold: usize, total_parties: usize, share_index: usize) -> Self {
        // In production: use proper DKG (Distributed Key Generation)
        // Here we create a placeholder key share
        let key_share = keygen::generate_key_share(share_index);
        
        Self {
            threshold,
            total_parties,
            share_index,
            key_share,
        }
    }

    /// Generate this node's signature share for a transaction
    pub fn sign_share(&self, message: &[u8]) -> SigningResult {
        use sha2::{Digest, Sha256};
        
        // In production: use proper TSS (e.g., GG20 protocol)
        // Here we compute HMAC(key_share, message) as a simplified share
        let mut hasher = Sha256::new();
        hasher.update(&self.key_share);
        hasher.update(message);
        let share = hasher.finalize().to_vec();

        tracing::debug!(
            "🔐 Generated signature share (index={}, {} bytes)",
            self.share_index,
            share.len()
        );

        SigningResult {
            transfer_id: String::new(),
            signature_share: share,
            share_index: self.share_index,
            is_final: false,
            aggregated_signature: None,
        }
    }

    /// Attempt to aggregate signature shares into a final signature
    /// Returns Some(signature) if threshold is met
    pub fn try_aggregate(&self, shares: &[Vec<u8>]) -> Option<Vec<u8>> {
        if shares.len() < self.threshold {
            tracing::debug!(
                "🔑 Not enough shares: {}/{} (need {})",
                shares.len(),
                self.total_parties,
                self.threshold,
            );
            return None;
        }

        // In production: use Lagrange interpolation to combine shares
        // Here we XOR all shares as a simplified aggregation
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for share in shares {
            hasher.update(share);
        }
        let aggregated = hasher.finalize().to_vec();

        tracing::info!(
            "🔓 Signature aggregated from {} shares ({} bytes)",
            shares.len(),
            aggregated.len()
        );

        Some(aggregated)
    }
}
