//! The system's secure store, for the secrets extensions ask for
//! (docs/spec/extensions.md, "Settings and secrets"): the login Keychain on
//! macOS, the Credential Manager on Windows, the Secret Service (GNOME
//! Keyring, KWallet) over D-Bus on Linux. Files only record that a secret
//! is set, never the secret.
//!
//! Every call may block (the system may ask the user), so callers run them
//! off the UI thread. Tests use [`Memory`] and never touch the real store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What items are filed under (the Keychain's "service", the Credential
/// Manager's target prefix, the Secret Service's `service` attribute).
pub const SERVICE: &str = "Pitwall extensions";

/// A store of secrets by account name.
pub trait SecretStore: Send + Sync {
    /// The secret kept for `account`, if any.
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    /// Keep `secret` for `account` (replacing any).
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    /// Forget `account`'s secret (nothing kept is fine).
    fn delete(&self, account: &str) -> Result<(), String>;
}

/// Secrets in memory (tests; and where the system has no store).
#[derive(Default)]
pub struct Memory(Mutex<HashMap<String, String>>);

impl SecretStore for Memory {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(account);
        Ok(())
    }
}

/// The system's store.
pub fn system() -> Arc<dyn SecretStore> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(super::secrets_macos::Keychain)
    }
    #[cfg(windows)]
    {
        Arc::new(super::secrets_windows::CredentialManager)
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(super::secrets_linux::SecretService)
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        Arc::new(Unavailable)
    }
}

/// No secure store on this system: nothing is kept.
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
struct Unavailable;

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
impl SecretStore for Unavailable {
    fn get(&self, _: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("This system has no secure store Pitwall can use.".into())
    }

    fn delete(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_keeps_replaces_and_forgets() {
        let m = Memory::default();
        assert_eq!(m.get("a").unwrap(), None);
        m.set("a", "1").unwrap();
        m.set("a", "2").unwrap();
        assert_eq!(m.get("a").unwrap().as_deref(), Some("2"));
        m.delete("a").unwrap();
        m.delete("a").unwrap();
        assert_eq!(m.get("a").unwrap(), None);
    }
}
