//! Windows: named pipes only the current user can open, a detached re-spawn
//! of the holder binary instead of fork, ConPTY (through `portable-pty`), and
//! processes by exact pid.
//!
//! The approach for the per-user pipe security descriptor (the account SID
//! from the process token in an SDDL DACL) follows Herdr's Windows platform
//! layer (Apache-2.0, see NOTICE); the code is Pitwall's own.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use interprocess::os::windows::named_pipe::{
    pipe_mode, DuplexPipeStream, PipeListener, PipeListenerOptions, PipeMode, RecvPipeStream, SendPipeStream,
};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use interprocess::ConnectWaitMode;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, SlavePty};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken, TerminateProcess, CREATE_BREAKAWAY_FROM_JOB,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};

use super::pipe_name::pipe_name;
use super::{Conn, PtySpec, Role};

/// How long a client waits for a busy pipe (every instance in use).
const CONNECT_WAIT: Duration = Duration::from_secs(3);
/// How long `listen` waits for a lingering predecessor to give up the name.
const TAKEOVER_WAIT: Duration = Duration::from_secs(3);
/// Set in the re-spawned holder's environment (removed before the child starts).
const HOLDER_ENV: &str = "PITWALL_HOLD_DETACHED";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Closes a raw handle on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

// --- Endpoint --------------------------------------------------------------

/// The account SID of this process's user, as text (`S-1-5-21-…`).
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
    // usize storage keeps TOKEN_USER aligned and its trailing SID alive.
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

/// Full access for this user and SYSTEM, nobody else (no inherited ACEs).
fn current_user_only() -> io::Result<SecurityDescriptor> {
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{})", current_user_sid()?);
    let wide = widestring::U16CString::from_str(sddl).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    SecurityDescriptor::deserialize(&wide)
}

pub type Listener = PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>;
pub type RecvHalf = RecvPipeStream<pipe_mode::Bytes>;
pub type SendHalf = SendPipeStream<pipe_mode::Bytes>;

/// Listen on the pipe for `path`. A predecessor that is on its way out (its
/// child exited) may still hold the name for a moment: wait for it.
pub fn listen(path: &Path) -> io::Result<Listener> {
    let name = pipe_name(&path.to_string_lossy());
    let deadline = Instant::now() + TAKEOVER_WAIT;
    loop {
        let res = PipeListenerOptions::new()
            .path(name.as_str())
            .mode(PipeMode::Bytes)
            .security_descriptor(Some(current_user_only()?))
            .create_duplex::<pipe_mode::Bytes>();
        match res {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

pub fn accept(listener: &Listener) -> io::Result<Stream> {
    listener.accept().map(Stream::new)
}

pub fn connect(path: &Path) -> io::Result<Stream> {
    let name = pipe_name(&path.to_string_lossy());
    DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path_with_wait_mode(name.as_str(), ConnectWaitMode::Timeout(CONNECT_WAIT))
        .map(Stream::new)
}

/// A pipe connection with receive timeouts (named pipes have none of their
/// own): before a read, wait until bytes are available, so a timed-out read
/// consumes nothing.
pub struct Stream {
    pipe: DuplexPipeStream<pipe_mode::Bytes>,
    /// Milliseconds; 0 = no timeout.
    recv_timeout: AtomicU64,
}

impl Stream {
    fn new(pipe: DuplexPipeStream<pipe_mode::Bytes>) -> Stream {
        Stream { pipe, recv_timeout: AtomicU64::new(0) }
    }

    fn raw(&self) -> RawHandle {
        self.pipe.as_handle().as_raw_handle()
    }

    fn wait_readable(&self) -> io::Result<()> {
        let ms = self.recv_timeout.load(Ordering::Relaxed);
        if ms == 0 {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let mut avail = 0u32;
            let ok = unsafe {
                PeekNamedPipe(self.raw() as HANDLE, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut())
            };
            // Closed or broken: let the read report it (EOF).
            if ok == 0 || avail > 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Conn for Stream {
    fn set_recv_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        let ms = timeout.map_or(0, |t| t.as_millis().clamp(1, u64::MAX as u128) as u64);
        self.recv_timeout.store(ms, Ordering::Relaxed);
        Ok(())
    }

    /// Writes of a few frames never wait on a reading peer for long; named
    /// pipes have no send timeout.
    fn set_send_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        Ok(())
    }

    fn split(self) -> (RecvHalf, SendHalf) {
        self.pipe.split()
    }
}

impl Read for &Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.wait_readable()?;
        (&self.pipe).read(buf)
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
}

impl Write for &Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.pipe).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.pipe.flush()
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        (&*self).flush()
    }
}

/// The pipe's directory is still made, so the layout matches Unix.
pub fn prepare_endpoint(path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// Access is set when the pipe is created (`listen`).
pub fn secure_endpoint(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// A pipe disappears with its last handle: nothing is left behind.
pub fn remove_stale_endpoint(_path: &Path) {}

pub fn endpoint_token(_path: &Path) -> Option<u64> {
    None
}

pub fn remove_endpoint(_path: &Path, _token: Option<u64>) {}

// --- Detaching -------------------------------------------------------------

/// A folder the holder can sit in without keeping anything of the user's busy
/// (a process's current folder can't be deleted on Windows).
fn neutral_dir() -> PathBuf {
    std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\"))
}

/// No fork on Windows: the launcher starts this same program again, detached
/// (no console, its own process group, out of the caller's job when allowed)
/// with its stdout on a pipe; the second instance sees `HOLDER_ENV` and is
/// the holder, reporting on that stdout.
pub fn detach() -> io::Result<Role> {
    if std::env::var_os(HOLDER_ENV).is_some() {
        std::env::remove_var(HOLDER_ENV);
        let _ = std::env::set_current_dir(neutral_dir());
        // SAFETY: our stdout is the launcher's pipe; we take it over (and
        // close it after the report) and leave no std handle pointing at it.
        let out = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if out.is_null() || out == INVALID_HANDLE_VALUE {
            return Err(io::Error::other("the holder has no report pipe"));
        }
        unsafe { SetStdHandle(STD_OUTPUT_HANDLE, std::ptr::null_mut()) };
        let report = unsafe { File::from_raw_handle(out as RawHandle) };
        return Ok(Role::Holder(Box::new(report)));
    }
    let (reader, writer) = io::pipe()?;
    let exe = std::env::current_exe()?;
    let spawn = |flags: u32| -> io::Result<()> {
        let mut cmd = Command::new(&exe);
        cmd.args(std::env::args_os().skip(1))
            .env(HOLDER_ENV, "1")
            .current_dir(neutral_dir())
            .stdin(Stdio::null())
            .stdout(writer.try_clone()?)
            .stderr(Stdio::null())
            .creation_flags(flags);
        cmd.spawn().map(drop)
    };
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    // A job that doesn't allow breaking away (some terminals, CI) refuses
    // the flag: then the holder stays in it.
    spawn(flags | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| spawn(flags))?;
    drop(writer);
    Ok(Role::Launcher(Box::new(reader)))
}

/// A console program started from a GUI app would open a console window.
pub fn hide_console(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

// --- PTY -------------------------------------------------------------------

pub struct Pty {
    pub pid: u32,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: PtyControl,
}

/// Resize / hang up / kill / wait for the ConPTY's child. Cheap to clone.
#[derive(Clone)]
pub struct PtyControl {
    inner: Arc<Control>,
}

type Console = (Box<dyn MasterPty + Send>, Box<dyn SlavePty + Send>);

struct Control {
    /// The pseudo console; dropping it closes it (`ClosePseudoConsole`).
    console: Mutex<Option<Console>>,
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: u32,
    exited: AtomicBool,
}

impl PtyControl {
    pub fn resize(&self, cols: u16, rows: u16) {
        if let Some((master, _)) = lock(&self.inner.console).as_ref() {
            let _ = master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    /// Close the pseudo console: its programs get `CTRL_CLOSE_EVENT`, the
    /// Windows counterpart of a hang-up.
    pub fn hangup(&self) {
        let console = lock(&self.inner.console).take();
        drop(console);
    }

    /// End the child and everything it started (each by exact pid).
    pub fn kill(&self) {
        if self.inner.exited.load(Ordering::SeqCst) {
            return;
        }
        for pid in descendants(self.inner.pid).into_iter().rev() {
            force_kill(pid);
        }
        let _ = lock(&self.inner.killer).kill();
    }

    /// Block until the child exits; its exit code.
    pub fn wait(&self) -> i32 {
        let child = lock(&self.inner.child).take();
        let code = match child {
            Some(mut c) => c.wait().map(|s| s.exit_code() as i32).unwrap_or(-1),
            None => -1,
        };
        self.inner.exited.store(true, Ordering::SeqCst);
        // ConPTY keeps the output pipe open until the console is closed:
        // close it so the reader sees the end.
        self.hangup();
        code
    }
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Start the program in a new ConPTY.
pub fn spawn_pty(spec: &PtySpec) -> io::Result<Pty> {
    let pair = portable_pty::native_pty_system()
        .openpty(PtySize { rows: spec.rows, cols: spec.cols, pixel_width: 0, pixel_height: 0 })
        .map_err(other)?;
    let mut cmd = CommandBuilder::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.cwd(cwd);
    }
    let child = pair.slave.spawn_command(cmd).map_err(other)?;
    let pid = child.process_id().unwrap_or(0);
    let reader = pair.master.try_clone_reader().map_err(other)?;
    let writer = pair.master.take_writer().map_err(other)?;
    let killer = child.clone_killer();
    Ok(Pty {
        pid,
        reader,
        writer,
        control: PtyControl {
            inner: Arc::new(Control {
                console: Mutex::new(Some((pair.master, pair.slave))),
                child: Mutex::new(Some(child)),
                killer: Mutex::new(killer),
                pid,
                exited: AtomicBool::new(false),
            }),
        },
    })
}

// --- Processes -------------------------------------------------------------

/// `(pid, parent pid)` of every process (Toolhelp snapshot).
fn process_parents() -> Vec<(u32, u32)> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let snap = Handle(snap);
    let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut out = Vec::new();
    let mut ok = unsafe { Process32FirstW(snap.0, &mut entry) } != 0;
    while ok {
        out.push((entry.th32ProcessID, entry.th32ParentProcessID));
        ok = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    out
}

/// Every process started (directly or not) by `root`, parents before children.
fn descendants(root: u32) -> Vec<u32> {
    let table = process_parents();
    let mut out = Vec::new();
    let mut todo = vec![root];
    while let Some(p) = todo.pop() {
        for &(pid, ppid) in &table {
            if ppid == p && pid != p && pid != root && !out.contains(&pid) {
                out.push(pid);
                todo.push(pid);
            }
        }
    }
    out
}

/// Whether a process with this exact pid is running.
pub fn alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        // Exists but isn't ours to query (another user's): still alive.
        return io::Error::last_os_error().raw_os_error() == Some(5);
    }
    let h = Handle(h);
    let mut code = 0u32;
    unsafe { GetExitCodeProcess(h.0, &mut code) != 0 && code == STILL_ACTIVE as u32 }
}

/// Terminate one exact pid (never a group or a name).
pub fn force_kill(pid: u32) {
    // 0: the idle process, 4: System.
    if pid <= 4 || pid == std::process::id() {
        return;
    }
    let h = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if !h.is_null() {
        let h = Handle(h);
        unsafe { TerminateProcess(h.0, 1) };
    }
}
