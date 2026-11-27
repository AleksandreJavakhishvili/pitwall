//! The https client extensions from an npm package are fetched with
//! ([`pitwall_core::extensions::npm::Http`], docs/spec/extensions.md,
//! "Updates from an npm package"): ureq with rustls and Mozilla's roots.
//! No credentials, cookies or proxy (ureq reads none from the
//! environment), a user agent naming Pitwall, timeouts, at most `limit`
//! bytes, redirects only to https. Plain http only to a loopback address:
//! a test's stand-in for a registry.

use std::io::Read;
use std::time::Duration;

use pitwall_core::extensions::npm::{is_loopback, status_text, GetError, Http, OFFLINE};

/// Pitwall's https client.
pub struct Https;

impl Http for Https {
    fn get(&self, url: &str, accept: &str, limit: u64, connect: Duration, timeout: Duration) -> Result<Vec<u8>, GetError> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(connect)
            .timeout(timeout)
            .redirects(5)
            .https_only(!is_loopback(url))
            .user_agent(concat!("pitwall/", env!("CARGO_PKG_VERSION")))
            .build();
        let resp = match agent.get(url).set("Accept", accept).call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => return Err(GetError::Failed(status_text(code))),
            Err(ureq::Error::Transport(t)) => {
                return Err(GetError::Failed(match t.kind() {
                    ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Io | ureq::ErrorKind::ProxyConnect => OFFLINE.into(),
                    ureq::ErrorKind::InsecureRequestHttpsOnly => "The registry sent Pitwall to an address that isn't https. Not fetched.".into(),
                    _ => format!("Couldn't fetch it: {}", t.to_string().chars().take(200).collect::<String>()),
                }))
            }
        };
        if resp.header("Content-Length").and_then(|l| l.trim().parse::<u64>().ok()).is_some_and(|l| l > limit) {
            return Err(GetError::TooBig);
        }
        let mut buf = Vec::new();
        resp.into_reader().take(limit + 1).read_to_end(&mut buf).map_err(|_| GetError::Failed("Couldn't reach it: the download stopped (offline, or too slow).".into()))?;
        if buf.len() as u64 > limit {
            return Err(GetError::TooBig);
        }
        Ok(buf)
    }
}

/// The extensions index over https (marketplace.md §7): the same client,
/// with a conditional GET (`If-None-Match`) so a daily check of an
/// unchanged index costs one 304.
impl pitwall_core::extensions::index::IndexHttp for Https {
    fn get(&self, url: &str, etag: Option<&str>, limit: u64) -> Result<pitwall_core::extensions::index::Answer, GetError> {
        use pitwall_core::extensions::index::Answer;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirects(5)
            .https_only(!is_loopback(url))
            .user_agent(concat!("pitwall/", env!("CARGO_PKG_VERSION")))
            .build();
        let mut req = agent.get(url).set("Accept", "application/json");
        if let Some(e) = etag {
            req = req.set("If-None-Match", e);
        }
        let resp = match req.call() {
            Ok(r) if r.status() == 304 => return Ok(Answer::NotModified),
            Ok(r) => r,
            Err(ureq::Error::Status(304, _)) => return Ok(Answer::NotModified),
            Err(ureq::Error::Status(code, _)) => return Err(GetError::Failed(format!("The extensions index answered {code}."))),
            Err(ureq::Error::Transport(t)) => {
                return Err(GetError::Failed(match t.kind() {
                    ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Io | ureq::ErrorKind::ProxyConnect => "Couldn't reach the extensions index: this computer seems to be offline.".into(),
                    _ => format!("Couldn't fetch the extensions index: {}", t.to_string().chars().take(200).collect::<String>()),
                }))
            }
        };
        let new_etag = resp.header("ETag").map(str::to_string);
        if resp.header("Content-Length").and_then(|l| l.trim().parse::<u64>().ok()).is_some_and(|l| l > limit) {
            return Err(GetError::TooBig);
        }
        let mut buf = Vec::new();
        resp.into_reader().take(limit + 1).read_to_end(&mut buf).map_err(|_| GetError::Failed("Couldn't reach the extensions index: the download stopped.".into()))?;
        if buf.len() as u64 > limit {
            return Err(GetError::TooBig);
        }
        Ok(Answer::Body { bytes: buf, etag: new_etag })
    }
}
