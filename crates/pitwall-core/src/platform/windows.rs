//! Windows implementations: `%APPDATA%\Pitwall`, PowerShell as the login
//! shell, per-user named pipes for local sockets, and the `pitwall-hook`
//! binary as the hook relay. Process queries are in `windows_procs.rs`.
//!
//! The per-user pipe security descriptor (the account SID from the process
//! token in an SDDL DACL) follows the approach of Herdr's Windows platform
//! layer (Apache-2.0, see NOTICE); the code is Pitwall's own.

use std::io::{self, Read, Write};
use std::os::windows::io::{AsHandle, AsRawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use interprocess::os::windows::named_pipe::{pipe_mode, DuplexPipeStream, PipeListener, PipeListenerOptions, PipeMode};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use interprocess::ConnectWaitMode;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken, CREATE_NO_WINDOW};

use pitwall_proto::pipe::pipe_name;
use crate::exec::{Cmd, Exec, LocalExec};

pub use super::windows_procs::{process_cwds, process_table};

/// How the UI names this machine (`host::HostInfo`).
pub const MACHINE_LABEL: &str = "This PC";
/// Ctrl+Shift shortcuts: Ctrl alone belongs to the terminal.
pub const SHORTCUTS: crate::host::Shortcuts = crate::host::Shortcuts::CtrlShift;
/// No Dock; the tray icon brings a hidden window back.
pub const HAS_DOCK: bool = false;
pub const HAS_TRAY: bool = true;
/// A File menu without Edit accelerators (Ctrl+C belongs to terminals).
pub const MENU_BAR: crate::host::MenuBar = crate::host::MenuBar::File;
/// Taskbar overlay icon + tray tooltip.
pub const BADGE: crate::host::Badge = crate::host::Badge::Taskbar;
/// Per-user named pipes.
pub const LOCAL_SOCKETS: crate::host::LocalSockets = crate::host::LocalSockets::NamedPipe;

/// How long a client waits for a busy pipe.
const CONNECT_WAIT: Duration = Duration::from_secs(3);

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home_dir() -> PathBuf {
    env_path("USERPROFILE")
        .or_else(|| match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
            (Some(d), Some(p)) => Some(PathBuf::from(format!("{}{}", d.to_string_lossy(), p.to_string_lossy()))),
            _ => None,
        })
        .unwrap_or_else(|| PathBuf::from(r"C:\"))
}

/// `%APPDATA%\Pitwall`, or `$PITWALL_HOME` when set.
pub fn data_dir() -> PathBuf {
    match std::env::var_os(crate::paths::HOME_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => app_support_dir().join("Pitwall"),
    }
}

/// Per-user application data of other apps (`%APPDATA%`: VS Code keeps its
/// recently opened folders in `%APPDATA%\Code\User\globalStorage`).
pub fn app_support_dir() -> PathBuf {
    env_path("APPDATA").unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
}

/// `PATHEXT`, lower case (`.com .exe .bat .cmd` when unset).
fn path_exts() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|e| e.starts_with('.'))
        .map(|e| e.to_ascii_lowercase())
        .collect()
}

/// A file Windows runs as a program (its extension is in `PATHEXT`).
pub fn is_executable(path: &Path) -> bool {
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()));
    path.is_file() && ext.is_some_and(|e| path_exts().contains(&e))
}

/// Windows has no executable bit. (Tests use it on every OS.)
#[allow(dead_code)]
pub fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// `path` plus each `PATHEXT` extension (`rulesync` → `rulesync.cmd`, …).
pub fn executable_candidates(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![path.to_path_buf()];
    for ext in path_exts() {
        let mut p = path.as_os_str().to_owned();
        p.push(&ext);
        out.push(PathBuf::from(p));
    }
    out
}

/// `std::fs::canonicalize` without the `\\?\` prefix where a plain path
/// says the same (`C:\…`), so paths compare and display as users know them.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    let p = std::fs::canonicalize(path)?;
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        let b = rest.as_bytes();
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' && rest.len() < 260 {
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(p)
}

/// Console programs started from the GUI app would flash a console window.
pub fn hide_console(cmd: &mut std::process::Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Start the user's program (their editor) and leave it running on its own:
/// no stdio, the app's login PATH, never waited for here (a thread reaps
/// it) and never killed.
pub fn spawn_detached(argv: &[String]) -> io::Result<()> {
    let (program, args) = argv.split_first().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let mut c = std::process::Command::new(program);
    c.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(path) = crate::shell::spawn_path() {
        c.env("PATH", path);
    }
    hide_console(&mut c);
    let mut child = c.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// A symlink at `link` to `target` (tests; needs Developer Mode or admin).
#[cfg(test)]
pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// Opens a file with its associated program ("Open in editor" without an
/// editor set or found).
pub const SYSTEM_OPENER: &[&str] = &["explorer.exe"];

// ---------------------------------------------------------------- PATH and shell

/// A string value from the registry (environment strings expanded).
fn reg_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (subkey, value) = (wide(subkey), wide(value));
    let mut size = 0u32;
    let rc = unsafe {
        RegGetValueW(root, subkey.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let rc = unsafe {
        RegGetValueW(root, subkey.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    if rc != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// PATH as a new terminal would have it (system + user, from the registry),
/// then Pitwall's own: programs installed after Pitwall started are found.
pub fn fresh_path() -> String {
    let machine = reg_string(HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment", "Path");
    let user = reg_string(HKEY_CURRENT_USER, "Environment", "Path");
    let own = std::env::var("PATH").ok();
    let mut seen: Vec<String> = Vec::new();
    for part in [machine, user, own].into_iter().flatten() {
        for dir in part.split(';').map(str::trim).filter(|d| !d.is_empty()) {
            if !seen.iter().any(|s| s.eq_ignore_ascii_case(dir)) {
                seen.push(dir.to_string());
            }
        }
    }
    seen.join(";")
}

/// PATH without asking a shell: the registry's ([`fresh_path`]), which is
/// what PowerShell's login PATH is too (no slow PowerShell start needed).
pub fn system_path() -> Option<String> {
    Some(fresh_path())
}

/// Where package managers put programs outside the registry's PATH.
pub fn common_bin_dirs(home: &Path) -> Vec<PathBuf> {
    vec![home.join(".cargo").join("bin")]
}

/// PowerShell 7 (`pwsh.exe`) when installed, else Windows PowerShell.
pub fn default_login_shell() -> String {
    static SHELL: OnceLock<String> = OnceLock::new();
    SHELL
        .get_or_init(|| {
            let pwsh = fresh_path().split(';').map(|d| Path::new(d).join("pwsh.exe")).find(|p| p.is_file());
            match pwsh {
                Some(p) => p.to_string_lossy().into_owned(),
                None => {
                    let root = env_path("SystemRoot").unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
                    root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe").to_string_lossy().into_owned()
                }
            }
        })
        .clone()
}

/// Where `program` is installed: `where.exe` with the fresh PATH, preferring
/// what Windows can start directly (`.exe`, `.cmd`, …) over extensionless
/// npm shims meant for bash.
pub fn which(program: &str) -> Option<String> {
    if program.contains(['\\', '/']) {
        return executable_candidates(Path::new(program)).into_iter().find(|p| is_executable(p)).map(|p| p.to_string_lossy().into_owned());
    }
    let path = fresh_path();
    let out = LocalExec.run(&Cmd::new(&["where.exe", program]).env(&[("PATH", &path)]).timeout(Duration::from_secs(10))).ok()?;
    if !out.ok() {
        return None;
    }
    pick_where(&out.stdout_text(), &path_exts())
}

/// The first line of `where` output that Windows can run directly.
pub(crate) fn pick_where(out: &str, exts: &[String]) -> Option<String> {
    let lines: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let runnable = |l: &str| {
        let lower = l.to_ascii_lowercase();
        exts.iter().any(|e| lower.ends_with(e.as_str()))
    };
    lines.iter().find(|l| runnable(l)).or(lines.first()).map(|l| l.to_string())
}

// ---------------------------------------------------------------- hook relay

/// `pitwall-hook.exe` shipped next to Pitwall (`$PITWALL_HOOK_BIN` first;
/// tests: the target folder above `deps`).
fn hook_binary() -> Option<PathBuf> {
    if let Some(p) = env_path("PITWALL_HOOK_BIN") {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("pitwall-hook.exe"), dir.parent()?.join("pitwall-hook.exe")].into_iter().find(|p| p.is_file())
}

/// Copy the relay binary to `dest` (a stable path that hook configs keep).
/// A copy that is running right now can't be replaced: then the one there stays.
pub fn install_hook_relay(dest: &Path, _script: &str) -> Result<(), String> {
    let Some(src) = hook_binary() else {
        return if dest.is_file() { Ok(()) } else { Err("the hook relay (pitwall-hook.exe) is missing next to Pitwall".into()) };
    };
    let same = match (std::fs::read(&src), std::fs::read(dest)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Ok(());
    }
    match std::fs::copy(&src, dest) {
        Ok(_) => Ok(()),
        Err(_) if dest.is_file() => Ok(()),
        Err(e) => Err(format!("could not install the hook relay: {e}")),
    }
}

/// The relay's path with forward slashes (bash, which Claude Code runs hooks
/// with on Windows, and cmd / PowerShell all run it), quoted if it has spaces.
pub fn hook_relay_command(dest: &Path) -> String {
    let p = dest.to_string_lossy().replace('\\', "/");
    if p.contains(' ') {
        format!("\"{p}\"")
    } else {
        p
    }
}

// ---------------------------------------------------------------- privacy

/// Windows has no Full Disk Access permission.
pub fn full_disk_access_probes() -> Vec<io::Result<()>> {
    Vec::new()
}

pub fn open_system_url(_url: &str) -> Result<(), String> {
    Err("privacy settings are a macOS feature".into())
}

// ---------------------------------------------------------------- local sockets

/// Closes a raw handle on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut needed = 0u32;
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), needed, &mut needed) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetTokenInformation filled an aligned TOKEN_USER.
    let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };
    let mut text: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut len = 0;
    while unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
    unsafe { LocalFree(text.cast()) };
    Ok(sid)
}

/// Full access for this user and SYSTEM, nobody else.
fn current_user_only() -> io::Result<SecurityDescriptor> {
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{})", current_user_sid()?);
    let wide = widestring::U16CString::from_str(sddl).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    SecurityDescriptor::deserialize(&wide)
}

type Pipe = DuplexPipeStream<pipe_mode::Bytes>;

/// A listening named pipe (derived from a Pitwall socket path), reachable
/// only by this user.
pub struct LocalListener(PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>);

impl LocalListener {
    /// Listen on the pipe for `path`; another Pitwall listening there is an
    /// error (`AddrInUse`).
    pub fn bind(path: &Path) -> io::Result<LocalListener> {
        let name = pipe_name(&path.to_string_lossy());
        if Pipe::connect_by_path_with_wait_mode(name.as_str(), ConnectWaitMode::Timeout(Duration::from_millis(200))).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AddrInUse, format!("another Pitwall is listening at {}", path.display())));
        }
        PipeListenerOptions::new()
            .path(name.as_str())
            .mode(PipeMode::Bytes)
            .security_descriptor(Some(current_user_only()?))
            .create_duplex::<pipe_mode::Bytes>()
            .map(LocalListener)
    }

    pub fn accept(&self) -> io::Result<LocalStream> {
        self.0.accept().map(LocalStream::new)
    }

    pub fn incoming(&self) -> impl Iterator<Item = LocalStream> + '_ {
        std::iter::from_fn(move || loop {
            match self.accept() {
                Ok(s) => return Some(s),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        })
    }
}

struct Inner {
    pipe: Pipe,
    /// Milliseconds; 0 = none.
    read_timeout: AtomicU64,
}

/// One pipe connection. Clones share it (one can read while another writes).
/// Named pipes have no read timeout of their own: a read waits until bytes
/// are available (or the time is up, consuming nothing).
#[derive(Clone)]
pub struct LocalStream(Arc<Inner>);

impl LocalStream {
    fn new(pipe: Pipe) -> LocalStream {
        LocalStream(Arc::new(Inner { pipe, read_timeout: AtomicU64::new(0) }))
    }

    /// Connect to the pipe for `path`.
    pub fn connect(path: &Path) -> io::Result<LocalStream> {
        let name = pipe_name(&path.to_string_lossy());
        Pipe::connect_by_path_with_wait_mode(name.as_str(), ConnectWaitMode::Timeout(CONNECT_WAIT)).map(LocalStream::new)
    }

    pub fn set_timeouts(&self, d: Duration) {
        let _ = self.set_read_timeout(Some(d));
    }

    pub fn set_read_timeout(&self, d: Option<Duration>) -> io::Result<()> {
        let ms = d.map_or(0, |t| t.as_millis().clamp(1, u64::MAX as u128) as u64);
        self.0.read_timeout.store(ms, Ordering::Relaxed);
        Ok(())
    }

    pub fn try_clone(&self) -> io::Result<LocalStream> {
        Ok(self.clone())
    }

    /// The process at the other end (on the listening side).
    pub fn peer_pid(&self) -> Option<u32> {
        self.0.pipe.client_process_id().ok().filter(|p| *p != 0)
    }

    fn wait_readable(&self) -> io::Result<()> {
        let ms = self.0.read_timeout.load(Ordering::Relaxed);
        if ms == 0 {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_millis(ms);
        let h = self.0.pipe.as_handle().as_raw_handle() as HANDLE;
        loop {
            let mut avail = 0u32;
            let ok = unsafe { PeekNamedPipe(h, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut()) };
            if ok == 0 || avail > 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Two connected ends (tests).
    #[cfg(test)]
    pub fn pair() -> io::Result<(LocalStream, LocalStream)> {
        let path = std::env::temp_dir().join(format!("pw-pair-{}-{}.sock", std::process::id(), uuid::Uuid::new_v4()));
        let listener = LocalListener::bind(&path)?;
        let client = std::thread::spawn(move || LocalStream::connect(&path));
        let server = listener.accept()?;
        let client = client.join().map_err(|_| io::Error::other("connect panicked"))??;
        Ok((client, server))
    }
}

impl Read for LocalStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.wait_readable()?;
        (&self.0.pipe).read(buf)
    }
}

impl Write for LocalStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.0.pipe).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.pipe.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_sockets_carry_bytes_both_ways() {
        let (mut a, mut b) = LocalStream::pair().unwrap();
        a.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
        b.set_timeouts(Duration::from_millis(50));
        assert_eq!(b.read(&mut buf).unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert_eq!(b.peer_pid(), Some(std::process::id()));
    }

    #[test]
    fn where_output_prefers_runnable_files() {
        let exts: Vec<String> = [".exe", ".cmd"].map(String::from).to_vec();
        let out = "C:\\Users\\d\\AppData\\Roaming\\npm\\claude\r\nC:\\Users\\d\\AppData\\Roaming\\npm\\claude.cmd\r\n";
        assert_eq!(pick_where(out, &exts).as_deref(), Some("C:\\Users\\d\\AppData\\Roaming\\npm\\claude.cmd"));
        assert_eq!(pick_where("", &exts), None);
    }

    #[test]
    fn hook_command_runs_in_any_shell() {
        assert_eq!(hook_relay_command(Path::new(r"C:\Users\dev\AppData\Roaming\Pitwall\bin\pitwall-hook.exe")), "C:/Users/dev/AppData/Roaming/Pitwall/bin/pitwall-hook.exe");
        assert_eq!(hook_relay_command(Path::new(r"C:\Users\A B\x.exe")), "\"C:/Users/A B/x.exe\"");
    }
}
