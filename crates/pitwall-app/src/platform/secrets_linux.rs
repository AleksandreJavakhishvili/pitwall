//! Secrets in the Secret Service (GNOME Keyring, KWallet) over the session
//! D-Bus: items of the default collection with the attributes
//! `service` = [`SERVICE`] and `account`, sent in the "plain" session (the
//! bus is the user's own). A locked keyring isn't unlocked from here: the
//! user is told to unlock it.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::secrets::{SecretStore, SERVICE};

const DEST: &str = "org.freedesktop.secrets";
const PATH: &str = "/org/freedesktop/secrets";
const DEFAULT: &str = "/org/freedesktop/secrets/aliases/default";

type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

fn err(e: impl std::fmt::Display) -> String {
    format!("The Secret Service (your keyring) couldn't be used: {e}")
}

const LOCKED: &str = "Your keyring is locked: unlock it (for example by signing in again), then try again.";

pub struct SecretService;

struct Session {
    conn: Connection,
    path: OwnedObjectPath,
}

impl Session {
    fn open() -> Result<Self, String> {
        let conn = Connection::session().map_err(err)?;
        let path = {
            let service = Proxy::new(&conn, DEST, PATH, "org.freedesktop.Secret.Service").map_err(err)?;
            let (_, path): (OwnedValue, OwnedObjectPath) = service.call("OpenSession", &("plain", Value::from(""))).map_err(err)?;
            path
        };
        Ok(Session { conn, path })
    }

    fn service(&self) -> Result<Proxy<'_>, String> {
        Proxy::new(&self.conn, DEST, PATH, "org.freedesktop.Secret.Service").map_err(err)
    }

    /// The items for `account`: unlocked ones, and whether locked ones exist.
    fn find(&self, account: &str) -> Result<(Vec<OwnedObjectPath>, bool), String> {
        let attrs: HashMap<&str, &str> = HashMap::from([("service", SERVICE), ("account", account)]);
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = self.service()?.call("SearchItems", &(attrs,)).map_err(err)?;
        Ok((unlocked, !locked.is_empty()))
    }
}

/// Whether a prompt path means "the user has to act" (anything but `/`).
fn prompted(p: &OwnedObjectPath) -> bool {
    p.as_str() != "/"
}

impl SecretStore for SecretService {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        let s = Session::open()?;
        let (items, locked) = s.find(account)?;
        let Some(item) = items.into_iter().next() else {
            return if locked { Err(LOCKED.into()) } else { Ok(None) };
        };
        let paths: Vec<ObjectPath> = vec![item.as_ref()];
        let secrets: HashMap<OwnedObjectPath, Secret> = s.service()?.call("GetSecrets", &(paths, s.path.as_ref())).map_err(err)?;
        let Some((_, _, value, _)) = secrets.into_values().next() else { return Ok(None) };
        String::from_utf8(value).map(Some).map_err(|_| "The saved secret isn't text.".to_string())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        let s = Session::open()?;
        let collection = Proxy::new(&s.conn, DEST, DEFAULT, "org.freedesktop.Secret.Collection").map_err(err)?;
        let attrs: HashMap<String, String> = HashMap::from([("service".to_string(), SERVICE.to_string()), ("account".to_string(), account.to_string())]);
        let mut props: HashMap<&str, Value> = HashMap::new();
        props.insert("org.freedesktop.Secret.Item.Label", Value::from(format!("{SERVICE}: {account}")));
        props.insert("org.freedesktop.Secret.Item.Attributes", Value::from(attrs));
        let value = (s.path.as_ref(), Vec::<u8>::new(), secret.as_bytes().to_vec(), "text/plain");
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) = collection.call("CreateItem", &(props, value, true)).map_err(err)?;
        if prompted(&prompt) {
            return Err(LOCKED.into());
        }
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let s = Session::open()?;
        let (items, locked) = s.find(account)?;
        if items.is_empty() && locked {
            return Err(LOCKED.into());
        }
        for item in items {
            let p = Proxy::new(&s.conn, DEST, item, "org.freedesktop.Secret.Item").map_err(err)?;
            let prompt: OwnedObjectPath = p.call("Delete", &()).map_err(err)?;
            if prompted(&prompt) {
                return Err(LOCKED.into());
            }
        }
        Ok(())
    }
}
