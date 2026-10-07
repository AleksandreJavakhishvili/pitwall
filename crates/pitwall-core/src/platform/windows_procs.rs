//! Windows process queries for recognising agents (docs/spec/terminals.md)
//! and finding where they work. They return text in the same shapes as the
//! Unix `ps` / `lsof` queries, so `crate::procs` parses both:
//!
//! - `process_table`: `pid ppid args` rows (Toolhelp snapshot + each
//!   process's command line);
//! - `terminal_groups`: `pid foreground` rows. Windows has no terminal
//!   process groups: a ConPTY's foreground is what its shell started last
//!   (following nested shells down), or the shell itself at its prompt;
//! - `process_args`: `pid args` rows;
//! - `process_cwds`: read from each process's parameters block (best effort,
//!   64-bit processes of this user).
//!
//! In `args` the program is its executable's file name (`claude.exe`), not
//! its full path, so paths with spaces don't split it.
//!
//! Reading another process's command line with
//! `ProcessCommandLineInformation`, and its current directory from the PEB,
//! follows the approach of Herdr's Windows platform layer (Apache-2.0, see
//! NOTICE); the code is Pitwall's own.

use std::ffi::c_void;
use std::time::Duration;

use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation, ProcessCommandLineInformation};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};
use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

/// Console hosts ConPTY starts next to the shell; never the foreground.
const CONSOLE_HOSTS: &[&str] = &["conhost.exe", "openconsole.exe"];
/// Shells: the foreground search goes on below one of these.
const SHELLS: &[&str] = &["pwsh.exe", "powershell.exe", "cmd.exe", "bash.exe", "sh.exe", "zsh.exe", "fish.exe", "nu.exe"];

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn open(pid: u32, access: u32) -> Option<Handle> {
    let h = unsafe { OpenProcess(access, 0, pid) };
    (!h.is_null()).then_some(Handle(h))
}

/// One row of the Toolhelp snapshot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Proc {
    pub pid: u32,
    pub ppid: u32,
    /// Executable file name (`pwsh.exe`).
    pub exe: String,
}

fn snapshot() -> Vec<Proc> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let snap = Handle(snap);
    let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut out = Vec::new();
    let mut ok = unsafe { Process32FirstW(snap.0, &mut e) } != 0;
    while ok {
        let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        out.push(Proc { pid: e.th32ProcessID, ppid: e.th32ParentProcessID, exe: String::from_utf16_lossy(&e.szExeFile[..len]) });
        ok = unsafe { Process32NextW(snap.0, &mut e) } != 0;
    }
    out
}

/// The command line of `pid` (needs only limited query access).
fn command_line(pid: u32) -> Option<String> {
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut needed = 0u32;
    unsafe { NtQueryInformationProcess(h.0, ProcessCommandLineInformation, std::ptr::null_mut(), 0, &mut needed) };
    let header = std::mem::size_of::<UnicodeString>();
    if (needed as usize) < header {
        return None;
    }
    // Room for a command line that grows between the two calls.
    let mut buf = vec![0u8; needed as usize + 512];
    let status = unsafe { NtQueryInformationProcess(h.0, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), buf.len() as u32, &mut needed) };
    if status < 0 {
        return None;
    }
    // SAFETY: on success the buffer starts with a UNICODE_STRING whose
    // characters follow it in the same buffer.
    let us = unsafe { buf.as_ptr().cast::<UnicodeString>().read_unaligned() };
    let len = usize::from(us.length);
    if len == 0 || len % 2 != 0 || header + len > buf.len() {
        return None;
    }
    let units: Vec<u16> = buf[header..header + len].chunks_exact(2).map(|c| u16::from_ne_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units))
}

/// Windows command-line splitting (`CommandLineToArgvW`).
fn split_args(line: &str) -> Vec<String> {
    let wide: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
    let mut argc = 0i32;
    let argv = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut argc) };
    if argv.is_null() || argc <= 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(argc as usize);
    for i in 0..argc as usize {
        let p = unsafe { *argv.add(i) };
        let mut n = 0;
        while unsafe { *p.add(n) } != 0 {
            n += 1;
        }
        out.push(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) }));
    }
    unsafe { LocalFree(argv.cast()) };
    out
}

/// `args` for a process: its executable's name, then its arguments.
pub(crate) fn render(exe: &str, argv: &[String]) -> String {
    let mut out = exe.to_string();
    for a in argv.iter().skip(1) {
        out.push(' ');
        out.push_str(a);
    }
    out
}

fn args_of(p: &Proc) -> String {
    match command_line(p.pid) {
        Some(line) => render(&p.exe, &split_args(&line)),
        None => p.exe.clone(),
    }
}

/// `pid ppid args` for every process.
pub fn process_table() -> Option<String> {
    let procs = snapshot();
    if procs.is_empty() {
        return None;
    }
    Some(procs.iter().filter(|p| p.pid != 0).map(|p| format!("{} {} {}\n", p.pid, p.ppid, args_of(p))).collect())
}

/// `pid args` for each of `pids` that exists.
pub fn process_args(pids: &[u32]) -> Option<String> {
    if pids.is_empty() {
        return Some(String::new());
    }
    let procs = snapshot();
    Some(procs.iter().filter(|p| pids.contains(&p.pid)).map(|p| format!("{} {}\n", p.pid, args_of(p))).collect())
}

/// When `pid` started (100 ns ticks), for "started last".
fn started(pid: u32) -> u64 {
    let Some(h) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else { return 0 };
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (zero, zero, zero, zero);
    if unsafe { GetProcessTimes(h.0, &mut c, &mut e, &mut k, &mut u) } == 0 {
        return 0;
    }
    (u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime)
}

/// The foreground of the terminal whose shell is `shell`: the newest child
/// (console hosts aside), following nested shells down; the shell itself
/// when it has none. `started` gives creation times.
pub(crate) fn foreground(shell: u32, procs: &[Proc], started: &dyn Fn(u32) -> u64) -> u32 {
    let mut fg = shell;
    for _ in 0..16 {
        let child = procs
            .iter()
            .filter(|p| p.ppid == fg && p.pid != fg && !CONSOLE_HOSTS.contains(&p.exe.to_ascii_lowercase().as_str()))
            .max_by_key(|p| started(p.pid));
        let Some(child) = child else { break };
        fg = child.pid;
        if !SHELLS.contains(&child.exe.to_ascii_lowercase().as_str()) {
            break;
        }
    }
    fg
}

/// `pid foreground` per pid (see the module docs).
pub fn terminal_groups(pids: &[u32]) -> Option<String> {
    if pids.is_empty() {
        return Some(String::new());
    }
    let procs = snapshot();
    let mut out = String::new();
    for &pid in pids.iter().filter(|p| procs.iter().any(|q| q.pid == **p)) {
        out.push_str(&format!("{pid} {}\n", foreground(pid, &procs, &started)));
    }
    Some(out)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct BasicInfo {
    exit_status: i32,
    peb: *mut c_void,
    affinity_mask: usize,
    base_priority: i32,
    unique_pid: usize,
    parent_pid: usize,
}

fn read<T: Copy>(h: &Handle, addr: usize) -> Option<T> {
    let mut v = std::mem::MaybeUninit::<T>::uninit();
    let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(h.0, addr as *const c_void, v.as_mut_ptr().cast(), std::mem::size_of::<T>(), &mut got) };
    (ok != 0 && got == std::mem::size_of::<T>()).then(|| unsafe { v.assume_init() })
}

/// Current folder of `pid`: PEB → process parameters → `CurrentDirectory`
/// (x64 layout; `None` where it can't be read).
fn cwd_of(pid: u32) -> Option<String> {
    if !cfg!(target_pointer_width = "64") {
        return None;
    }
    // Offsets in the 64-bit PEB and RTL_USER_PROCESS_PARAMETERS.
    const PEB_PARAMETERS: usize = 0x20;
    const PARAMETERS_CURRENT_DIRECTORY: usize = 0x38;
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ)?;
    let mut info = std::mem::MaybeUninit::<BasicInfo>::uninit();
    let status = unsafe {
        NtQueryInformationProcess(h.0, ProcessBasicInformation, info.as_mut_ptr().cast(), std::mem::size_of::<BasicInfo>() as u32, std::ptr::null_mut())
    };
    if status < 0 {
        return None;
    }
    let peb = unsafe { info.assume_init() }.peb as usize;
    if peb == 0 {
        return None;
    }
    let params: usize = read(&h, peb + PEB_PARAMETERS)?;
    if params == 0 {
        return None;
    }
    let dir: UnicodeString = read(&h, params + PARAMETERS_CURRENT_DIRECTORY)?;
    let len = usize::from(dir.length) / 2;
    if dir.buffer.is_null() || len == 0 || len > 32_768 {
        return None;
    }
    let mut buf = vec![0u16; len];
    let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(h.0, dir.buffer as *const c_void, buf.as_mut_ptr().cast(), len * 2, &mut got) };
    if ok == 0 || got != len * 2 {
        return None;
    }
    Some(trim_dir(&String::from_utf16_lossy(&buf)))
}

/// `C:\work\app\` → `C:\work\app`; a drive root keeps its backslash.
pub(crate) fn trim_dir(dir: &str) -> String {
    if dir.len() > 3 {
        dir.trim_end_matches('\\').to_string()
    } else {
        dir.to_string()
    }
}

/// `(pid, cwd)` of each process in `pids` whose folder could be read.
pub fn process_cwds(pids: &[u32], _timeout: Duration) -> Option<Vec<(u32, String)>> {
    Some(pids.iter().filter_map(|&p| cwd_of(p).map(|c| (p, c))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::{parse_pid_args, parse_pid_numbers, parse_table};

    fn p(pid: u32, ppid: u32, exe: &str) -> Proc {
        Proc { pid, ppid, exe: exe.into() }
    }

    #[test]
    fn foreground_follows_nested_shells_to_the_newest_program() {
        let procs = vec![
            p(10, 1, "pitwall-hold.exe"),
            p(11, 10, "pwsh.exe"),
            p(12, 11, "conhost.exe"),
            p(13, 11, "pwsh.exe"),
            p(14, 13, "claude.exe"),
            p(15, 14, "node.exe"),
            p(16, 13, "git.exe"),
        ];
        let started = |pid: u32| u64::from(pid);
        assert_eq!(foreground(11, &procs, &started), 16, "newest child of the nested shell");
        let at_prompt = vec![p(11, 10, "pwsh.exe"), p(12, 11, "conhost.exe")];
        assert_eq!(foreground(11, &at_prompt, &started), 11);
        let agent = vec![p(11, 10, "pwsh.exe"), p(14, 11, "claude.exe"), p(15, 14, "node.exe")];
        assert_eq!(foreground(11, &agent, &started), 14, "an agent's own children don't count");
    }

    #[test]
    fn rows_parse_like_ps() {
        let argv = ["C:\\Program Files\\nodejs\\node.exe".to_string(), "--resume".into(), "abc".into()];
        assert_eq!(render("claude.exe", &argv), "claude.exe --resume abc");
        let table = format!("{} {} {}\n", 14, 11, render("claude.exe", &argv));
        assert_eq!(parse_table(&table)[0].args, "claude.exe --resume abc");
        assert_eq!(parse_pid_args("14 claude.exe --resume abc\n")[&14], "claude.exe --resume abc");
        assert_eq!(parse_pid_numbers("11 14\n")[&11], 14);
        assert_eq!(trim_dir("C:\\work\\app\\"), "C:\\work\\app");
        assert_eq!(trim_dir("C:\\"), "C:\\");
    }

    #[test]
    fn reads_this_process() {
        let me = std::process::id();
        let args = parse_pid_args(&process_args(&[me]).unwrap());
        assert!(args[&me].contains(".exe"), "{:?}", args.get(&me));
        assert!(parse_table(&process_table().unwrap()).iter().any(|r| r.pid == me));
        let cwds = process_cwds(&[me], Duration::from_secs(1)).unwrap();
        let here = super::super::canonicalize(&std::env::current_dir().unwrap()).unwrap();
        assert_eq!(cwds.first().map(|(_, c)| c.to_ascii_lowercase()), Some(here.to_string_lossy().to_ascii_lowercase()));
    }
}
