//! The instance key (tailnet-peers.md §5): one Ed25519 key pair per Pitwall
//! data folder, in `peers/identity.key`, readable by its owner only. It
//! binds a peer connection to that Pitwall installation, not merely to
//! "some process on that node". Ported from the parked collaboration
//! branch's `net::InstanceKey`, on `ring` (already in the tree) instead of
//! `ed25519-dalek`.
//!
//! Text forms: a public key is `ed25519:<base64url>` (32 bytes, no
//! padding), a signature `ed25519:<base64url>` (64 bytes).

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair as _, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};

use pitwall_proto::{code, ErrorBody};

use crate::platform;

/// The prefix of a key's or signature's text form.
pub const KEY_PREFIX: &str = "ed25519:";

/// This installation's key pair. `Debug` never shows the secret.
pub struct InstanceKey {
    pair: Ed25519KeyPair,
    pkcs8: Vec<u8>,
}

/// `peers/identity.key`: the PKCS#8 document, and the public key to catch
/// a damaged file.
#[derive(Serialize, Deserialize)]
struct KeyFile {
    v: u32,
    alg: String,
    secret: String,
    public: String,
}

fn err(message: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::OTHER, message)
}

impl InstanceKey {
    /// Where the key lives, under Pitwall's data folder.
    pub const FILE: &'static str = "peers/identity.key";

    pub fn path(root: &Path) -> PathBuf {
        root.join(Self::FILE)
    }

    /// The key in `root`'s `peers/identity.key`, made on first use (0600).
    /// A damaged file is an error, never silently replaced: paired machines
    /// know this Pitwall by its key.
    pub fn load_or_create(root: &Path) -> Result<InstanceKey, ErrorBody> {
        let path = Self::path(root);
        match std::fs::read(&path) {
            Ok(bytes) => {
                platform::make_private(&path).map_err(|e| err(format!("instance key: {e}")))?;
                Self::from_file(&bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = Self::generate()?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| err(format!("instance key: {e}")))?;
                }
                platform::write_private(&path, &key.to_file()).map_err(|e| err(format!("instance key: {e}")))?;
                Ok(key)
            }
            Err(e) => Err(err(format!("instance key: {e}"))),
        }
    }

    /// A new random key (the OS's random source).
    pub fn generate() -> Result<InstanceKey, ErrorBody> {
        let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| err("no randomness for a key"))?;
        Self::from_pkcs8(doc.as_ref().to_vec()).ok_or_else(|| err("can't make a key"))
    }

    fn from_pkcs8(pkcs8: Vec<u8>) -> Option<InstanceKey> {
        let pair = Ed25519KeyPair::from_pkcs8(&pkcs8).ok()?;
        Some(InstanceKey { pair, pkcs8 })
    }

    /// `ed25519:<base64url>`
    pub fn public_key(&self) -> String {
        format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(self.pair.public_key().as_ref()))
    }

    /// A signature over `message`, `ed25519:<base64url>`.
    pub fn sign(&self, message: &[u8]) -> String {
        format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(self.pair.sign(message).as_ref()))
    }

    /// Whether `signature` (text form) is `public_key`'s over `message`.
    /// Malformed keys or signatures are simply not valid.
    pub fn verify(public_key: &str, message: &[u8], signature: &str) -> bool {
        let decode = |s: &str| s.strip_prefix(KEY_PREFIX).and_then(|b| URL_SAFE_NO_PAD.decode(b).ok());
        let (Some(key), Some(sig)) = (decode(public_key), decode(signature)) else { return false };
        key.len() == 32 && sig.len() == 64 && UnparsedPublicKey::new(&ED25519, key).verify(message, &sig).is_ok()
    }

    fn to_file(&self) -> Vec<u8> {
        let file = KeyFile { v: 1, alg: "ed25519".into(), secret: URL_SAFE_NO_PAD.encode(&self.pkcs8), public: self.public_key() };
        let mut bytes = serde_json::to_vec_pretty(&file).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    fn from_file(bytes: &[u8]) -> Result<InstanceKey, ErrorBody> {
        let damaged = || err(format!("{} is damaged; move it away to make a new key (then pair your machines again)", Self::FILE));
        let file: KeyFile = serde_json::from_slice(bytes).map_err(|_| damaged())?;
        if file.v != 1 || file.alg != "ed25519" {
            return Err(damaged());
        }
        let pkcs8 = URL_SAFE_NO_PAD.decode(file.secret.trim()).map_err(|_| damaged())?;
        let key = Self::from_pkcs8(pkcs8).ok_or_else(damaged)?;
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

/// A fresh random nonce for a handshake, base64url.
pub fn nonce() -> String {
    let mut bytes = [0u8; 32];
    // The system's random source failing is not something to go on from.
    SystemRandom::new().fill(&mut bytes).expect("system random source");
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::testing::TempDir;

    #[test]
    fn made_once_private_and_stable() {
        let dir = TempDir::new("peer-key");
        let key = InstanceKey::load_or_create(dir.path()).unwrap();
        let path = InstanceKey::path(dir.path());
        assert!(path.ends_with("peers/identity.key"));
        assert!(platform::is_private(&path));
        let again = InstanceKey::load_or_create(dir.path()).unwrap();
        assert_eq!(key.public_key(), again.public_key());
        assert!(key.public_key().starts_with("ed25519:"));
        assert!(!format!("{key:?}").contains(&URL_SAFE_NO_PAD.encode(&key.pkcs8)));

        std::fs::write(&path, b"{\"v\":1}").unwrap();
        assert!(InstanceKey::load_or_create(dir.path()).unwrap_err().message.contains("damaged"));
    }

    #[test]
    fn signs_and_verifies() {
        let key = InstanceKey::generate().unwrap();
        let other = InstanceKey::generate().unwrap();
        let sig = key.sign(b"record");
        assert!(InstanceKey::verify(&key.public_key(), b"record", &sig));
        assert!(!InstanceKey::verify(&key.public_key(), b"recorD", &sig));
        assert!(!InstanceKey::verify(&other.public_key(), b"record", &sig));
        assert!(!InstanceKey::verify("ed25519:nope", b"record", &sig));
        assert!(!InstanceKey::verify(&key.public_key(), b"record", "rsa:abc"));
        assert_ne!(key.public_key(), other.public_key());
        assert_ne!(nonce(), nonce());
    }
}
