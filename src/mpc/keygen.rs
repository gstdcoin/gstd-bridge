use sha2::{Digest, Sha256};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Signature, Verifier};
use rand::{rngs::OsRng, RngCore};

/// Per-validator Ed25519 key generation.
///
/// NOT Shamir Secret Sharing: each "share" below is an independent Ed25519
/// keypair derived from `(domain, share_index, local randomness)`, not a
/// point on a shared polynomial. There is no single underlying secret split
/// across shares, and no reconstruction/interpolation step — nor should
/// there be, since no party should ever hold the full secret in a real
/// threshold scheme. What this module actually gives you is real, verifiable
/// Ed25519 signing per validator (see `sign_with_share`/`verify_share`); a
/// genuine threshold scheme (FROST or similar DKG) is not implemented here.

/// A key share for one participant — contains the share value and the
/// corresponding public verification key
#[derive(Clone)]
pub struct KeyShare {
    /// Share index (1-based)
    pub index: usize,
    /// The share bytes (32 bytes, Ed25519 scalar)  
    pub share_bytes: Vec<u8>,
    /// Public verification key for this share
    pub verification_key: Vec<u8>,
}

/// Generate a key share using deterministic derivation from a master seed.
/// In production: use a proper DKG ceremony (Feldman's VSS / Pedersen's DKG)
/// where no single party ever knows the full secret.
///
/// Current implementation: derives shares from HKDF-like construction.
/// This is suitable for single-operator deployments where the bridge
/// operator controls all validator nodes.
pub fn generate_key_share(share_index: usize) -> Vec<u8> {
    // Use HKDF-like construction: SHA256(domain || index || entropy)
    let mut hasher = Sha256::new();
    hasher.update(b"gstd-bridge-keygen-v2-ed25519");
    hasher.update(share_index.to_le_bytes());
    // In production: replace with DKG ceremony output
    // For single-operator mode, use deterministic randomness
    hasher.update(OsRng.next_u64().to_le_bytes());
    
    let hash = hasher.finalize();
    
    // Clamp to valid Ed25519 scalar (RFC 8032)
    let mut scalar = hash.to_vec();
    scalar[0] &= 248;
    scalar[31] &= 127;
    scalar[31] |= 64;
    
    scalar
}

/// Generate a key share with its Ed25519 verification key
pub fn generate_key_share_with_vk(share_index: usize) -> KeyShare {
    let share_bytes = generate_key_share(share_index);
    
    // Derive Ed25519 keypair from share
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&share_bytes[..32]);
    let signing_key = SigningKey::from_bytes(&key_bytes);
    let vk = signing_key.verifying_key();
    
    KeyShare {
        index: share_index,
        share_bytes,
        verification_key: vk.to_bytes().to_vec(),
    }
}

/// Sign a message using a key share
/// Returns (signature_bytes, verifying_key_bytes) 
pub fn sign_with_share(share: &[u8], message: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&share[..32]);
    let signing_key = SigningKey::from_bytes(&key_bytes);
    let signature = signing_key.sign(message);
    let vk = signing_key.verifying_key();
    
    (signature.to_bytes().to_vec(), vk.to_bytes().to_vec())
}

/// Verify that a signature share is valid for the given message
/// Uses Ed25519 signature verification (not just empty check)
pub fn verify_share(signature_bytes: &[u8], message: &[u8], verification_key: &[u8]) -> bool {
    if signature_bytes.len() != 64 || verification_key.len() != 32 {
        return false;
    }
    
    let sig_bytes: [u8; 64] = match signature_bytes.try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    let vk_bytes: [u8; 32] = match verification_key.try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    
    let signature = match Signature::from_bytes(&sig_bytes) {
        s => s,
    };
    let vk = match VerifyingKey::from_bytes(&vk_bytes) {
        Ok(k) => k,
        Err(_) => return false,
    };
    
    vk.verify(message, &signature).is_ok()
}

/// Load a persisted key share from `path`, or generate a new one and save it.
///
/// Once saved, the same bytes are returned on every subsequent call as long as
/// the file is present and valid (exactly 32 bytes).  Delete the file to force
/// re-generation — but note that re-generation invalidates any prior MPC round.
pub fn load_or_generate_persistent_share(
    share_index: usize,
    path: &std::path::Path,
) -> anyhow::Result<Vec<u8>> {
    if path.exists() {
        let bytes = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("Failed to read key share {}: {e}", path.display()))?;
        if bytes.len() == 32 {
            tracing::info!("🔑 Loaded key share {} from {}", share_index, path.display());
            return Ok(bytes);
        }
        tracing::warn!("⚠️ Key share file is corrupt ({}B), regenerating: {}",
            bytes.len(), path.display());
    }

    let share = generate_key_share(share_index);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &share)
        .map_err(|e| anyhow::anyhow!("Failed to save key share {}: {e}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    tracing::info!("🔑 Generated and saved new key share {} → {}", share_index, path.display());
    Ok(share)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keygen_produces_32_bytes() {
        let share = generate_key_share(0);
        assert_eq!(share.len(), 32);
    }

    #[test]
    fn test_different_indices_different_keys() {
        let share_0 = generate_key_share(0);
        let share_1 = generate_key_share(1);
        assert_ne!(share_0, share_1);
    }
    
    #[test]
    fn test_sign_and_verify_share() {
        let key_share = generate_key_share_with_vk(1);
        let message = b"test bridge transfer";
        let (sig, vk) = sign_with_share(&key_share.share_bytes, message);
        assert!(verify_share(&sig, message, &vk));
    }
    
    #[test]
    fn test_invalid_signature_fails() {
        let key_share = generate_key_share_with_vk(1);
        let message = b"test bridge transfer";
        let (sig, vk) = sign_with_share(&key_share.share_bytes, message);
        // Verify with wrong message should fail
        assert!(!verify_share(&sig, b"wrong message", &vk));
    }
}
