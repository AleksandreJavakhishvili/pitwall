//! Windows: the socket is a named pipe only this user can open
//! (`pitwall_core::ipc`); the peer's pid comes from the pipe; the process
//! table from a Toolhelp snapshot.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;
use std::time::Duration;

use pitwall_core::ipc::{LocalListener, LocalStream};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

use crate::identity::Proc;

pub type Stream = LocalStream;

/// Accepted connections, like `UnixListener::incoming`.
pub struct Listener(LocalListener);

impl Listener {
    pub fn incoming(&self) -> impl Iterator<Item = io::Result<Stream>> + '_ {
        std::iter::from_fn(move || Some(self.0.accept()))
    }
}

/// Listen at the pipe for `path` (only this user can connect). Another
/// Pitwall listening there is an error.
pub fn bind(path: &Path) -> io::Result<Listener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    LocalListener::bind(path).map(Listener)
}

/// Wake a blocked `accept` (shutting down). The connection stays open until
/// the server drops it: a pipe client that is already gone when the server
/// gets to `ConnectNamedPipe` is skipped there (`ERROR_NO_DATA`), and the
/// accept would go on waiting.
pub fn poke(path: &Path) {
    if let Ok(mut s) = LocalStream::connect(path) {
        let _ = s.set_read_timeout(Some(POKE_WAIT));
        let _ = s.read(&mut [0u8; 1]);
    }
}

/// How long `poke` keeps its connection open at most.
const POKE_WAIT: Duration = Duration::from_secs(2);

/// The pid of the process at the other end.
pub fn peer_pid(s: &Stream) -> Option<u32> {
    s.peer_pid()
}

/// Every process's parent and program name (Toolhelp snapshot).
pub fn process_table() -> Option<HashMap<u32, Proc>> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut out = HashMap::new();
    let mut ok = unsafe { Process32FirstW(snap, &mut e) } != 0;
    while ok {
        let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        let name = String::from_utf16_lossy(&e.szExeFile[..len]);
        out.insert(e.th32ProcessID, Proc { ppid: e.th32ParentProcessID, name });
        ok = unsafe { Process32NextW(snap, &mut e) } != 0;
    }
    unsafe { CloseHandle(snap) };
    Some(out)
}
