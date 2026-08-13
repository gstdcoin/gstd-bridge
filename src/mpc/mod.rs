pub mod keygen;

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use tracing;

/// Per-validator signer, NOT a real threshold signature scheme.
///
/// What this actually does:
/// - Each validator holds its own independently-generated Ed25519 keypair
///   (see `keygen.rs` — there is no shared secret polynomial behind these)
/// - Each validator can produce a genuine, independently-verifiable Ed25519
///   signature over a message (`sign_share`/`verify_share`)
/// - Once `threshold` validators have each signed, `try_collect_quorum`
///   returns that set of independent signatures — it does NOT combine them
///   into a single aggregated/threshold signature. Ed25519 signatures from
///   unrelated keys cannot be linearly combined the way a real threshold
///   scheme (FROST, GG20) can; pretending otherwise was the previous
///   (removed) behavior of this module and produced bytes that were not a
///   valid signature on any target chain.
///
/// Upgrade path: replace with a real FROST/GG20 DKG + threshold-signing
/// protocol before this is used to move real funds.
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
}

impl ThresholdSigner {
    pub fn new(threshold: usize, total_parties: usize, share_index: usize) -> Self {
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

    /// Load a persisted key share from `path` (or generate and save a new one),
    /// then construct the signer.  Use this in production so that the key share
    /// survives node restarts and remains consistent across MPC rounds.
    pub fn load_or_create(
        threshold: usize,
        total_parties: usize,
        share_index: usize,
        key_share_path: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let share_bytes =
            keygen::load_or_generate_persistent_share(share_index, key_share_path)?;

        let mut kb = [0u8; 32];
        kb.copy_from_slice(&share_bytes[..32]);
        let vk = SigningKey::from_bytes(&kb).verifying_key().to_bytes().to_vec();

        tracing::info!(
            "🔐 ThresholdSigner ready: party {}/{}, threshold={}, vk={}",
            share_index, total_parties, threshold,
            hex::encode(&vk[..8])
        );

        Ok(Self {
            threshold,
            total_parties,
            share_index,
            key_share: share_bytes,
            verification_key: vk,
        })
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
        }
    }

    /// Verify a signature share from another party
    pub fn verify_share(&self, result: &SigningResult, message: &[u8]) -> bool {
        keygen::verify_share(&result.signature_share, message, &result.verification_key)
    }

    /// Once `threshold` signature shares have been collected, return them.
    ///
    /// This does NOT aggregate the shares into a single combined signature —
    /// see the module doc comment for why. It is simply a quorum check that
    /// hands back the raw shares for the caller to pass on (e.g. to
    /// `ChainMonitor::execute_withdrawal`, which already accepts a
    /// `Vec<Vec<u8>>` rather than one signature). Note the shares passed in
    /// here are not verified against a registered per-validator key at this
    /// call site — see `ConsensusEngine::record_vote` for what is and isn't
    /// authenticated before a share reaches this point.
    pub fn try_collect_quorum(&self, shares: &[Vec<u8>]) -> Option<Vec<Vec<u8>>> {
        if shares.len() < self.threshold {
            tracing::debug!(
                "🔑 Not enough shares: {}/{} (need {})",
                shares.len(),
                self.total_parties,
                self.threshold,
            );
            return None;
        }

        tracing::info!(
            "🔓 Quorum reached: {}/{} signature shares collected (not aggregated — see mpc/mod.rs doc comment)",
            shares.len(),
            self.total_parties,
        );

        Some(shares.to_vec())
    }
}

