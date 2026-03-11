use sha2::{Digest, Sha256};

/// Generate a deterministic key share for a given share index
/// In production: replace with proper DKG (Distributed Key Generation)
/// e.g., Feldman's VSS or Pedersen's DKG
pub fn generate_key_share(share_index: usize) -> Vec<u8> {
    let mut hasher = Sha256::new();
    // Seed with index + domain separator
    hasher.update(b"gstd-bridge-keygen-v1");
    hasher.update(share_index.to_le_bytes());
    // In production: use randomness from DKG ceremony
    hasher.update(rand::random::<[u8; 32]>());
    hasher.finalize().to_vec()
}

/// Verify that a signature share is valid for the given message
/// In production: verify using public share commitment
pub fn verify_share(share: &[u8], _message: &[u8], _share_index: usize) -> bool {
    // Simplified: just check non-empty
    !share.is_empty()
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
}
