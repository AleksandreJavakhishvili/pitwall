//! Device previews (docs/spec/device.md): the byte formats a device tile
//! reads and writes, shared by the app and `pitwall-simhost`.
//!
//! - [`video`]: the screen as H.264, framed as scrcpy frames it (codec
//!   meta, then a 12-byte header per packet). scrcpy-server sends it from
//!   an Android device; `pitwall-simhost` sends the same from an iOS
//!   Simulator, so the app has one reader and one decoder.
//! - [`sim`]: what the app sends `pitwall-simhost` on its stdin (touches,
//!   keys, buttons) and the lines it answers on stderr.
//!
//! Bytes only: no sockets, no processes.

pub mod sim;
pub mod video;
