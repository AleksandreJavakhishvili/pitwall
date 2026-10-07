//! [`Exec`] on an agw VM, through agw's own exec commands (step 8b): `agw vm
//! exec [--workspace <ws>] <vm> -- …` as the VM's admin user, or `agw agent
//! exec [--workspace <ws>] <agent> -- …` as an agent's own Linux user, so
//! agw's rules for who may touch what always apply. Never ssh directly.
//!
//! agw passes the remote command on with its arguments quoted, runs it in the
//! user's login shell and hands back its stdout, stderr and exit status
//! (verified with agw 0.19: `exit 3` → 3, a signal → 255, stdin and binary
//! output round-trip). It also exits 1 with `Error: …` on stderr for its own
//! failures (unknown VM or workspace), which looks just like a command
//! failing. So everything runs inside one small `sh -c` script that first
//! prints a start marker and then answers in frames:
//!
//! ```text
//! \x1ePW1\n                                   the script runs on the VM
//! \x1eR <status> <stdout len> <stderr len>\n  one frame per command, then
//! <stdout bytes><stderr bytes>                its output, byte for byte
//! ```
//!
//! No marker: agw (or the link to the VM) failed, and its message is the
//! error. `<status>` is the exit code, `X` when the command couldn't start
//! (missing folder or program: an `Err`, like [`LocalExec`]'s spawn
//! failure), `T` when the VM-side `timeout` ended it, `N` for a file that
//! isn't there. Each agw call costs about a second, so independent commands
//! and file reads share one script ([`Exec::run_all`], [`Exec::read_files`]),
//! and the home and temp folders are asked once.
//!
//! [`LocalExec`]: pitwall_core::exec::LocalExec

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use pitwall_core::exec::{Cmd, Exec, FileKind, Out, Stat};
use pitwall_core::provider::{PwError, Result};

use super::{parse, sq};

/// Reads for the UI (a file for the editor, stat, a path): short.
pub const QUICK: Duration = Duration::from_secs(60);
/// agw's own time (start, ssh) on top of the commands' timeouts.
const SLACK: Duration = Duration::from_secs(60);
/// Commands longer than this get no VM-side `timeout` (none was asked for).
const NO_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 3600);
const START: &[u8] = b"\x1ePW1\n";
const FRAME: &[u8] = b"\x1eR ";

/// Who the commands run as on the VM.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum User {
    /// The VM's admin user (`agw vm exec <vm>`).
    Admin { vm: String },
    /// An agent's own user (`agw agent exec <agent>`).
    Agent { agent: String },
}

pub struct AgwExec {
    agw: String,
    /// Runs agw on this Mac.
    runner: Arc<dyn Exec>,
    user: User,
    /// agw starts commands there (`--workspace`); `None`: the user's home.
    workspace: Option<String>,
    /// (home, temp dir), asked once.
    dirs: OnceLock<(String, String)>,
    /// Time agw gets on top of a call's own timeout.
    slack: Duration,
}

/// How one framed command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    Code(i32),
    CantStart,
    TimedOut,
    Missing,
}

#[derive(Debug, Clone)]
struct Frame {
    ended: Ended,
    out: Vec<u8>,
    err: Vec<u8>,
}

impl Frame {
    fn err_text(&self) -> String {
        String::from_utf8_lossy(&self.err).trim().to_string()
    }

    /// A file operation: its stdout on success, else its error output.
    fn done(self, what: &str) -> Result<Vec<u8>> {
        match self.ended {
            Ended::Code(0) => Ok(self.out),
            Ended::TimedOut => Err(PwError::other(format!("{what}: timed out"))),
            _ => Err(PwError::other(match self.err_text() {
                e if e.is_empty() => format!("{what}: failed"),
                e => e,
            })),
        }
    }
}

const PRELUDE: &str = r#"d=$(mktemp -d "${TMPDIR:-/tmp}/pitwall.XXXXXX") || exit 97
trap 'rm -rf "$d"' EXIT
to=; command -v timeout >/dev/null 2>&1 && to=timeout
emit() { printf '\036R %s %s %s\n' "$1" $(($(wc -c <"$d/o"))) $(($(wc -c <"$d/e"))); cat "$d/o" "$d/e"; }
printf '\036PW1\n'
"#;

/// One framed step of a script: `body` runs in a subshell; it marks "couldn't
/// start" by writing its message to `$d/x` and "no such file" by creating
/// `$d/n`. `timed`: exit 124 from the VM's `timeout` means it timed out.
fn step(body: &str, stdin: bool, timed: bool) -> String {
    let input = if stdin { "" } else { " </dev/null" };
    let timeout = if timed { r#" elif [ -n "$to" ] && [ "$s" = 124 ]; then s=T;"# } else { "" };
    format!(
        "rm -f \"$d/x\" \"$d/n\"; ( {body}\n){input} >\"$d/o\" 2>\"$d/e\"; s=$?\n\
         if [ -e \"$d/x\" ]; then mv -f \"$d/x\" \"$d/e\"; : >\"$d/o\"; s=X; elif [ -e \"$d/n\" ]; then s=N;{timeout} fi\n\
         emit \"$s\"\n"
    )
}

/// The body that runs `cmd` (folder, environment, VM-side timeout).
fn run_body(cmd: &Cmd) -> String {
    let mut b = String::new();
    if let Some(cwd) = cmd.cwd {
        let msg = sq(&format!("{cwd}: No such file or directory"));
        b.push_str(&format!("cd {} 2>/dev/null || {{ printf '%s\\n' {msg} >\"$d/x\"; exit 1; }}; ", sq(cwd)));
    }
    let program = cmd.argv.first().copied().unwrap_or_default();
    let msg = sq(&format!("{program}: No such file or directory"));
    b.push_str(&format!("command -v {} >/dev/null 2>&1 || {{ printf '%s\\n' {msg} >\"$d/x\"; exit 1; }}; ", sq(program)));
    b.push_str("exec ");
    if cmd.timeout < NO_TIMEOUT {
        b.push_str(&format!("${{to:+$to -k 5 {:.3}}} ", cmd.timeout.as_secs_f64()));
    }
    b.push_str("env");
    for (k, v) in cmd.env {
        b.push(' ');
        b.push_str(&sq(&format!("{k}={v}")));
    }
    for a in cmd.argv {
        b.push(' ');
        b.push_str(&sq(a));
    }
    b
}

/// At most `max` bytes of `path`; `N` when it isn't there.
fn read_body(path: &str, max: u64) -> String {
    let p = sq(path);
    format!("[ -e {p} ] || {{ : >\"$d/n\"; exit 0; }}; head -c {max} <{p}")
}

/// The frames after the start marker; `None` without the marker.
fn frames(stdout: &[u8]) -> Option<Vec<Frame>> {
    let at = stdout.windows(START.len()).position(|w| w == START)?;
    let mut rest = &stdout[at + START.len()..];
    let mut res = Vec::new();
    while let Some(after) = rest.strip_prefix(FRAME) {
        let nl = after.iter().position(|&b| b == b'\n')?;
        let head = std::str::from_utf8(&after[..nl]).ok()?;
        let mut f = head.split(' ');
        let ended = match f.next()? {
            "X" => Ended::CantStart,
            "T" => Ended::TimedOut,
            "N" => Ended::Missing,
            n => Ended::Code(n.parse().ok()?),
        };
        let (olen, elen): (usize, usize) = (f.next()?.parse().ok()?, f.next()?.parse().ok()?);
        let body = &after[nl + 1..];
        if body.len() < olen + elen {
            return None;
        }
        res.push(Frame { ended, out: body[..olen].to_vec(), err: body[olen..olen + elen].to_vec() });
        rest = &body[olen + elen..];
    }
    Some(res)
}

impl AgwExec {
    /// `agw`: the agw program; `runner` runs it (this Mac).
    pub fn new(agw: &str, runner: Arc<dyn Exec>, user: User, workspace: Option<&str>) -> AgwExec {
        AgwExec { agw: agw.to_string(), runner, user, workspace: workspace.map(String::from), dirs: OnceLock::new(), slack: SLACK }
    }

    /// Give agw `slack` on top of each call's timeout (default a minute).
    pub fn with_slack(self, slack: Duration) -> AgwExec {
        AgwExec { slack, ..self }
    }

    pub fn user(&self) -> &User {
        &self.user
    }

    pub fn workspace(&self) -> Option<&str> {
        self.workspace.as_deref()
    }

    /// agw's command line for running `script` there: options before the
    /// name, `--` before the remote command.
    pub fn argv(&self, script: &str) -> Vec<String> {
        let mut v = vec![self.agw.clone(), "--non-interactive".into()];
        let (what, name) = match &self.user {
            User::Admin { vm } => ("vm", vm),
            User::Agent { agent } => ("agent", agent),
        };
        v.extend([what.to_string(), "exec".into()]);
        if let Some(ws) = &self.workspace {
            v.extend(["--workspace".into(), ws.clone()]);
        }
        v.extend([name.clone(), "--".into(), "sh".into(), "-c".into(), script.to_string()]);
        v
    }

    /// Run `steps` in one agw call; exactly one frame per step, or the error
    /// agw (or the link) failed with.
    fn call(&self, steps: &[String], stdin: Option<&[u8]>, timeout: Duration) -> Result<Vec<Frame>> {
        let script = format!("{PRELUDE}{}", steps.concat());
        let argv = self.argv(&script);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let mut cmd = Cmd::new(&argv).timeout(timeout.saturating_add(self.slack));
        if let Some(input) = stdin {
            cmd = cmd.stdin(input);
        }
        let out = self.runner.run(&cmd)?;
        match frames(&out.stdout) {
            Some(f) if f.len() == steps.len() => Ok(f),
            _ => Err(self.failed(&out)),
        }
    }

    /// agw didn't run the script (or it was cut off): agw's own message.
    fn failed(&self, out: &Out) -> PwError {
        let msg = match out.stderr_text() {
            e if e.is_empty() => format!("exit status {}", out.status),
            e => parse::error_line(&e),
        };
        let at = match &self.user {
            User::Admin { vm } => vm.clone(),
            User::Agent { agent } => format!("agent {agent}"),
        };
        PwError::unreachable(format!("agw exec on {at}: {msg}"))
    }

    fn one(&self, body: String, stdin: Option<&[u8]>, timeout: Duration) -> Result<Frame> {
        let mut f = self.call(&[step(&body, stdin.is_some(), false)], stdin, timeout)?;
        Ok(f.remove(0))
    }

    /// A file operation's stdout (`what` names it in errors).
    fn file_op(&self, what: &str, body: String, stdin: Option<&[u8]>) -> Result<Vec<u8>> {
        self.one(body, stdin, QUICK)?.done(what)
    }

    fn dirs(&self) -> Result<&(String, String)> {
        if let Some(d) = self.dirs.get() {
            return Ok(d);
        }
        let out = self.file_op("home", r#"printf '%s\n%s' "$HOME" "${TMPDIR:-/tmp}""#.into(), None)?;
        let text = String::from_utf8_lossy(&out).into_owned();
        let (home, tmp) = text.split_once('\n').ok_or_else(|| PwError::other("agw exec: no home folder"))?;
        let tmp = match tmp.trim_end_matches('/') {
            "" => "/".to_string(),
            t => t.to_string(),
        };
        Ok(self.dirs.get_or_init(|| (home.to_string(), tmp)))
    }
}

fn to_out(f: Frame, cmd: &Cmd) -> Result<Out> {
    let program = cmd.argv.first().copied().unwrap_or_default();
    match f.ended {
        Ended::Code(status) => Ok(Out { status, stdout: f.out, stderr: f.err }),
        Ended::CantStart | Ended::Missing => Err(PwError::other(f.err_text())),
        Ended::TimedOut => Err(PwError::other(format!("{program}: timed out after {}s", cmd.timeout.as_secs_f32()))),
    }
}

impl Exec for AgwExec {
    fn run(&self, cmd: &Cmd) -> Result<Out> {
        if cmd.argv.is_empty() {
            return Err(PwError::other("empty command"));
        }
        let f = self.call(&[step(&run_body(cmd), cmd.stdin.is_some(), true)], cmd.stdin, cmd.timeout)?;
        to_out(f.into_iter().next().expect("one frame"), cmd)
    }

    /// One agw call for all of them (each with its own folder, environment
    /// and timeout). Input for more than one command can't share the call's
    /// stdin: those run one by one.
    fn run_all(&self, cmds: &[Cmd]) -> Vec<Result<Out>> {
        let inputs = cmds.iter().filter(|c| c.stdin.is_some()).count();
        if cmds.len() < 2 || inputs > 1 || cmds.iter().any(|c| c.argv.is_empty()) {
            return cmds.iter().map(|c| self.run(c)).collect();
        }
        let stdin = cmds.iter().find_map(|c| c.stdin);
        let steps: Vec<String> = cmds.iter().map(|c| step(&run_body(c), c.stdin.is_some(), true)).collect();
        let timeout = cmds.iter().fold(Duration::ZERO, |t, c| t.saturating_add(c.timeout));
        match self.call(&steps, stdin, timeout) {
            Ok(frames) => frames.into_iter().zip(cmds).map(|(f, c)| to_out(f, c)).collect(),
            Err(e) => cmds.iter().map(|_| Err(e.clone())).collect(),
        }
    }

    fn read_file(&self, path: &str, max: u64) -> Result<Option<Vec<u8>>> {
        self.read_files(&[path], max).pop().unwrap_or_else(|| Err(PwError::other("agw exec: no answer")))
    }

    fn read_files(&self, paths: &[&str], max: u64) -> Vec<Result<Option<Vec<u8>>>> {
        if paths.is_empty() {
            return Vec::new();
        }
        let steps: Vec<String> = paths.iter().map(|p| step(&read_body(p, max), false, false)).collect();
        let timeout = QUICK.saturating_add(Duration::from_millis(100).saturating_mul(paths.len() as u32));
        match self.call(&steps, None, timeout) {
            Ok(frames) => frames
                .into_iter()
                .zip(paths)
                .map(|(f, p)| match f.ended {
                    Ended::Missing => Ok(None),
                    _ => f.done(p).map(Some),
                })
                .collect(),
            Err(e) => paths.iter().map(|_| Err(e.clone())).collect(),
        }
    }

    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let p = sq(path);
        let body = format!("case {p} in */*) mkdir -p -- \"$(dirname -- {p})\" || exit 1;; esac; cat >{p}");
        self.file_op(path, body, Some(bytes)).map(|_| ())
    }

    fn remove_file(&self, path: &str) -> Result<()> {
        self.file_op(path, format!("rm -- {}", sq(path)), None).map(|_| ())
    }

    fn remove_dir(&self, path: &str) -> Result<()> {
        self.file_op(path, format!("rmdir -- {}", sq(path)), None).map(|_| ())
    }

    fn copy_file(&self, from: &str, to: &str) -> Result<()> {
        self.file_op(from, format!("cp -- {} {}", sq(from), sq(to)), None).map(|_| ())
    }

    fn stat(&self, path: &str) -> Result<Option<Stat>> {
        let p = sq(path);
        let body = format!(
            "if [ -L {p} ]; then k=L; elif [ -d {p} ]; then k=D; elif [ -f {p} ]; then k=F; \
             elif [ -e {p} ]; then k=O; else : >\"$d/n\"; exit 0; fi; \
             n=$(stat -c %s -- {p} 2>/dev/null || stat -f %z -- {p}) || exit 1; printf '%s %s' \"$k\" \"$n\""
        );
        let f = self.one(body, None, QUICK)?;
        if f.ended == Ended::Missing {
            return Ok(None);
        }
        let out = String::from_utf8_lossy(&f.done(path)?).into_owned();
        let (k, n) = out.trim().split_once(' ').ok_or_else(|| PwError::other(format!("{path}: unexpected stat output")))?;
        let kind = match k {
            "L" => FileKind::Symlink,
            "D" => FileKind::Dir,
            "F" => FileKind::File,
            _ => FileKind::Other,
        };
        Ok(Some(Stat { kind, len: n.trim().parse().unwrap_or(0) }))
    }

    fn real_path(&self, path: &str) -> Result<String> {
        let p = sq(path);
        let body = format!(
            "[ -e {p} ] || {{ printf '%s: No such file or directory\\n' {p} >&2; exit 1; }}; \
             realpath -- {p} 2>/dev/null || readlink -f -- {p}"
        );
        let out = self.file_op(path, body, None)?;
        let text = String::from_utf8_lossy(&out);
        Ok(text.strip_suffix('\n').unwrap_or(&text).to_string())
    }

    fn temp_dir(&self) -> Result<String> {
        Ok(self.dirs()?.1.clone())
    }

    fn home(&self) -> Result<String> {
        Ok(self.dirs()?.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::testing::FakeExec;

    fn framed(frames: &[(&str, &[u8], &[u8])]) -> Vec<u8> {
        let mut v = b"motd noise\n".to_vec();
        v.extend_from_slice(START);
        for (st, out, err) in frames {
            v.extend_from_slice(format!("\x1eR {st} {} {}\n", out.len(), err.len()).as_bytes());
            v.extend_from_slice(out);
            v.extend_from_slice(err);
        }
        v
    }

    #[test]
    fn argv_puts_options_before_the_name_and_the_command_after_dashes() {
        let x = FakeExec::new();
        let vm = AgwExec::new("/opt/agw", x.clone(), User::Admin { vm: "my-vm".into() }, Some("work"));
        let a = vm.argv("echo hi");
        assert_eq!(a, ["/opt/agw", "--non-interactive", "vm", "exec", "--workspace", "work", "my-vm", "--", "sh", "-c", "echo hi"]);
        let agent = AgwExec::new("agw", x, User::Agent { agent: "bot".into() }, None);
        assert_eq!(agent.argv("s"), ["agw", "--non-interactive", "agent", "exec", "bot", "--", "sh", "-c", "s"]);
    }

    #[test]
    fn frames_parse_binary_output_and_every_ending() {
        let bin: Vec<u8> = (0..=255u8).collect();
        let raw = framed(&[("0", &bin, b""), ("3", b"out", b"err\n"), ("X", b"", b"git: not found"), ("T", b"", b""), ("N", b"", b"")]);
        let f = frames(&raw).unwrap();
        assert_eq!(f.len(), 5);
        assert_eq!((f[0].ended.clone(), f[0].out.clone()), (Ended::Code(0), bin));
        assert_eq!((f[1].ended.clone(), &f[1].out[..], &f[1].err[..]), (Ended::Code(3), &b"out"[..], &b"err\n"[..]));
        assert_eq!(f[2].ended, Ended::CantStart);
        assert_eq!(f[3].ended, Ended::TimedOut);
        assert_eq!(f[4].ended, Ended::Missing);
        assert!(frames(b"Error: VM 'x' not found\n").is_none(), "no marker: agw failed");
        let mut cut = framed(&[("0", b"abcdef", b"")]);
        cut.truncate(cut.len() - 2);
        assert!(frames(&cut).is_none(), "cut off mid-frame");
    }

    #[test]
    fn agws_own_errors_are_errors_not_command_output() {
        let x = FakeExec::new();
        x.on_exit(&["/opt/agw", "--non-interactive", "vm", "exec", ".."], 1, "", "Error: VM 'gone' not found\n  Hint: agw vm list\n");
        let e = AgwExec::new("/opt/agw", x.clone(), User::Admin { vm: "gone".into() }, None);
        let err = e.run(&Cmd::new(&["git", "status"])).unwrap_err();
        assert!(err.is(pitwall_core::provider::ErrorCode::Unreachable), "{err:?}");
        assert_eq!(err.message, "agw exec on gone: VM 'gone' not found");
        // Batches fail as a whole, with the same error for each command.
        let all = e.run_all(&[Cmd::new(&["a"]), Cmd::new(&["b"])]);
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|r| r.as_ref().unwrap_err().message == err.message));
        assert_eq!(x.calls().len(), 2, "one agw call per batch");
    }

    #[test]
    fn scripts_quote_every_argument_and_frame_each_command() {
        let cmd = Cmd::new(&["git", "-C", "/w s", "log", "--format=%H it's"]).cwd("/w s").env(&[("GIT_INDEX_FILE", "/tmp/i x")]);
        let b = run_body(&cmd);
        assert!(b.starts_with("cd '/w s' 2>/dev/null || "), "{b}");
        assert!(b.contains("command -v 'git' >/dev/null"), "{b}");
        assert!(b.contains("exec ${to:+$to -k 5 600.000} env 'GIT_INDEX_FILE=/tmp/i x' 'git' '-C' '/w s' 'log' '--format=%H it'\\''s'"), "{b}");
        let s = step(&b, false, true);
        assert!(s.contains("</dev/null") && s.contains("s=T") && s.ends_with("emit \"$s\"\n"));
        assert!(!step("x", true, false).contains("</dev/null") && !step("x", true, false).contains("s=T"));
        assert!(!run_body(&Cmd::new(&["true"]).timeout(Duration::MAX)).contains("$to"), "no timeout asked: none on the VM");
    }
}
