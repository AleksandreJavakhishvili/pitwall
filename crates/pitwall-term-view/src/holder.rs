//! A Pitwall holder connection as a [`TermStream`] (feature `holder`): the
//! terminal shows what the holder's PTY prints, and input / resizes go to
//! the holder over its attach protocol (`pitwall-hold`, proto.rs).

use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use pitwall_hold::client::{self, Reader, Writer};
use pitwall_hold::{proto, Msg};

use crate::terminal::{Feed, TermSize, TermStream};

/// An attached holder connection.
pub struct HolderStream {
    writer: Mutex<Writer>,
    reader: Mutex<Option<Reader>>,
}

impl HolderStream {
    /// Connect to the holder listening at `socket` and attach. With
    /// `replay`, the holder first sends what it buffered (the screen so far).
    pub fn connect(socket: &Path, replay: bool) -> io::Result<HolderStream> {
        let mut conn = client::connect(socket, Duration::from_secs(5))?;
        conn.send(&proto::attach(replay))?;
        let (reader, writer) = conn.split();
        Ok(HolderStream { writer: Mutex::new(writer), reader: Mutex::new(Some(reader)) })
    }

    fn send(&self, frame: &[u8]) {
        let _ = self.writer.lock().unwrap().write_all(frame);
    }
}

impl TermStream for HolderStream {
    fn attach(&self, feed: Feed) {
        let Some(mut reader) = self.reader.lock().unwrap().take() else { return };
        thread::Builder::new()
            .name("holder-read".into())
            .spawn(move || {
                while let Ok(Some((ty, body))) = proto::read_frame(&mut reader) {
                    match Msg::decode(ty, body) {
                        Ok(Msg::Output(bytes) | Msg::Replay(bytes)) => feed.push(&bytes),
                        Ok(Msg::Exit(_)) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
                feed.close();
            })
            .expect("spawn holder reader");
    }

    fn write(&self, bytes: &[u8]) {
        self.send(&proto::input(bytes));
    }

    fn resize(&self, size: TermSize) {
        self.send(&proto::resize(size.cols, size.rows));
    }
}
