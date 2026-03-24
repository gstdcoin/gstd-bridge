pub mod keygen;

use serde::{Deserialize, Serialize};
use tracing;

/// Threshold Signature Scheme (TSS) manager
/// Manages the MPC protocol for generating shared signatures
/// without any single node holding the full private key.
///
/// Security model:
/// - N validators each hold a key share (Ed25519 signing key derived from Shamir's share)
/// - T of N shares are needed to produce a valid aggregated signature (threshold)
/// - Each share independently produces a verifiable Ed25519 signature
/// - Aggregation combines T verified signatures into a final approval
///
/// Upgrade path: Replace with GG20/FROST protocol for full MPC without
/// any trusted dealer. Current implementation uses dealer-based Shamir's
/// secret sharing which is suitable for single-operator deployments.
pub struct ThresholdSigner {
    /// Threshold (minimum shares to sign)
    pub threshold: usize,
    /// Total number of participants
    pub total_parties: usize,
    /// This node's share index (1-based)
    pub share_index: usize,
    /// This node's key share (Ed25519 scalar, 32 bytes)
    pub key_share: Vec<u8>,
    /// This node's verification key (Ed25519 public key, 32 bytes)
    pub verification_key: Vec<u8>,
}

/// Result of an MPC signing round
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningResult {
    pub transfer_id: String,
    /// Ed25519 signature (64 bytes)
    pub signature_share: Vec<u8>,
    /// Ed25519 verification key for this share (32 bytes)
    pub verification_key: Vec<u8>,
    pub share_index: usize,
    pub is_final: bool,
    /// Aggregated signature (only set when threshold reached)
    pub aggregated_signature: Option<Vec<u8>>,
}

impl ThresholdSigner {
    pub fn new(threshold: usize, total_parties: usize, share_index: usize) -> Self {
        // Generate Ed25519 key share with verification key
        let key_share_data = keygen::generate_key_share_with_vk(share_index);
        
        tracing::info!(
            "🔐 ThresholdSigner initialized: party {}/{}, threshold={}, vk={}",
            share_index, total_parties, threshold,
            hex::encode(&key_share_data.verification_key[..8])
        );
        
        Self {
            threshold,
            total_parties,
            share_index,
            key_share: key_share_data.share_bytes,
            verification_key: key_share_data.verification_key,
        }
    }

    /// Generate this node's signature share for a transaction
    /// Produces a real Ed25519 signature that can be independently verified
    pub fn sign_share(&self, message: &[u8]) -> SigningResult {
        let (signature, vk) = keygen::sign_with_share(&self.key_share, message);

        tracing::debug!(
            "🔐 Generated Ed25519 signature share (index={}, sig={}, vk={})",
            self.share_index,
            hex::encode(&signature[..8]),
            hex::encode(&vk[..8]),
        );

        SigningResult {
            transfer_id: String::new(),
            signature_share: signature,
            verification_key: vk,
            share_index: self.share_index,
            is_final: false,
            aggregated_signature: None,
        }
    }

    /// Verify a signature share from another party
    pub fn verify_share(&self, result: &SigningResult, message: &[u8]) -> bool {
        keygen::verify_share(&result.signature_share, message, &result.verification_key)
    }

    /// Attempt to aggregate raw signature shares into a final signature
    /// Backward-compatible API accepting Vec<Vec<u8>> from consensus engine
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

        // Convert raw bytes to indexed shares for Lagrange aggregation
        let indexed_shares: Vec<(usize, Vec<u8>)> = shares
            .iter()
            .enumerate()
            .map(|(i, s)| (i + 1, s.clone()))
            .collect();

        let aggregated = keygen::aggregate_shares(&indexed_shares, self.threshold)?;

        tracing::info!(
            "🔓 Threshold signature aggregated: {} shares → {} bytes",
            shares.len(),
            aggregated.len()
        );

        Some(aggregated)
    }

    /// Aggregate verified SigningResult shares (full MPC version)
    /// Verifies each Ed25519 signature before aggregation
    pub fn try_aggregate_verified(&self, shares: &[SigningResult], message: &[u8]) -> Option<Vec<u8>> {
        if shares.len() < self.threshold {
            return None;
        }

        let mut verified_shares: Vec<(usize, Vec<u8>)> = Vec::new();
        for share in shares {
            if !keygen::verify_share(&share.signature_share, message, &share.verification_key) {
                tracing::warn!("⚠️ Invalid signature from party {} — rejected", share.share_index);
                continue;
            }
            verified_shares.push((share.share_index, share.signature_share.clone()));
        }

        if verified_shares.len() < self.threshold {
            tracing::warn!(
                "⚠️ Only {}/{} shares verified (need {})",
                verified_shares.len(), shares.len(), self.threshold,
            );
            return None;
        }

        let aggregated = keygen::aggregate_shares(&verified_shares, self.threshold)?;

        tracing::info!(
            "🔓 Verified threshold signature: {} shares → {} bytes (Ed25519 + Lagrange)",
            verified_shares.len(),
            aggregated.len()
        );

        Some(aggregated)
    }
}

