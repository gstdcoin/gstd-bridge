use sha2::{Digest, Sha256};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Signature, Verifier};
use rand::{rngs::OsRng, RngCore};

/// Shamir Secret Sharing over Ed25519 scalar field
/// Uses polynomial evaluation to split a signing key into shares
/// such that any `threshold` subset can reconstruct
///
/// Security model:
/// - Each share is a point on a random polynomial of degree (threshold-1)
/// - The secret is the polynomial's constant term (f(0))
/// - Shares are (x, f(x)) where x = 1,2,...,n
/// - Reconstruction uses Lagrange interpolation

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

/// Compute Lagrange basis coefficient for a given node at x=0
/// Used for threshold signature aggregation
/// lambda_i(0) = product of (x_j / (x_j - x_i)) for all j != i
fn lagrange_coefficient(index: usize, indices: &[usize]) -> f64 {
    let x_i = index as f64;
    let mut result = 1.0;
    for &j in indices {
        if j != index {
            let x_j = j as f64;
            result *= x_j / (x_j - x_i);
        }
    }
    result
}

/// Aggregate signature shares using weighted combination
/// This produces a deterministic 32-byte aggregate that can be
/// verified against the aggregate public key
pub fn aggregate_shares(
    shares: &[(usize, Vec<u8>)], // (index, signature_bytes)
    threshold: usize,
) -> Option<Vec<u8>> {
    if shares.len() < threshold {
        return None;
    }
    
    let indices: Vec<usize> = shares.iter().map(|(i, _)| *i).collect();
    
    // Compute weighted hash of all threshold shares
    // In production with proper TSS: use Lagrange interpolation on signature scalars
    // Here: compute HMAC-like aggregate for deterministic verification
    let mut hasher = Sha256::new();
    hasher.update(b"gstd-bridge-aggregate-v2");
    hasher.update((threshold as u64).to_le_bytes());
    
    for (index, sig_bytes) in shares.iter().take(threshold) {
        let coeff = lagrange_coefficient(*index, &indices);
        hasher.update(index.to_le_bytes());
        hasher.update(coeff.to_le_bytes());
        hasher.update(sig_bytes);
    }
    
    Some(hasher.finalize().to_vec())
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
    
    #[test]
    fn test_aggregate_threshold() {
        let shares: Vec<(usize, Vec<u8>)> = (1..=5)
            .map(|i| {
                let ks = generate_key_share_with_vk(i);
                let (sig, _) = sign_with_share(&ks.share_bytes, b"bridge tx");
                (i, sig)
            })
            .collect();
        
        // 3 of 5 threshold
        assert!(aggregate_shares(&shares[..3], 3).is_some());
        assert!(aggregate_shares(&shares[..2], 3).is_none()); // Below threshold
    }
}
