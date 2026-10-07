//! [`FakeProvider`] and [`FakeTerm`]: a provider whose "agents" are a tiny
//! in-memory shell, so the engine and the provider contract run without real
//! processes (architecture.md §7). Tests can also drive a terminal by hand
//! through its [`FakeTermCtl`]: write agent output, end it, read what the
//! agent was sent, see every resize.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::error::{ErrorCode, PwError, Result};
use crate::exec::Exec;
use crate::kind::{launch, KindCatalog};
use pitwall_proto::CreateForm;
use crate::provider::{
    CreateSpec, Discovered, ExitInfo, HookTransport, KindOnMachine, LaunchIntent, LaunchSpec, Locator, Machine,
    MachineId, NativeState, Provider, ProviderCaps, ProviderId, Started, TermIo, TermSize,
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------ process

/// One fake agent process: its output history, input, size and exit.
struct Proc {
    pid: u32,
    st: Mutex<ProcState>,
    /// Answers typed lines like a minimal shell (`echo`, `stty size`, `exit N`).
    shell: bool,
}

#[derive(Default)]
struct ProcState {
    history: Vec<u8>,
    input: Vec<u8>,
    line: Vec<u8>,
    size: Option<TermSize>,
    resizes: Vec<TermSize>,
    exit: Option<i32>,
    conns: Vec<Arc<Conn>>,
}

/// One connection's live output queue.
#[derive(Default)]
struct Conn {
    q: Mutex<(VecDeque<u8>, bool)>,
    ready: Condvar,
}

impl Conn {
    fn push(&self, bytes: &[u8]) {
        lock(&self.q).0.extend(bytes);
        self.ready.notify_all();
    }
    fn end(&self) {
        lock(&self.q).1 = true;
        self.ready.notify_all();
    }
    fn ended(&self) -> bool {
        lock(&self.q).1
    }
}

static NEXT_PID: AtomicU32 = AtomicU32::new(90_000);

impl Proc {
    fn new(size: TermSize, shell: bool) -> Arc<Proc> {
        let st = ProcState { size: Some(size), ..Default::default() };
        Arc::new(Proc { pid: NEXT_PID.fetch_add(1, Ordering::Relaxed), st: Mutex::new(st), shell })
    }

    fn output(&self, bytes: &[u8]) {
        let mut st = lock(&self.st);
        if st.exit.is_some() {
            return;
        }
        st.history.extend_from_slice(bytes);
        for c in &st.conns {
            c.push(bytes);
        }
    }

    fn exit(&self, code: i32) {
        let mut st = lock(&self.st);
        if st.exit.is_some() {
            return;
        }
        st.exit = Some(code);
        for c in st.conns.drain(..) {
            c.end();
        }
    }

    fn running(&self) -> bool {
        lock(&self.st).exit.is_none()
    }

    fn connect(&self, with_history: bool) -> (Arc<Conn>, Vec<u8>) {
        let conn = Arc::new(Conn::default());
        let mut st = lock(&self.st);
        if st.exit.is_some() {
            conn.end();
        } else {
            st.conns.push(conn.clone());
        }
        (conn, if with_history { st.history.clone() } else { Vec::new() })
    }

    fn disconnect(&self, conn: &Arc<Conn>) {
        lock(&self.st).conns.retain(|c| !Arc::ptr_eq(c, conn));
        conn.end();
    }

    fn input(&self, bytes: &[u8]) {
        let lines = {
            let mut st = lock(&self.st);
            if st.exit.is_some() {
                return;
            }
            st.input.extend_from_slice(bytes);
            if !self.shell {
                return;
            }
            let mut lines = Vec::new();
            let mut echo = Vec::new();
            for &b in bytes {
                if b == b'\r' || b == b'\n' {
                    echo.extend_from_slice(b"\r\n");
                    lines.push(String::from_utf8_lossy(&std::mem::take(&mut st.line)).into_owned());
                } else {
                    echo.push(b);
                    st.line.push(b);
                }
            }
            drop(st);
            self.output(&echo);
            lines
        };
        for line in lines {
            self.run_line(line.trim());
        }
    }

    fn run_line(&self, line: &str) {
        if let Some(text) = line.strip_prefix("echo ") {
            // The contract echoes `$((1+1))` so output differs from the typed
            // line; expand it like a shell would.
            let text = text.replace("$((1+1))", "2");
            self.output(format!("{text}\r\n").as_bytes());
        } else if line == "stty size" {
            let size = lock(&self.st).size.unwrap_or(TermSize::DEFAULT);
            self.output(format!("{} {}\r\n", size.rows, size.cols).as_bytes());
        } else if let Some(code) = line.strip_prefix("exit") {
            self.exit(code.trim().parse().unwrap_or(0));
            return;
        }
        self.output(b"$ ");
    }

    fn resize(&self, size: TermSize) {
        let mut st = lock(&self.st);
        st.size = Some(size);
        st.resizes.push(size);
    }
}

// ------------------------------------------------------------------ terminal

/// One connection to a fake process, as a [`TermIo`].
pub struct FakeTerm {
    proc: Arc<Proc>,
    conn: Arc<Conn>,
    /// Snapshot taken when connecting; `None`: read at `take_history`.
    history: Option<Vec<u8>>,
    eof_is_exit: bool,
}

/// Drives a fake process from a test.
#[derive(Clone)]
pub struct FakeTermCtl(Arc<Proc>);

impl FakeTerm {
    /// A process that only does what the test makes it do (no shell).
    pub fn new(size: TermSize, pid: Option<u32>) -> (FakeTerm, FakeTermCtl) {
        let mut proc = Proc::new(size, false);
        if let Some(pid) = pid {
            Arc::get_mut(&mut proc).expect("fresh").pid = pid;
        }
        let (conn, _) = proc.connect(false);
        (FakeTerm { proc: proc.clone(), conn, history: None, eof_is_exit: true }, FakeTermCtl(proc))
    }

    fn open(proc: &Arc<Proc>, with_history: bool, eof_is_exit: bool) -> FakeTerm {
        let (conn, history) = proc.connect(with_history);
        FakeTerm { proc: proc.clone(), conn, history: Some(history), eof_is_exit }
    }
}

impl FakeTermCtl {
    /// Output from "before": what the terminal of `FakeTerm::new` (and any
    /// later connection) replays as history.
    pub fn set_history(&self, bytes: &[u8]) {
        lock(&self.0.st).history = bytes.to_vec();
    }
    /// Agent output to every connection.
    pub fn output(&self, bytes: &[u8]) {
        self.0.output(bytes);
    }
    pub fn exit(&self, code: i32) {
        self.0.exit(code);
    }
    /// Everything the agent was sent.
    pub fn input(&self) -> Vec<u8> {
        lock(&self.0.st).input.clone()
    }
    pub fn size(&self) -> TermSize {
        lock(&self.0.st).size.unwrap_or(TermSize::DEFAULT)
    }
    pub fn resizes(&self) -> Vec<TermSize> {
        lock(&self.0.st).resizes.clone()
    }
    pub fn running(&self) -> bool {
        self.0.running()
    }
    pub fn pid(&self) -> u32 {
        self.0.pid
    }
    /// Drop every attachment without ending the process (a tmux client
    /// going away).
    pub fn drop_links(&self) {
        let conns: Vec<_> = lock(&self.0.st).conns.drain(..).collect();
        for c in conns {
            c.end();
        }
    }
}

struct ConnReader(Arc<Conn>);

impl Read for ConnReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut q = lock(&self.0.q);
        loop {
            if !q.0.is_empty() {
                let n = buf.len().min(q.0.len());
                for (i, b) in q.0.drain(..n).enumerate() {
                    buf[i] = b;
                }
                return Ok(n);
            }
            if q.1 {
                return Ok(0);
            }
            q = self.0.ready.wait(q).unwrap_or_else(|e| e.into_inner());
        }
    }
}

struct ProcWriter(Arc<Proc>);

impl Write for ProcWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.input(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TermIo for FakeTerm {
    fn take_history(&mut self) -> Vec<u8> {
        // `FakeTerm::new`: what the test put there before the host took it.
        self.history.take().unwrap_or_else(|| lock(&self.proc.st).history.clone())
    }
    fn take_reader(&mut self) -> Box<dyn Read + Send> {
        Box::new(ConnReader(self.conn.clone()))
    }
    fn take_writer(&mut self) -> Box<dyn Write + Send> {
        Box::new(ProcWriter(self.proc.clone()))
    }
    fn resize(&self, size: TermSize) -> Result<()> {
        self.proc.resize(size);
        Ok(())
    }
    fn try_wait(&self) -> Result<Option<ExitInfo>> {
        if let Some(code) = lock(&self.proc.st).exit {
            return Ok(Some(ExitInfo { code: Some(code) }));
        }
        Ok(self.conn.ended().then_some(ExitInfo { code: None }))
    }
    fn close(&self, _grace: Duration) {
        if self.eof_is_exit {
            // Hang-up.
            self.proc.exit(129);
        } else {
            self.proc.disconnect(&self.conn);
        }
    }
    fn pid(&self) -> Option<u32> {
        Some(self.proc.pid)
    }
    fn eof_is_exit(&self) -> bool {
        self.eof_is_exit
    }
    fn size(&self) -> Option<TermSize> {
        lock(&self.proc.st).size
    }
}

// ------------------------------------------------------------------ provider

/// What a [`FakeProvider`] was asked to launch.
#[derive(Debug, Clone, PartialEq)]
pub struct Launched {
    pub locator: Locator,
    pub kind: String,
    pub cwd: String,
    /// The command line `launch::plan` makes for it.
    pub command_line: String,
    pub resumed: bool,
    pub size: TermSize,
}

struct Session {
    proc: Arc<Proc>,
    cwd: String,
    kind: String,
}

/// A provider with one in-memory machine whose agents are fake shells.
/// Capabilities default to the local provider's (minus `local_process`:
/// its pids are made up) and can be changed per test.
pub struct FakeProvider {
    id: ProviderId,
    machine: Machine,
    caps: Mutex<ProviderCaps>,
    exec: Arc<dyn Exec>,
    /// EOF means exit (local-like) or only a dropped attachment (tmux-like).
    eof_is_exit: Mutex<bool>,
    sessions: Mutex<BTreeMap<String, Session>>,
    launched: Mutex<Vec<Launched>>,
    fail_next: Mutex<Option<PwError>>,
    /// A platform-like form (`set_form`): sessions are made by name with
    /// its options.
    form: Mutex<Option<CreateForm>>,
    created: Mutex<Vec<(String, BTreeMap<String, String>)>>,
}

impl FakeProvider {
    pub fn new(exec: Arc<dyn Exec>) -> Arc<FakeProvider> {
        FakeProvider::named("fake", "fake-machine", exec)
    }

    pub fn named(id: &str, machine: &str, exec: Arc<dyn Exec>) -> Arc<FakeProvider> {
        Arc::new(FakeProvider {
            id: ProviderId::new(id),
            machine: Machine { id: MachineId::new(machine), label: format!("Fake {machine}"), detail: None },
            caps: Mutex::new(FakeProvider::local_like()),
            exec,
            eof_is_exit: Mutex::new(true),
            sessions: Mutex::default(),
            launched: Mutex::default(),
            fail_next: Mutex::default(),
            form: Mutex::default(),
            created: Mutex::default(),
        })
    }

    /// The local provider's capabilities, except `local_process`.
    pub fn local_like() -> ProviderCaps {
        ProviderCaps {
            create: true,
            platform_create: false,
            resume: true,
            start: true,
            attach_existing: false,
            survives_detach: true,
            exec: true,
            process_cwd: true,
            capture: false,
            hooks: HookTransport::LocalSocket,
            rules: true,
            custom_command: true,
            local_process: false,
            git_poll_ms: 0,
        }
    }

    pub fn set_caps(&self, caps: ProviderCaps) {
        *lock(&self.caps) = caps;
    }

    /// tmux-like: terminals come from `attach`, EOF is only a dropped link.
    pub fn set_remote(&self, remote: bool) {
        *lock(&self.eof_is_exit) = !remote;
    }

    /// The next create/start fails with `e`.
    pub fn fail_next(&self, e: PwError) {
        *lock(&self.fail_next) = Some(e);
    }

    /// Make agents the way a platform does (agw): `form` describes the
    /// choices; `create` names the session after the agent, works in
    /// `/srv/<options.workspace or name>` and runs `options.template` (the
    /// platform's word for the program). Its fields aren't checked here: the
    /// engine does that against the form.
    pub fn set_form(&self, form: CreateForm) {
        lock(&self.caps).platform_create = !form.folder;
        *lock(&self.form) = Some(form);
    }

    /// Sessions made through a form: (name, options).
    pub fn created(&self) -> Vec<(String, BTreeMap<String, String>)> {
        lock(&self.created).clone()
    }

    pub fn machine_id(&self) -> &MachineId {
        &self.machine.id
    }

    pub fn launched(&self) -> Vec<Launched> {
        lock(&self.launched).clone()
    }

    /// Drive the process of the session `native` (agent id for local-like use).
    pub fn ctl(&self, native: &str) -> Option<FakeTermCtl> {
        lock(&self.sessions).get(native).map(|s| FakeTermCtl(s.proc.clone()))
    }

    /// A running session the provider has but Pitwall didn't start (for
    /// `discover`/adopt tests).
    pub fn add_session(&self, native: &str, kind: &str, cwd: &str) -> FakeTermCtl {
        let proc = Proc::new(TermSize::DEFAULT, true);
        lock(&self.sessions).insert(native.into(), Session { proc: proc.clone(), cwd: cwd.into(), kind: kind.into() });
        FakeTermCtl(proc)
    }

    /// The session `native`'s process moved to `cwd` (what `process_cwd` reports).
    pub fn set_cwd(&self, native: &str, cwd: &str) {
        if let Some(s) = lock(&self.sessions).get_mut(native) {
            s.cwd = cwd.into();
        }
    }

    fn check(&self, ok: bool, what: &str) -> Result<()> {
        if ok {
            Ok(())
        } else {
            Err(PwError::unsupported(format!("{} can't {what}", self.id)))
        }
    }

    fn launch(&self, loc: Locator, spec: &LaunchSpec) -> Result<Started> {
        if let Some(e) = lock(&self.fail_next).take() {
            return Err(e);
        }
        let caps = *lock(&self.caps);
        let (session_id, resume) = match &spec.intent {
            LaunchIntent::Fresh => (None, false),
            LaunchIntent::Resume(id) => {
                self.check(caps.resume, "resume")?;
                (Some(id.as_str()), true)
            }
        };
        let hook = spec.hooks.as_ref().map_or("", |h| h.command.as_str());
        let plan = launch::plan(spec.kind, session_id, resume, hook, spec.worktree);
        let proc = Proc::new(spec.size, true);
        if plan.resumed {
            proc.output(format!("resumed:{}\r\n", plan.session_id.as_deref().unwrap_or("")).as_bytes());
        }
        proc.output(b"$ ");
        let old = lock(&self.sessions).insert(
            loc.native.clone(),
            Session { proc: proc.clone(), cwd: spec.cwd.into(), kind: spec.kind.id.clone() },
        );
        if let Some(old) = old {
            old.proc.exit(129);
        }
        lock(&self.launched).push(Launched {
            locator: loc.clone(),
            kind: spec.kind.id.clone(),
            cwd: spec.cwd.into(),
            command_line: plan.command_line,
            resumed: plan.resumed,
            size: spec.size,
        });
        let eof_is_exit = *lock(&self.eof_is_exit);
        Ok(Started {
            locator: loc,
            conversation_id: plan.session_id,
            resumed: plan.resumed,
            cwd: spec.cwd.into(),
            // Local-like: the session is the process; tmux-like: attach to it.
            term: eof_is_exit.then(|| Box::new(FakeTerm::open(&proc, true, true)) as Box<dyn TermIo>),
            kind: None,
        })
    }

    fn session<T>(&self, loc: &Locator, f: impl FnOnce(&Session) -> T) -> Option<T> {
        lock(&self.sessions).get(&loc.native).map(f)
    }

    fn mine(&self, loc: &Locator) -> Result<()> {
        if loc.provider != self.id || loc.machine != self.machine.id {
            return Err(PwError::not_found(format!("{loc} is not on {}", self.id)));
        }
        Ok(())
    }
}

impl Provider for FakeProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn caps(&self) -> ProviderCaps {
        *lock(&self.caps)
    }

    fn machines(&self) -> Result<Vec<Machine>> {
        Ok(vec![self.machine.clone()])
    }

    fn kinds(&self, _m: &MachineId, catalog: &KindCatalog) -> Result<Vec<KindOnMachine>> {
        Ok(catalog.kinds().into_iter().map(|kind| KindOnMachine { kind, installed: true, path: None }).collect())
    }

    fn discover(&self, _m: &MachineId) -> Result<Vec<Discovered>> {
        Ok(lock(&self.sessions)
            .iter()
            .map(|(native, s)| Discovered {
                locator: Locator::new(&self.id, &self.machine.id, native),
                kind: s.kind.clone(),
                state: Some(if s.proc.running() { NativeState::Running } else { NativeState::Stopped }),
                cwd: Some(s.cwd.clone()),
                title: None,
                workspace: None,
                user: None,
            })
            .collect())
    }

    fn create_form(&self, m: &MachineId) -> Result<CreateForm> {
        Ok(lock(&self.form).clone().unwrap_or_else(|| CreateForm::folder(self.id.as_str(), m.as_str(), &self.machine.label)))
    }

    fn create(&self, spec: &CreateSpec) -> Result<Started> {
        self.check(self.caps().create, "start agents")?;
        if *spec.machine != self.machine.id {
            return Err(PwError::not_found(format!("no machine {}", spec.machine)));
        }
        if lock(&self.form).is_none() {
            if !spec.options.is_empty() {
                return Err(PwError::unsupported("only folders"));
            }
            let loc = Locator::new(&self.id, &self.machine.id, spec.launch.agent);
            return self.launch(loc, &spec.launch);
        }
        if lock(&self.sessions).contains_key(spec.name) {
            return Err(PwError::new(ErrorCode::Conflict, format!("session '{}' already exists", spec.name)));
        }
        let cwd = format!("/srv/{}", spec.options.get("workspace").map_or(spec.name, String::as_str));
        let launch = LaunchSpec { cwd: &cwd, ..spec.launch.clone() };
        let mut started = self.launch(Locator::new(&self.id, &self.machine.id, spec.name), &launch)?;
        started.kind = spec.options.get("template").cloned();
        if let (Some(s), Some(k)) = (lock(&self.sessions).get_mut(spec.name), &started.kind) {
            s.kind = k.clone();
        }
        lock(&self.created).push((spec.name.to_string(), spec.options.clone()));
        Ok(started)
    }

    fn start(&self, loc: &Locator, launch: &LaunchSpec) -> Result<Started> {
        self.mine(loc)?;
        self.check(self.caps().start, "start agents again")?;
        self.launch(loc.clone(), launch)
    }

    fn attach(&self, loc: &Locator, _size: TermSize) -> Result<Box<dyn TermIo>> {
        self.mine(loc)?;
        let caps = self.caps();
        self.check(caps.survives_detach || caps.attach_existing, "attach")?;
        let proc = self.session(loc, |s| s.proc.clone()).ok_or_else(PwError::not_running)?;
        if !proc.running() {
            return Err(PwError::not_running());
        }
        Ok(Box::new(FakeTerm::open(&proc, true, *lock(&self.eof_is_exit))))
    }

    fn stop(&self, loc: &Locator) -> Result<()> {
        self.mine(loc)?;
        if let Some(p) = self.session(loc, |s| s.proc.clone()) {
            p.exit(129);
        }
        Ok(())
    }

    /// Deletes sessions it started; one it only had (`add_session`, an
    /// adopted session) or made through its form (a platform's, like agw)
    /// is left as it is.
    fn remove(&self, loc: &Locator) -> Result<()> {
        self.mine(loc)?;
        let platform = lock(&self.created).iter().any(|(name, _)| *name == loc.native);
        if platform || !lock(&self.launched).iter().any(|l| l.locator == *loc) {
            return Ok(());
        }
        if let Some(s) = lock(&self.sessions).remove(&loc.native) {
            s.proc.exit(129);
        }
        Ok(())
    }

    fn state(&self, loc: &Locator) -> Result<NativeState> {
        self.mine(loc)?;
        Ok(match self.session(loc, |s| s.proc.running()) {
            Some(true) => NativeState::Running,
            Some(false) => NativeState::Stopped,
            None => NativeState::Gone,
        })
    }

    fn exec(&self, m: &MachineId) -> Result<Arc<dyn Exec>> {
        self.check(self.caps().exec, "run commands")?;
        if *m != self.machine.id {
            return Err(PwError::new(ErrorCode::NotFound, format!("no machine {m}")));
        }
        Ok(self.exec.clone())
    }

    fn process_cwd(&self, loc: &Locator, _pid: Option<u32>) -> Result<Option<String>> {
        self.mine(loc)?;
        self.check(self.caps().process_cwd, "find a process's folder")?;
        Ok(self.session(loc, |s| s.proc.running().then(|| s.cwd.clone())).flatten())
    }

    fn capture(&self, loc: &Locator) -> Result<String> {
        self.mine(loc)?;
        self.check(self.caps().capture, "capture a screen")?;
        let proc = self.session(loc, |s| s.proc.clone()).ok_or_else(PwError::not_running)?;
        let text = String::from_utf8_lossy(&lock(&proc.st).history).into_owned();
        Ok(text)
    }
}
