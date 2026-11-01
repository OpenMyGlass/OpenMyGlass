//! Long-term device identity: an ed25519 keypair that is the trust anchor for the
//! persistent one-time pairing. The public key is exchanged in [`omg_protocol::Hello`]
//! and bonded once; thereafter each side proves possession of its private key with a
//! challenge/response, and rejects any peer whose key is not the bonded one.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::CoreError;

/// Domain-separation context signed together with a challenge nonce.
pub const AUTH_CONTEXT: &[u8] = b"openmyglass-companion-auth-v1";

/// A device's long-term signing identity.
#[derive(Clone)]
pub struct Identity {
    signing: SigningKey,
}

impl Identity {
    /// Generate a fresh random identity.
    pub fn generate() -> Identity {
        let mut secret = [0u8; 32];
        getrandom::getrandom(&mut secret).expect("OS RNG available");
        Identity {
            signing: SigningKey::from_bytes(&secret),
        }
    }

    /// Reconstruct from a base64-encoded 32-byte secret seed.
    pub fn from_secret_b64(s: &str) -> Result<Identity, CoreError> {
        let bytes = B64.decode(s.trim()).map_err(|_| CoreError::BadIdentity)?;
        let seed: [u8; 32] = bytes.try_into().map_err(|_| CoreError::BadIdentity)?;
        Ok(Identity {
            signing: SigningKey::from_bytes(&seed),
        })
    }

    /// Base64 of the 32-byte secret seed (store this; keep it private).
    pub fn secret_b64(&self) -> String {
        B64.encode(self.signing.to_bytes())
    }

    /// Base64 of the 32-byte public key (the identity shared with the peer).
    pub fn public_b64(&self) -> String {
        B64.encode(self.signing.verifying_key().to_bytes())
    }

    /// Sign `AUTH_CONTEXT || nonce` and return the base64 signature.
    pub fn sign_challenge(&self, nonce: &[u8]) -> String {
        let mut msg = Vec::with_capacity(AUTH_CONTEXT.len() + nonce.len());
        msg.extend_from_slice(AUTH_CONTEXT);
        msg.extend_from_slice(nonce);
        let sig: Signature = self.signing.sign(&msg);
        B64.encode(sig.to_bytes())
    }
}

/// Verify that `sig_b64` is `peer_pub_b64`'s signature over `AUTH_CONTEXT || nonce`.
pub fn verify_challenge(peer_pub_b64: &str, nonce: &[u8], sig_b64: &str) -> bool {
    let Ok(pub_bytes) = B64.decode(peer_pub_b64.trim()) else {
        return false;
    };
    let Ok(pub_arr): Result<[u8; 32], _> = pub_bytes.try_into() else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pub_arr) else {
        return false;
    };
    let Ok(sig_bytes) = B64.decode(sig_b64.trim()) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return false;
    };
    let mut msg = Vec::with_capacity(AUTH_CONTEXT.len() + nonce.len());
    msg.extend_from_slice(AUTH_CONTEXT);
    msg.extend_from_slice(nonce);
    vk.verify(&msg, &sig).is_ok()
}

/// A fresh random nonce, base64-encoded.
pub fn random_nonce_b64(len: usize) -> String {
    let mut buf = vec![0u8; len];
    getrandom::getrandom(&mut buf).expect("OS RNG available");
    B64.encode(buf)
}

/// Decode a base64 nonce back to bytes.
pub fn decode_nonce(b64: &str) -> Result<Vec<u8>, CoreError> {
    B64.decode(b64.trim())
        .map_err(|_| CoreError::Protocol("bad nonce base64".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = Identity::generate();
        let nonce = b"a-nonce-value";
        let sig = id.sign_challenge(nonce);
        assert!(verify_challenge(&id.public_b64(), nonce, &sig));
        // Wrong nonce fails.
        assert!(!verify_challenge(&id.public_b64(), b"other", &sig));
        // Wrong key fails.
        let other = Identity::generate();
        assert!(!verify_challenge(&other.public_b64(), nonce, &sig));
    }

    #[test]
    fn secret_roundtrip() {
        let id = Identity::generate();
        let restored = Identity::from_secret_b64(&id.secret_b64()).unwrap();
        assert_eq!(id.public_b64(), restored.public_b64());
    }
}
