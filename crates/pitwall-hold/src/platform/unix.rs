//! Unix (macOS, Linux): socket files, fork + setsid, forkpty, signals.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use interprocess::local_socket::{GenericFilePath, Name, ToFsName};

use super::PtySpec;

fn errno() -> io::Error {
    io::Error::last_os_error()
}

fn set_cloexec(fd: RawFd) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
    }
}

// --- Endpoint --------------------------------------------------------------

/// The socket file itself.
pub fn endpoint_name(path: &Path) -> io::Result<Name<'_>> {
    path.to_fs_name::<GenericFilePath>()
}

/// Create the socket's directory, private to the user (0700).
pub fn prepare_endpoint(path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Socket file only the user can connect to (0600).
pub fn secure_endpoint(path: &Path) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// A socket file nobody listens on any more (or whose holder only lingers).
pub fn remove_stale_endpoint(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Identity of the endpoint we bound (the inode), so cleanup never removes a
/// successor's socket that took over the same path.
pub fn endpoint_token(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.ino())
}

pub fn remove_endpoint(path: &Path, token: Option<u64>) {
    if token.is_some() && endpoint_token(path) == token {
        let _ = std::fs::remove_file(path);
    }
}

// --- Detaching -------------------------------------------------------------

pub enum Role {
    /// The process that was started: read the holder's one-line report.
    Launcher(File),
    /// The detached holder: write the report, then serve.
    Holder(File),
}

/// Fork; the child leaves the caller's session (`setsid`), ignores SIGHUP,
/// moves to `/` and points stdio at /dev/null. Call before any thread exists.
pub fn detach() -> io::Result<Role> {
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(errno());
    }
    set_cloexec(fds[0]);
    set_cloexec(fds[1]);
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(errno());
    }
    if pid > 0 {
        unsafe { libc::close(fds[1]) };
        return Ok(Role::Launcher(unsafe { File::from_raw_fd(fds[0]) }));
    }
    unsafe {
        libc::close(fds[0]);
        libc::setsid();
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        let _ = libc::chdir(c"/".as_ptr());
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            for fd in 0..3 {
                libc::dup2(null, fd);
            }
            if null > 2 {
                libc::close(null);
            }
        }
    }
    Ok(Role::Holder(unsafe { File::from_raw_fd(fds[1]) }))
}

// --- PTY -------------------------------------------------------------------

pub struct Pty {
    pub pid: u32,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: PtyControl,
}

/// Resize / signal / wait for the PTY's child. Cheap to clone.
#[derive(Clone)]
pub struct PtyControl {
    inner: Arc<Control>,
}

struct Control {
    master: File,
    pid: libc::pid_t,
    reaped: AtomicBool,
}

impl PtyControl {
    pub fn resize(&self, cols: u16, rows: u16) {
        let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        unsafe { libc::ioctl(self.inner.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    }

    fn signal(&self, sig: libc::c_int) {
        if !self.inner.reaped.load(Ordering::SeqCst) {
            unsafe { libc::kill(self.inner.pid, sig) };
        }
    }

    /// SIGHUP: the polite way to ask a terminal program to leave.
    pub fn hangup(&self) {
        self.signal(libc::SIGHUP);
    }

    pub fn kill(&self) {
        self.signal(libc::SIGKILL);
    }

    /// Block until the child exits; its exit code, or 128 + signal.
    pub fn wait(&self) -> i32 {
        loop {
            let mut st: libc::c_int = 0;
            let r = unsafe { libc::waitpid(self.inner.pid, &mut st, 0) };
            if r == self.inner.pid {
                if libc::WIFEXITED(st) {
                    self.inner.reaped.store(true, Ordering::SeqCst);
                    return libc::WEXITSTATUS(st);
                }
                if libc::WIFSIGNALED(st) {
                    self.inner.reaped.store(true, Ordering::SeqCst);
                    return 128 + libc::WTERMSIG(st);
                }
                continue; // stopped / continued
            }
            if r < 0 && errno().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            self.inner.reaped.store(true, Ordering::SeqCst);
            return -1;
        }
    }
}

/// Everything the child needs, prepared before `fork` (nothing allocates after).
struct Exec {
    program: CString,
    argv: Vec<CString>,
    cwd: Option<CString>,
}

/// In the forked child: only async-signal-safe calls from here on.
unsafe fn exec_child(e: &Exec, argv: &[*const libc::c_char]) -> ! {
    for sig in [libc::SIGPIPE, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGCHLD] {
        libc::signal(sig, libc::SIG_DFL);
    }
    let mut empty: libc::sigset_t = std::mem::zeroed();
    libc::sigemptyset(&mut empty);
    libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut());
    if let Some(cwd) = &e.cwd {
        if libc::chdir(cwd.as_ptr()) != 0 {
            let msg = b"pitwall-hold: could not enter the working directory\r\n";
            libc::write(2, msg.as_ptr().cast(), msg.len());
            libc::_exit(126);
        }
    }
    libc::execvp(e.program.as_ptr(), argv.as_ptr());
    let msg = b"pitwall-hold: could not start the program\r\n";
    libc::write(2, msg.as_ptr().cast(), msg.len());
    libc::_exit(127);
}

/// Start the program in a new PTY. Call before any thread exists.
pub fn spawn_pty(spec: &PtySpec) -> io::Result<Pty> {
    let c = |s: &[u8]| CString::new(s).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in argument"));
    let program = c(spec.program.as_bytes())?;
    let mut args = vec![program.clone()];
    for a in &spec.args {
        args.push(c(a.as_bytes())?);
    }
    let cwd = spec.cwd.as_ref().map(|p| c(p.as_os_str().as_bytes())).transpose()?;
    let exec = Exec { program, argv: args, cwd };
    let mut argv: Vec<*const libc::c_char> = exec.argv.iter().map(|a| a.as_ptr()).collect();
    argv.push(std::ptr::null());

    let mut ws = libc::winsize { ws_row: spec.rows, ws_col: spec.cols, ws_xpixel: 0, ws_ypixel: 0 };
    let mut master: libc::c_int = -1;
    let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
    if pid < 0 {
        return Err(errno());
    }
    if pid == 0 {
        unsafe { exec_child(&exec, &argv) }
    }
    set_cloexec(master);
    let reader = unsafe { File::from_raw_fd(master) };
    let writer = reader.try_clone()?;
    let control = reader.try_clone()?;
    Ok(Pty {
        pid: pid as u32,
        reader: Box::new(reader),
        writer: Box::new(writer),
        control: PtyControl { inner: Arc::new(Control { master: control, pid, reaped: AtomicBool::new(false) }) },
    })
}

// --- Processes -------------------------------------------------------------

/// Whether a process with this exact pid exists.
pub fn alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || errno().raw_os_error() == Some(libc::EPERM)
}

/// SIGKILL one exact pid (never a group).
pub fn force_kill(pid: u32) {
    if pid > 1 && pid <= i32::MAX as u32 {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
}
