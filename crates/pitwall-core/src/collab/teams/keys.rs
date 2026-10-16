//! Signing and checking team records (§4.1). Records are signed with the
//! device's instance key (tailnet-peers.md §5, `peers/identity.key`), which
//! the network side owns: it hands collaboration a [`RecordSigner`]. Keys
//! and signatures are written `ed25519:<base64url>` (no padding), as the
//! network side's instance key (`net::InstanceKey`) writes them, so a
//! [`RecordSigner`] can be backed by it directly.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::collab::team::DeviceKey;

const PREFIX: &str = "ed25519:";

/// This device's instance key, for signing records it writes (as admin, or
/// for its own entry).
pub trait RecordSigner: Send + Sync {
    /// The public key, `ed25519:…`.
    fn key(&self) -> DeviceKey;
    /// A signature over `msg`, `ed25519:…`.
    fn sign(&self, msg: &[u8]) -> String;
}

/// Checks a record's signature against a device key.
pub trait SignatureCheck: Send + Sync {
    fn verify(&self, key: &DeviceKey, msg: &[u8], sig: &str) -> bool;
}

/// Ed25519, the only scheme (§4.1).
#[derive(Debug, Default, Clone, Copy)]
pub struct Ed25519;

impl SignatureCheck for Ed25519 {
    fn verify(&self, key: &DeviceKey, msg: &[u8], sig: &str) -> bool {
        let Some(key) = decode_key(key) else { return false };
        let Some(sig) = sig.strip_prefix(PREFIX).and_then(|s| URL_SAFE_NO_PAD.decode(s).ok()) else { return false };
        let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) else { return false };
        key.verify(msg, &Signature::from_bytes(&sig)).is_ok()
    }
}

fn decode_key(key: &DeviceKey) -> Option<VerifyingKey> {
    let bytes = key.0.strip_prefix(PREFIX).and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())?;
    VerifyingKey::from_bytes(&<[u8; 32]>::try_from(bytes.as_slice()).ok()?).ok()
}

/// A signer over an Ed25519 secret key held in memory. The network side
/// builds one from the instance key file (or implements [`RecordSigner`]
/// itself); tests use fixed seeds.
pub struct Ed25519Signer {
    key: SigningKey,
}

impl Ed25519Signer {
    /// From the 32-byte secret seed.
    pub fn from_seed(seed: [u8; 32]) -> Ed25519Signer {
        Ed25519Signer { key: SigningKey::from_bytes(&seed) }
    }
}

impl RecordSigner for Ed25519Signer {
    fn key(&self) -> DeviceKey {
        DeviceKey(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(self.key.verifying_key().as_bytes())))
    }

    fn sign(&self, msg: &[u8]) -> String {
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(self.key.sign(msg).to_bytes()))
    }
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// A short fingerprint of a device key, for invite links (`k=`).
pub fn key_fingerprint(key: &DeviceKey) -> String {
    sha256_hex(key.0.as_bytes())[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_check_against_their_key_only() {
        let a = Ed25519Signer::from_seed([1; 32]);
        let b = Ed25519Signer::from_seed([2; 32]);
        let sig = a.sign(b"record");
        assert!(a.key().0.starts_with("ed25519:"));
        assert!(Ed25519.verify(&a.key(), b"record", &sig));
        assert!(!Ed25519.verify(&a.key(), b"recorD", &sig));
        assert!(!Ed25519.verify(&b.key(), b"record", &sig));
        assert!(!Ed25519.verify(&DeviceKey("ed25519:nonsense".into()), b"record", &sig));
        assert!(!Ed25519.verify(&a.key(), b"record", "ed25519:AAAA"));
        assert_eq!(key_fingerprint(&a.key()).len(), 16);
    }
}
