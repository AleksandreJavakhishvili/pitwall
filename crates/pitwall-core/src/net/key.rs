//! The instance key (tailnet-peers.md §5, collaboration teams.md §4.1): one
//! Ed25519 key pair per Pitwall data folder, in `peers/identity.key`,
//! readable by its owner only. It identifies this installation and signs
//! team records; requests are never signed (callers are proven by their
//! network, protocol.md §8.2). Network-neutral: every backend's device is
//! this key plus its address there.
//!
//! Text forms: a public key is `ed25519:<base64url>` (32 bytes, no
//! padding), a signature `ed25519:<base64url>` (64 bytes); both are what
//! `collab::team::DeviceKey` and a record's `sig` hold.

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::{PwError, Result};
use crate::platform;

/// The prefix of a key's or signature's text form.
pub const KEY_PREFIX: &str = "ed25519:";

/// This installation's key pair. `Debug` never shows the secret.
#[derive(Clone)]
pub struct InstanceKey {
    signing: SigningKey,
}

/// `peers/identity.key`: the seed, and the public key to catch a damaged
/// file.
#[derive(Serialize, Deserialize)]
struct KeyFile {
    v: u32,
    alg: String,
    secret: String,
    public: String,
}

impl InstanceKey {
    /// Where the key lives, under Pitwall's data folder (tailnet-peers.md §5).
    pub const FILE: &'static str = "peers/identity.key";

    /// The key file under a data folder (`Paths::root`).
    pub fn path(root: &Path) -> PathBuf {
        root.join(Self::FILE)
    }

    /// The key in `root`'s `peers/identity.key`, made on first use (0600).
    /// A damaged file is an error, never silently replaced: teams know this
    /// device by its key.
    pub fn load_or_create(root: &Path) -> Result<InstanceKey> {
        let path = Self::path(root);
        match std::fs::read(&path) {
            Ok(bytes) => {
                platform::make_private(&path).map_err(|e| PwError::from(e).context("instance key"))?;
                Self::from_file(&bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = Self::generate()?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| PwError::from(e).context("instance key"))?;
                }
                platform::write_private(&path, &key.to_file())
                    .map_err(|e| PwError::from(e).context("instance key"))?;
                Ok(key)
            }
            Err(e) => Err(PwError::from(e).context("instance key")),
        }
    }

    /// A new random key (the OS's random source).
    pub fn generate() -> Result<InstanceKey> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| PwError::other(format!("no randomness for a key: {e}")))?;
        Ok(InstanceKey { signing: SigningKey::from_bytes(&seed) })
    }

    /// A key from its 32-byte seed (tests and key rotation tooling).
    pub fn from_seed(seed: [u8; 32]) -> InstanceKey {
        InstanceKey { signing: SigningKey::from_bytes(&seed) }
    }

    /// `ed25519:<base64url>`, as team records list devices.
    pub fn public_key(&self) -> String {
        format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(self.signing.verifying_key().as_bytes()))
    }

    /// A signature over `message`, `ed25519:<base64url>`.
    pub fn sign(&self, message: &[u8]) -> String {
        format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(self.signing.sign(message).to_bytes()))
    }

    /// Whether `signature` (text form) is `public_key`'s over `message`.
    /// Malformed keys or signatures are simply not valid.
    pub fn verify(public_key: &str, message: &[u8], signature: &str) -> bool {
        let decode = |s: &str| s.strip_prefix(KEY_PREFIX).and_then(|b| URL_SAFE_NO_PAD.decode(b).ok());
        let (Some(key), Some(sig)) = (decode(public_key), decode(signature)) else { return false };
        let (Ok(key), Ok(sig)) = (<[u8; 32]>::try_from(key), <[u8; 64]>::try_from(sig)) else { return false };
        let Ok(key) = VerifyingKey::from_bytes(&key) else { return false };
        key.verify_strict(message, &Signature::from_bytes(&sig)).is_ok()
    }

    fn to_file(&self) -> Vec<u8> {
        let file = KeyFile {
            v: 1,
            alg: "ed25519".into(),
            secret: URL_SAFE_NO_PAD.encode(self.signing.to_bytes()),
            public: self.public_key(),
        };
        let mut bytes = serde_json::to_vec_pretty(&file).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    fn from_file(bytes: &[u8]) -> Result<InstanceKey> {
        let damaged = || PwError::other(format!("{} is damaged; move it away to make a new key", Self::FILE));
        let file: KeyFile = serde_json::from_slice(bytes).map_err(|_| damaged())?;
        if file.v != 1 || file.alg != "ed25519" {
            return Err(damaged());
        }
        let seed = URL_SAFE_NO_PAD.decode(file.secret.trim()).map_err(|_| damaged())?;
        let seed = <[u8; 32]>::try_from(seed).map_err(|_| damaged())?;
        let key = InstanceKey::from_seed(seed);
        if key.public_key() != file.public {
            return Err(damaged());
        }
        Ok(key)
    }
}

impl std::fmt::Debug for InstanceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InstanceKey({})", self.public_key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn made_once_private_and_stable() {
        let dir = std::env::temp_dir().join(format!("pw-key-{}", uuid::Uuid::new_v4()));
        let key = InstanceKey::load_or_create(&dir).unwrap();
        let path = InstanceKey::path(&dir);
        assert!(path.ends_with("peers/identity.key"));
        assert!(platform::is_private(&path));
        let again = InstanceKey::load_or_create(&dir).unwrap();
        assert_eq!(key.public_key(), again.public_key());
        assert!(key.public_key().starts_with("ed25519:"));
        assert!(!format!("{key:?}").contains(&URL_SAFE_NO_PAD.encode(key.signing.to_bytes())));

        std::fs::write(&path, b"{\"v\":1}").unwrap();
        assert!(InstanceKey::load_or_create(&dir).unwrap_err().contains("damaged"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signs_and_verifies() {
        let key = InstanceKey::from_seed([7; 32]);
        let other = InstanceKey::from_seed([8; 32]);
        let sig = key.sign(b"record");
        assert!(InstanceKey::verify(&key.public_key(), b"record", &sig));
        assert!(!InstanceKey::verify(&key.public_key(), b"recorD", &sig));
        assert!(!InstanceKey::verify(&other.public_key(), b"record", &sig));
        assert!(!InstanceKey::verify("ed25519:nope", b"record", &sig));
        assert!(!InstanceKey::verify(&key.public_key(), b"record", "rsa:abc"));
        assert_ne!(InstanceKey::generate().unwrap().public_key(), InstanceKey::generate().unwrap().public_key());
    }
}
