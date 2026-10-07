//! Running programs through the user's login shell ([`LoginShell`],
//! architecture.md §9 decision 7). Apps started from the Dock or a desktop
//! launcher don't inherit the terminal's PATH, so agents are launched through the user's
//! login + interactive shell; on Windows that shell is PowerShell, which also
//! picks up PATH changes made since Pitwall started. Programs run through
//! [`LocalExec`]: this is about this machine.
//!
//! The shell is `$PITWALL_SHELL` when set, else the platform's default
//! (`$SHELL`, else zsh on macOS and bash on Linux; `pwsh` if installed, else Windows PowerShell). The
//! command-line syntax follows the shell's [`Flavor`].

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use crate::exec::{Cmd, Exec, LocalExec};
use crate::platform;

/// A login shell that hangs (waiting on something in the user's rc files).
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(60);

/// Asking the login shell for its PATH (in the background in the app): an rc
/// file that hangs leaves the fallback PATH in place.
pub const PATH_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Marks the PATH line in a login shell's output (rc files may print too).
const PATH_MARK: &str = "__PITWALL_PATH__";

/// Overrides the login shell (a program path or name; its syntax is taken
/// from its file name: `pwsh`/`powershell`, `cmd`, else POSIX sh).
pub const SHELL_ENV: &str = "PITWALL_SHELL";

/// PowerShell: PATH as the system and the user have it now (Machine + User,
/// as a new terminal would see it), then what Pitwall was started with.
const PS_FRESH_PATH: &str = "$env:Path = (@([Environment]::GetEnvironmentVariable('Path','Machine'), [Environment]::GetEnvironmentVariable('Path','User'), $env:Path) | Where-Object { $_ }) -join ';'";

/// Command-line syntax of a shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// sh, bash, zsh, fish (as far as `-l -i -c` and quoting go).
    Posix,
    /// PowerShell 7 (`pwsh`) or Windows PowerShell.
    PowerShell,
    /// `cmd.exe`.
    Cmd,
}

impl Flavor {
    /// From the shell program's file name.
    pub fn of(program: &str) -> Flavor {
        let name = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
        match name.strip_suffix(".exe").unwrap_or(&name) {
            "pwsh" | "powershell" => Flavor::PowerShell,
            "cmd" => Flavor::Cmd,
            _ => Flavor::Posix,
        }
    }
}

/// The shell agents and probes run in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginShell {
    pub program: String,
    pub flavor: Flavor,
}

impl LoginShell {
    pub fn new(program: impl Into<String>) -> LoginShell {
        let program = program.into();
        LoginShell { flavor: Flavor::of(&program), program }
    }

    /// `$PITWALL_SHELL`, else the platform's default.
    pub fn current() -> LoginShell {
        match std::env::var(SHELL_ENV) {
            Ok(p) if !p.trim().is_empty() => LoginShell::new(p.trim()),
            _ => LoginShell::new(platform::default_login_shell()),
        }
    }

    /// One argument, quoted for this shell when it needs it.
    pub fn quote(&self, arg: &str) -> String {
        match self.flavor {
            Flavor::Posix => posix_quote(arg),
            Flavor::PowerShell => {
                if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:+\\".contains(c)) {
                    arg.to_string()
                } else {
                    format!("'{}'", arg.replace('\'', "''"))
                }
            }
            Flavor::Cmd => {
                if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:+\\".contains(c)) {
                    arg.to_string()
                } else {
                    format!("\"{}\"", arg.replace('"', "\"\""))
                }
            }
        }
    }

    /// `$SHELL` in a kind's command (the shell kind) means this shell.
    fn expand_self(&self, command_line: &str) -> String {
        let me = match self.flavor {
            Flavor::PowerShell => format!("& {} -NoLogo", self.quote(&self.program)),
            _ => self.quote(&self.program),
        };
        replace_word(command_line, "$SHELL", &me)
    }

    /// (program, args) that run `command_line` as the terminal's program, with
    /// the user's environment. POSIX: `$SHELL -l -i -c 'exec <line>'` (the
    /// agent replaces the shell); PowerShell: the line after a PATH refresh,
    /// or the interactive shell itself for `$SHELL`.
    pub fn launch(&self, command_line: &str) -> (String, Vec<String>) {
        let line = command_line.trim();
        match self.flavor {
            Flavor::Posix => self.run(&format!("exec {line}")),
            Flavor::PowerShell if line == "$SHELL" => {
                let args = ["-NoLogo", "-NoExit", "-Command", PS_FRESH_PATH];
                (self.program.clone(), args.map(String::from).to_vec())
            }
            Flavor::PowerShell => {
                let line = self.expand_self(line);
                // A first word in quotes is a string, not a command, without `&`.
                let call = if line.starts_with(['\'', '"']) { format!("& {line}") } else { line };
                let args = ["-NoLogo", "-Command", &format!("{PS_FRESH_PATH}; {call}")];
                (self.program.clone(), args.map(String::from).to_vec())
            }
            Flavor::Cmd if line == "$SHELL" => (self.program.clone(), vec!["/D".into()]),
            Flavor::Cmd => (self.program.clone(), vec!["/D".into(), "/S".into(), "/C".into(), format!("\"{}\"", self.expand_self(line))]),
        }
    }

    /// (program, args) that run `script` and return (probes: PATH, versions).
    pub fn run(&self, script: &str) -> (String, Vec<String>) {
        let args: Vec<String> = match self.flavor {
            Flavor::Posix => vec!["-l".into(), "-i".into(), "-c".into(), script.into()],
            Flavor::PowerShell => vec!["-NoLogo".into(), "-NonInteractive".into(), "-Command".into(), format!("{PS_FRESH_PATH}; {script}")],
            Flavor::Cmd => vec!["/D".into(), "/S".into(), "/C".into(), format!("\"{script}\"")],
        };
        (self.program.clone(), args)
    }

    /// Run `first`, then `then` (a terminal that resumes its agent, then is a
    /// shell again: terminals.md §3).
    pub fn then(&self, first: &str, then: &str) -> String {
        match self.flavor {
            Flavor::Posix => format!("{first}; exec {then}"),
            Flavor::PowerShell => format!("{first}; {then}"),
            Flavor::Cmd => format!("{first} & {then}"),
        }
    }

    /// A script that runs `program` (a path) with `args` (already quoted),
    /// replacing the shell where it can.
    pub fn invoke(&self, program: &str, args: &str) -> String {
        let p = self.quote(program);
        match self.flavor {
            Flavor::Posix => format!("exec {p} {args}"),
            Flavor::PowerShell => format!("& {p} {args}"),
            Flavor::Cmd => format!("{p} {args}"),
        }
    }

    /// A script that prints `mark` followed by PATH on a line of its own.
    pub fn print_path(&self, mark: &str) -> String {
        match self.flavor {
            Flavor::Posix => format!("printf '\\n{mark}%s\\n' \"$PATH\""),
            Flavor::PowerShell => format!("Write-Output ''; Write-Output ('{mark}' + $env:Path)"),
            Flavor::Cmd => format!("echo.& echo {mark}%PATH%"),
        }
    }

    /// Whether a JSON argument (`claude --settings '{…}'`) reaches the
    /// program intact. PowerShell's argument passing to native programs
    /// strips embedded double quotes in some versions, so it gets a file.
    pub fn passes_json(&self) -> bool {
        self.flavor == Flavor::Posix
    }

    /// Run `script` in this shell, time-boxed: its stdout.
    pub fn output(&self, script: &str, timeout: Duration) -> Option<String> {
        let (program, args) = self.run(script);
        let mut argv = vec![program.as_str()];
        argv.extend(args.iter().map(String::as_str));
        crate::exec::local_stdout(&argv, timeout)
    }
}

/// `word` replaced where it stands alone (not followed by a name character).
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(word) {
        let after = &rest[at + word.len()..];
        out.push_str(&rest[..at]);
        if after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            out.push_str(word);
        } else {
            out.push_str(with);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The login shell program (`$SHELL` on Unix).
pub fn user_shell() -> String {
    LoginShell::current().program
}

/// How agents are launched, as (program, args): `$SHELL -l -i -c 'exec <line>'`
/// on Unix (see [`LoginShell::launch`]).
pub fn launch_invocation(command_line: &str) -> (String, Vec<String>) {
    LoginShell::current().launch(command_line)
}

/// The PATH for everything Pitwall starts, once the app set one
/// ([`adopt_login_path`]); `None`: this process's own (the CLI and the
/// daemon, which have a terminal's PATH).
static SPAWN_PATH: RwLock<Option<String>> = RwLock::new(None);

/// The PATH to give programs Pitwall starts ([`LocalExec`], holders), when
/// it differs from this process's own.
pub fn spawn_path() -> Option<String> {
    SPAWN_PATH.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Set the PATH for programs Pitwall starts from now on. The process
/// environment itself is never changed: other threads may be reading it.
pub fn set_spawn_path(path: String) {
    *SPAWN_PATH.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// PATH as the user's terminal has it, resolved once: the registry's on
/// Windows (system + user, what PowerShell's login PATH is too); elsewhere
/// the login shell's (see [`resolve_path`]). Blocks while the shell starts;
/// the app asks it in the background ([`adopt_login_path`]).
pub fn login_env_path() -> String {
    probed_login_path().unwrap_or_else(|| {
        let current = std::env::var("PATH").ok();
        fallback_path(None, current.as_deref(), &platform::home_dir())
    })
}

/// [`login_env_path`] as the shell answered it; `None` when it couldn't.
fn probed_login_path() -> Option<String> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        platform::system_path().or_else(|| {
            let current = std::env::var("PATH").ok();
            let login = probe_path(&LoginShell::current(), PATH_PROBE_TIMEOUT)?;
            Some(merge_paths(vec![split(&login), current.as_deref().map(split).unwrap_or_default()]))
        })
    })
    .clone()
}

/// Give everything the desktop app starts (git, agw and what agw starts,
/// rulesync, holders, …) the PATH the user's terminal has: apps opened from
/// the Dock, Finder or a desktop launcher get a minimal one
/// (`/usr/bin:/bin:/usr/sbin:/sbin` on macOS).
///
/// Never blocks start-up: the spawn PATH is at once the last login PATH
/// (`cache`, a file in Pitwall's data folder) or, the first time, this
/// process's PATH plus the common program dirs that exist (Homebrew,
/// `~/.local/bin`, `~/.cargo/bin`). The login shell is then asked on a
/// thread of its own; its answer replaces the spawn PATH and the cache. On
/// Windows the registry's PATH is read at once, with no shell.
///
/// The returned thread is the background probe (tests wait for it).
pub fn adopt_login_path(cache: Option<PathBuf>) -> Option<std::thread::JoinHandle<()>> {
    if let Some(path) = platform::system_path() {
        set_spawn_path(path);
        return None;
    }
    let current = std::env::var("PATH").ok();
    let cached = cache.as_deref().and_then(|f| std::fs::read_to_string(f).ok()).filter(|p| !p.trim().is_empty());
    set_spawn_path(fallback_path(cached.as_deref().map(str::trim), current.as_deref(), &platform::home_dir()));
    std::thread::Builder::new()
        .name("login-path".into())
        .spawn(move || {
            let Some(path) = probed_login_path() else { return };
            set_spawn_path(path.clone());
            if let Some(file) = cache.filter(|_| cached.as_deref().map(str::trim) != Some(path.as_str())) {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(file, &path);
            }
        })
        .ok()
}

/// PATH without asking the shell: `cached` (the last login PATH), then
/// `current`, then those of [`platform::common_bin_dirs`] that exist.
pub fn fallback_path(cached: Option<&str>, current: Option<&str>, home: &Path) -> String {
    let common: Vec<PathBuf> = platform::common_bin_dirs(home).into_iter().filter(|d| d.is_dir()).collect();
    merge_paths(vec![cached.map(split).unwrap_or_default(), current.map(split).unwrap_or_default(), common])
}

/// The login shell's PATH (`shell` asked to print it, within `timeout`), with
/// the dirs of `current` it lacks after it; [`fallback_path`] when the shell
/// can't be asked (missing, failing, hanging, printing nothing).
pub fn resolve_path(shell: &LoginShell, current: Option<&str>, home: &Path, timeout: Duration) -> String {
    match probe_path(shell, timeout) {
        Some(login) => merge_paths(vec![split(&login), current.map(split).unwrap_or_default()]),
        None => fallback_path(None, current, home),
    }
}

/// What `shell` (login + interactive) says PATH is, started with this
/// process's own PATH (not a spawn PATH it would only add to).
pub fn probe_path(shell: &LoginShell, timeout: Duration) -> Option<String> {
    let (program, args) = shell.run(&shell.print_path(PATH_MARK));
    let mut argv = vec![program.as_str()];
    argv.extend(args.iter().map(String::as_str));
    let own = std::env::var("PATH").unwrap_or_default();
    let out = LocalExec.run(&Cmd::new(&argv).env(&[("PATH", &own)]).timeout(timeout)).ok()?;
    out.stdout_text()
        .lines()
        .rev()
        .find_map(|l| l.trim_end_matches('\r').strip_prefix(PATH_MARK))
        .map(str::to_string)
        .filter(|p| !p.trim().is_empty())
}

fn split(path: &str) -> Vec<PathBuf> {
    std::env::split_paths(path).collect()
}

/// The dirs in order, empty ones and repeats left out, joined as PATH.
fn merge_paths(parts: Vec<Vec<PathBuf>>) -> String {
    let mut seen: Vec<PathBuf> = Vec::new();
    for dir in parts.into_iter().flatten() {
        if !dir.as_os_str().is_empty() && !seen.contains(&dir) {
            seen.push(dir);
        }
    }
    std::env::join_paths(seen).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

fn posix_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// One argument quoted for this machine's login shell.
pub fn quote(arg: &str) -> String {
    LoginShell::current().quote(arg)
}

/// Where `program` is installed, as the user's shell resolves it (POSIX:
/// `command -v` in the login shell; Windows: `where`, with PATH as a new
/// terminal would have it).
pub fn which(program: &str) -> Option<String> {
    let shell = LoginShell::current();
    if shell.flavor != Flavor::Posix {
        return platform::which(program);
    }
    let script = format!("command -v {}", posix_quote(program));
    let out = LocalExec
        .run(&Cmd::new(&[&shell.program, "-l", "-i", "-c", &script]).timeout(LOGIN_SHELL_TIMEOUT))
        .ok()?;
    if !out.ok() {
        return None;
    }
    out.stdout_text()
        .lines()
        .map(str::trim)
        .rfind(|l| l.starts_with('/'))
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flavors_from_program_names() {
        assert_eq!(Flavor::of("/bin/zsh"), Flavor::Posix);
        assert_eq!(Flavor::of("bash"), Flavor::Posix);
        assert_eq!(Flavor::of(r"C:\Program Files\PowerShell\7\pwsh.exe"), Flavor::PowerShell);
        assert_eq!(Flavor::of(r"C:\Windows\System32\WindowsPowerShell\v1.0\PowerShell.EXE"), Flavor::PowerShell);
        assert_eq!(Flavor::of("cmd.exe"), Flavor::Cmd);
    }

    #[test]
    fn posix_is_unchanged() {
        let sh = LoginShell::new("/bin/zsh");
        assert_eq!(sh.quote("abc"), "abc");
        assert_eq!(sh.quote("it's"), r"'it'\''s'");
        assert_eq!(sh.launch("claude --resume x"), ("/bin/zsh".into(), vec!["-l".into(), "-i".into(), "-c".into(), "exec claude --resume x".into()]));
        assert_eq!(sh.then("claude", "$SHELL"), "claude; exec $SHELL");
        assert!(sh.passes_json());
    }

    #[test]
    fn powershell_quoting_and_launch() {
        let ps = LoginShell::new(r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_eq!(ps.quote("--session-id"), "--session-id");
        assert_eq!(ps.quote(r"C:\Users\dev\x.exe"), r"C:\Users\dev\x.exe");
        assert_eq!(ps.quote("it's here"), "'it''s here'");
        assert_eq!(ps.quote("@splat"), "'@splat'");
        assert_eq!(ps.quote("a,b"), "'a,b'");
        assert_eq!(ps.quote(""), "''");
        let (prog, args) = ps.launch("claude --resume abc");
        assert_eq!(prog, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_eq!(&args[..2], ["-NoLogo", "-Command"]);
        assert!(args[2].starts_with("$env:Path = ") && args[2].ends_with("; claude --resume abc"), "{}", args[2]);
        // A quoted program needs the call operator.
        let (_, args) = ps.launch("'C:\\a b\\tool.exe' --x");
        assert!(args[2].ends_with("; & 'C:\\a b\\tool.exe' --x"), "{}", args[2]);
        // The shell kind: the interactive shell itself, no nesting.
        let (_, args) = ps.launch("$SHELL");
        assert_eq!(&args[..3], ["-NoLogo", "-NoExit", "-Command"]);
        // A terminal resuming its agent, then a shell again.
        let line = ps.then("claude --resume abc", "$SHELL");
        let (_, args) = ps.launch(&line);
        assert!(args[2].ends_with("; claude --resume abc; & 'C:\\Program Files\\PowerShell\\7\\pwsh.exe' -NoLogo"), "{}", args[2]);
        assert!(!ps.passes_json());
        assert!(ps.print_path("MARK").contains("'MARK' + $env:Path"));
    }

    #[test]
    fn paths_merge_in_order_without_repeats_or_empties() {
        let p = |s: &str| PathBuf::from(s);
        let merged = merge_paths(vec![vec![p("/a"), p(""), p("/b")], vec![p("/b"), p("/c"), p("/a")]]);
        assert_eq!(split(&merged), vec![p("/a"), p("/b"), p("/c")]);
    }

    #[test]
    fn a_shell_that_cannot_run_falls_back_to_current_and_existing_common_dirs() {
        let home = crate::testing::TempDir::new("login-path-home");
        let common = platform::common_bin_dirs(home.path());
        let mine: Vec<&PathBuf> = common.iter().filter(|d| d.starts_with(home.path())).collect();
        std::fs::create_dir_all(mine[0]).unwrap();
        let sh = LoginShell::new("/no/such/shell-for-pitwall");
        let path = resolve_path(&sh, Some("/usr/bin"), home.path(), Duration::from_secs(5));
        let dirs = split(&path);
        assert_eq!(dirs[0], PathBuf::from("/usr/bin"), "{path}");
        assert!(dirs.contains(mine[0]), "{path}");
        assert!(mine[1..].iter().all(|d| !dirs.contains(d)), "missing dirs are left out: {path}");
        // The last login PATH first, when there is one.
        let quick = split(&fallback_path(Some("/cached/bin"), Some("/usr/bin"), home.path()));
        assert_eq!(&quick[..2], [PathBuf::from("/cached/bin"), PathBuf::from("/usr/bin")]);
    }

    #[test]
    fn shell_variable_is_replaced_as_a_word_only() {
        assert_eq!(replace_word("$SHELL; $SHELLX $SHELL", "$SHELL", "sh"), "sh; $SHELLX sh");
    }
}
