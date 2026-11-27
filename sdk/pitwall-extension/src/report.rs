//! What a tile reports about the extension, shown where an adapter's
//! report is: its connection's [`status`].

use crate::bindings::pitwall::tile as wit;

/// The extension's connection to its service, as an adapter reports it:
/// the pill on its card in Settings ("Connected", "Signed out"…), its
/// checklist's Connected step, and the update's check that it connects
/// again. The manifest says `"status": true` in `contributes`; until the
/// first [`set`] it is [`State::Connecting`]. Report it from a component
/// that runs while the extension is on (a toolbar button with `"badge":
/// true` keeps it running with no view open).
///
/// ```ignore
/// status::set(status::State::SignedOut, "The server refused the API key: paste a new one in Settings.");
/// ```
pub mod status {
    use super::wit;

    /// How the connection is.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum State {
        /// Starting to connect, or connecting again.
        Connecting,
        /// Connected and working.
        Ready,
        /// A credential is missing, wrong, expired or lacks access.
        SignedOut,
        /// It can't connect, or the service failed.
        Error,
    }

    /// The connection now, with a sentence for the user (at most 500
    /// characters; usually empty for [`State::Ready`]).
    pub fn set(state: State, message: &str) {
        let s = match state {
            State::Connecting => wit::status::State::Connecting,
            State::Ready => wit::status::State::Ready,
            State::SignedOut => wit::status::State::SignedOut,
            State::Error => wit::status::State::Error,
        };
        wit::status::set(s, message);
    }
}
