//! Errors with a code (architecture.md §2): clients react to the code, people
//! read the message. Providers, terminals and [`Exec`](crate::exec::Exec)
//! return [`PwError`]; the host-facing service functions still return
//! `String` (the message) until the protocol carries codes (step 5), so
//! `PwError` converts into `String` with `?`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// No such agent, session, file or machine.
    NotFound,
    /// The provider (or kind) can't do this at all; matches a `false` capability.
    Unsupported,
    /// The agent or session isn't running.
    NotRunning,
    /// The machine or provider can't be reached right now.
    Unreachable,
    /// It exists already, or someone else changed it first.
    Conflict,
    /// Not allowed (approval denied, permissions).
    Denied,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PwError {
    pub code: ErrorCode,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, PwError>;

impl PwError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> PwError {
        PwError { code, message: message.into() }
    }
    pub fn other(message: impl Into<String>) -> PwError {
        PwError::new(ErrorCode::Other, message)
    }
    pub fn not_found(message: impl Into<String>) -> PwError {
        PwError::new(ErrorCode::NotFound, message)
    }
    pub fn unsupported(message: impl Into<String>) -> PwError {
        PwError::new(ErrorCode::Unsupported, message)
    }
    pub fn not_running() -> PwError {
        PwError::new(ErrorCode::NotRunning, "agent is not running")
    }
    pub fn unreachable(message: impl Into<String>) -> PwError {
        PwError::new(ErrorCode::Unreachable, message)
    }

    /// The same code with `context: ` in front of the message.
    pub fn context(self, context: &str) -> PwError {
        PwError { code: self.code, message: format!("{context}: {}", self.message) }
    }

    pub fn is(&self, code: ErrorCode) -> bool {
        self.code == code
    }

    /// Whether the message contains `needle` (tests and git error sniffing).
    pub fn contains(&self, needle: &str) -> bool {
        self.message.contains(needle)
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.message.starts_with(prefix)
    }
}

impl fmt::Display for PwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PwError {}

impl From<String> for PwError {
    fn from(message: String) -> PwError {
        PwError::other(message)
    }
}

impl From<&str> for PwError {
    fn from(message: &str) -> PwError {
        PwError::other(message)
    }
}

impl From<std::io::Error> for PwError {
    fn from(e: std::io::Error) -> PwError {
        let code = match e.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::Denied,
            std::io::ErrorKind::Unsupported => ErrorCode::Unsupported,
            std::io::ErrorKind::AlreadyExists => ErrorCode::Conflict,
            _ => ErrorCode::Other,
        };
        PwError::new(code, e.to_string())
    }
}

/// Host-facing functions still return `Result<_, String>`.
impl From<PwError> for String {
    fn from(e: PwError) -> String {
        e.message
    }
}

impl PartialEq<&str> for PwError {
    fn eq(&self, other: &&str) -> bool {
        self.message == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_survive_context_and_messages_convert() {
        let e = PwError::not_running().context("send");
        assert!(e.is(ErrorCode::NotRunning));
        assert_eq!(e.to_string(), "send: agent is not running");
        let s: String = PwError::unsupported("no resume").into();
        assert_eq!(s, "no resume");
        let io: PwError = std::io::Error::from(std::io::ErrorKind::NotFound).into();
        assert!(io.is(ErrorCode::NotFound));
        assert_eq!(PwError::from("x"), "x");
    }
}
