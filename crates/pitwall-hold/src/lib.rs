//! `pitwall-hold`: one tiny process per agent that owns the PTY master and the
//! agent's process, so the terminal survives anything that happens to Pitwall
//! itself (app quit, rebuild, crash, later a daemon update). See
//! architecture.md §9, decision 1.
//!
//! The holder only provides bytes. Ring buffers for the UI, the headless
//! `Screen`, fan-out to panes, status detection and paste timing all stay in
//! the client (today the app's `session.rs`).
//!
//! # Running it
//!
//! ```text
//! pitwall-hold --socket <path> [--cols N] [--rows N] [--cwd DIR] [--grace-ms N]
//!              -- <program> [args…]
//! pitwall-hold --version
//! ```
//!
//! The process you start detaches a holder (Unix: fork + `setsid()`;
//! Windows: it re-spawns itself detached, in its own process group, so it
//! survives its parent and gets none of its terminal signals). The holder
//! binds the socket (refusing if a live holder with a running child already
//! answers there) and starts the child; the launcher then prints
//! `ready <holder_pid> <child_pid>` on stdout and exits 0 (on failure: a
//! message on stderr, exit 1). The holder passes its own environment to the
//! child unchanged: the caller prepares it. The child runs in the PTY with
//! `cwd` as its working directory; the holder itself sits in `/`.
//!
//! When the child exits, the holder sends `EXIT` to attached clients, keeps
//! answering for `--grace-ms` (default 3000) so a client that connects late
//! can still read the final status, removes its socket and exits. After a
//! `SHUTDOWN` request it exits as soon as the child is gone and its clients
//! have their last frames.
//!
//! # Protocol, version 1
//!
//! A local stream socket (`interprocess`: a Unix domain socket file, mode
//! 0600 in a 0700 directory; a named pipe on Windows). OS-neutral: no fd
//! passing, no signals on the wire. Every message is a frame:
//!
//! ```text
//! frame := len:u32be  type:u8  body        (len counts type + body; ≤ 16 MiB)
//! ```
//!
//! Client → holder:
//!
//! | type | name     | body                    | effect |
//! |------|----------|-------------------------|--------|
//! | 0x01 | HELLO    | version:u16be           | must be first; answered by WELCOME |
//! | 0x02 | ATTACH   | replay:u8               | stream output to this connection: REPLAY frames with the ring (≤ 1 MiB, after the terminal modes set before it) if `replay` = 1, then OUTPUT; EXIT if the child is gone |
//! | 0x03 | INPUT    | bytes                   | written to the PTY verbatim |
//! | 0x04 | RESIZE   | cols:u16be rows:u16be   | `TIOCSWINSZ` (the child gets SIGWINCH); Windows: resizes the ConPTY |
//! | 0x05 | STATUS   | —                       | answered by INFO |
//! | 0x06 | SHUTDOWN | grace_ms:u32be          | hang up on the child (SIGHUP; Windows: close its console), kill it after `grace_ms`; the holder exits after it |
//!
//! Holder → client:
//!
//! | type | name    | body |
//! |------|---------|------|
//! | 0x81 | WELCOME | version:u16be, then an INFO body |
//! | 0x82 | OUTPUT  | bytes from the PTY |
//! | 0x83 | REPLAY  | bytes from the ring, sent once right after ATTACH, before any OUTPUT |
//! | 0x84 | EXIT    | code:i32be (exit status, or 128 + signal) |
//! | 0x85 | INFO    | holder_pid:u32be child_pid:u32be cols:u16be rows:u16be exited:u8 code:i32be |
//! | 0x86 | ERROR   | UTF-8 message |
//!
//! Compatibility: the frame header and the HELLO / WELCOME layouts are
//! frozen. A holder always answers HELLO with WELCOME carrying its own
//! version; if it can't speak the client's version it closes the connection
//! after WELCOME. Within a version, changes are additive: unknown frame types
//! are answered with ERROR and otherwise ignored, and extra trailing body
//! bytes are ignored. A holder is only ever replaced when its version changes,
//! and only when the user chooses to.
//!
//! # Portability
//!
//! OS-specific code lives only in `platform/` (see `platform/mod.rs` for the
//! interface a Windows port implements); `proto`, `server` and `client` are
//! OS-neutral.

pub mod client;
pub mod modes;
mod platform;
pub mod proto;
pub mod server;

pub use proto::{Info, Msg, PROTOCOL_VERSION};
