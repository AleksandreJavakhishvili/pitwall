//! `pitwall mcp`: Pitwall's MCP server, on stdin/stdout (JSON-RPC 2.0, one
//! message per line, as MCP's stdio transport says). Pitwall adds it to
//! the agents it starts (`agents.showFiles`, `agents.showWeb`,
//! `agents.showSettings`) so they can show the user files, web pages and
//! Pitwall's Settings. Nothing is ever printed but protocol messages.
//!
//! It is a proxy: the agent's CLI keeps this process as long as the agent
//! runs, across Pitwall updates, so the tools aren't this build's. Their
//! list, the instructions and every call go to the running Pitwall
//! (`mcp.tools`, `mcp.call`, a connection each, so a Pitwall started after
//! the agent still answers), which checks and answers them; the answers
//! pass through as JSON. When the tools Pitwall has change (it was updated
//! and restarted), the agent is sent `notifications/tools/list_changed` so
//! it reads them again: noticed on each call and every
//! [`WATCH_EVERY`]. Pitwall not running, or older than `mcp.*`: this
//! build's own tools ([`pitwall_proto::mcp`]), said on stderr.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use pitwall_client::{Client, Error};
use pitwall_proto::mcp::{self, McpCall, McpCalled, McpTools};
use pitwall_proto::show::MCP_SERVER;
use pitwall_proto::{caps, code, method, Role, ViewerOpen, WebOpen};

/// Protocol versions this server speaks, newest first.
pub const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// How often the running Pitwall's tools are looked at between calls.
pub const WATCH_EVERY: Duration = Duration::from_secs(20);

/// Why the running Pitwall didn't answer an `mcp.*` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Miss {
    /// Not reached (not running): nothing was sent.
    Down(String),
    /// Reached, but older than `mcp.*`.
    Old,
    /// Reached, and it refused (nothing was done).
    Refused(String),
    /// Sent, but no answer came back: it may have been done.
    Lost(String),
}

/// The running Pitwall (tests plug in their own).
pub trait Upstream: Sync {
    /// `mcp.tools`.
    fn tools(&self) -> Result<McpTools, Miss>;
    /// `mcp.call`.
    fn call(&self, c: &McpCall) -> Result<McpCalled, Miss>;
    /// `viewer.open`, for a Pitwall older than `mcp.*`: its answer as it
    /// came, and in words.
    fn open(&self, req: &ViewerOpen) -> Result<(Value, String), Error>;
    /// `web.open`, for a Pitwall older than `mcp.*` (which has none: it is
    /// told so).
    fn open_web(&self, req: &WebOpen) -> Result<(Value, String), Error> {
        let _ = req;
        Err(Error::Server(pitwall_proto::ErrorBody::new(code::UNSUPPORTED, crate::open::NO_WEB)))
    }
}

/// The running Pitwall at `socket`.
pub struct SocketUpstream {
    pub socket: PathBuf,
}

impl SocketUpstream {
    fn client(&self) -> Result<Client, Error> {
        Client::connect_as(&self.socket, &format!("pitwall-mcp/{}", env!("CARGO_PKG_VERSION")), Role::Cli)
    }

    fn mcp(&self, name: &str, params: Value) -> Result<Value, Miss> {
        let mut c = self.client().map_err(|e| Miss::Down(e.to_string()))?;
        if !c.has(caps::MCP) {
            return Err(Miss::Old);
        }
        c.call(name, params).map_err(|e| match e {
            Error::Server(b) => Miss::Refused(b.message),
            other => Miss::Lost(other.to_string()),
        })
    }
}

impl Upstream for SocketUpstream {
    fn tools(&self) -> Result<McpTools, Miss> {
        let v = self.mcp(method::MCP_TOOLS, json!({}))?;
        serde_json::from_value(v).map_err(|e| Miss::Refused(format!("its tools: {e}")))
    }

    fn call(&self, c: &McpCall) -> Result<McpCalled, Miss> {
        let v = self.mcp(method::MCP_CALL, serde_json::to_value(c).unwrap_or_default())?;
        // Read leniently: whatever came, it was done.
        Ok(serde_json::from_value(v.clone()).unwrap_or(McpCalled { result: v, stamp: String::new() }))
    }

    fn open(&self, req: &ViewerOpen) -> Result<(Value, String), Error> {
        crate::open::open(&mut self.client()?, req)
    }

    fn open_web(&self, req: &WebOpen) -> Result<(Value, String), Error> {
        crate::open::open_web(&mut self.client()?, req)
    }
}

/// What a failed fallback call tells the agent, and whether it is an error
/// (one that may have opened isn't).
fn failure(e: &Error) -> (String, bool) {
    match e {
        Error::Connect { .. } | Error::Rejected(_) => ("Pitwall isn't running (or can't be reached), so nothing was opened.".into(), true),
        Error::Server(b) if b.code == code::OFF => (b.message.clone(), true),
        Error::Server(b) if b.message == crate::open::NO_WEB => (b.message.clone(), true),
        Error::Server(b) => (format!("Nothing was opened: {}", b.message), true),
        other => (lost(&other.to_string()), false),
    }
}

fn lost(e: &str) -> String {
    format!("The request reached Pitwall, but its answer didn't come back ({e}): the files may well have opened. Ask the user before showing them again.")
}

fn list_changed() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })
}

/// What the agent was given and told.
#[derive(Default)]
struct State {
    /// The stamp of the tools it was last listed.
    listed: Option<String>,
    /// A stamp it was told about since (`list_changed`, once per change).
    told: Option<String>,
    /// Initialized: notifications may go out.
    ready: bool,
    /// Said on stderr that this build's tools stand in.
    said: bool,
}

/// One agent's server.
pub struct Proxy<'a> {
    up: &'a dyn Upstream,
    agent: Option<String>,
    state: Mutex<State>,
}

impl<'a> Proxy<'a> {
    pub fn new(up: &'a dyn Upstream, agent: Option<String>) -> Self {
        Proxy { up, agent, state: Mutex::new(State::default()) }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The running Pitwall's tools, else this build's.
    fn tools(&self) -> McpTools {
        match self.up.tools() {
            Ok(t) => t,
            Err(m) => {
                let mut st = self.state();
                if !st.said {
                    st.said = true;
                    eprintln!(
                        "pitwall mcp: {}; serving this build's tools ({})",
                        match m {
                            Miss::Old => "the running Pitwall is older than its tool server".to_string(),
                            Miss::Down(e) | Miss::Refused(e) | Miss::Lost(e) => format!("Pitwall isn't reachable ({e})"),
                        },
                        env!("CARGO_PKG_VERSION")
                    );
                }
                mcp::own()
            }
        }
    }

    /// `list_changed` when the tools `stamp` stands for aren't the ones the
    /// agent was given (once per change).
    pub fn check(&self, stamp: &str) -> Option<Value> {
        let mut st = self.state();
        if stamp.is_empty() || !st.ready || st.listed.as_deref().is_none_or(|l| l == stamp) || st.told.as_deref() == Some(stamp) {
            return None;
        }
        st.told = Some(stamp.into());
        Some(list_changed())
    }

    /// The running Pitwall's tools looked at (the watcher): `list_changed`
    /// when they changed. Not reachable: nothing to say.
    pub fn watch(&self) -> Option<Value> {
        match self.up.tools() {
            Ok(t) => self.check(&t.stamp),
            Err(Miss::Old) => self.check(&mcp::own().stamp),
            Err(_) => None,
        }
    }

    /// `tools/call`: its result, and `list_changed` when the answering
    /// Pitwall's tools aren't the ones the agent has.
    pub fn call(&self, params: &Value) -> (Value, Option<Value>) {
        let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
        match self.up.call(&McpCall { name: name.clone(), arguments: arguments.clone(), agent: self.agent.clone(), cli: false, timeout_ms: None }) {
            Ok(c) => {
                let told = self.check(&c.stamp);
                (c.result, told)
            }
            Err(Miss::Down(_)) => (mcp::result("Pitwall isn't running (or can't be reached), so nothing was opened.", None, true), None),
            Err(Miss::Refused(e)) => (mcp::result(&format!("Nothing was opened: {e}"), None, true), None),
            Err(Miss::Lost(e)) => (mcp::result(&lost(&e), None, false), None),
            // An older Pitwall: `viewer.open`, with this build's tools.
            Err(Miss::Old) => {
                let r = match mcp::request(&name, &arguments, self.agent.clone()) {
                    Err(e) => mcp::result(&format!("Nothing was opened: {e}"), None, true),
                    // `mcp.*` came before `settings.open`: an older Pitwall
                    // has neither.
                    Ok(mcp::Request::Settings(_)) => mcp::result(crate::open::NO_SETTINGS, None, true),
                    Ok(mcp::Request::Device(_)) => mcp::result(crate::open::NO_DEVICE, None, true),
                    Ok(mcp::Request::Web(req)) => match self.up.open_web(&req) {
                        Ok((v, text)) => mcp::result(&text, Some(v), false),
                        Err(e) => {
                            let (text, error) = failure(&e);
                            mcp::result(&text, None, error)
                        }
                    },
                    Ok(mcp::Request::Viewer(req)) => match self.up.open(&req) {
                        Ok((v, text)) => mcp::result(&text, Some(v), false),
                        Err(e) => {
                            let (text, error) = failure(&e);
                            mcp::result(&text, None, error)
                        }
                    },
                };
                (r, self.check(&mcp::own().stamp))
            }
        }
    }

    /// The answers to one message, in order: its reply (none for a
    /// notification or a response) and maybe `list_changed`.
    pub fn handle(&self, msg: &Value) -> Vec<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str);
        let (Some(id), Some(method)) = (id, method) else {
            if method == Some("notifications/initialized") {
                self.state().ready = true;
            }
            return vec![];
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let mut after = None;
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
                let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": true } },
                    "serverInfo": { "name": MCP_SERVER, "title": "Pitwall", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": self.tools().instructions
                })
            }
            "ping" => json!({}),
            "tools/list" => {
                let t = self.tools();
                let mut st = self.state();
                st.ready = true;
                st.listed = Some(t.stamp);
                st.told = None;
                json!({ "tools": t.tools })
            }
            "tools/call" => {
                let (r, told) = self.call(&params);
                after = told;
                r
            }
            other => {
                return vec![json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {other}") } })];
            }
        };
        let mut out = vec![json!({ "jsonrpc": "2.0", "id": id, "result": result })];
        out.extend(after);
        out
    }
}

/// Serve until stdin closes; with `watch`, the running Pitwall's tools are
/// also looked at that often between messages.
pub fn serve(proxy: &Proxy, input: impl BufRead, out: &mut (impl Write + Send), watch: Option<Duration>) -> std::io::Result<()> {
    let out = Mutex::new(out);
    let send = |v: &Value| -> std::io::Result<()> {
        let mut o = out.lock().unwrap_or_else(|p| p.into_inner());
        writeln!(o, "{v}")?;
        o.flush()
    };
    let done = AtomicBool::new(false);
    std::thread::scope(|s| {
        if let Some(every) = watch {
            let (done, send) = (&done, &send);
            s.spawn(move || {
                let step = Duration::from_millis(100).min(every);
                let mut waited = Duration::ZERO;
                while !done.load(Ordering::Relaxed) {
                    std::thread::sleep(step);
                    waited += step;
                    if waited < every {
                        continue;
                    }
                    waited = Duration::ZERO;
                    if let Some(n) = proxy.watch() {
                        let _ = send(&n);
                    }
                }
            });
        }
        let served = (|| {
            for line in input.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let replies = match serde_json::from_str::<Value>(&line) {
                    Ok(msg) if msg.is_object() => proxy.handle(&msg),
                    Ok(_) => vec![json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32600, "message": "one JSON-RPC request per line" } })],
                    Err(e) => vec![json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } })],
                };
                for r in &replies {
                    send(r)?;
                }
            }
            Ok(())
        })();
        done.store(true, Ordering::Relaxed);
        served
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::ErrorBody;

    /// A Pitwall as the proxy sees it: serving `mcp.*` (with `tools`, which
    /// a test may change), older than that, or not running.
    struct Fake {
        mode: Mutex<Mode>,
        tools: Mutex<McpTools>,
        calls: Mutex<Vec<McpCall>>,
        opens: Mutex<Vec<ViewerOpen>>,
        old_answer: fn(&ViewerOpen) -> Result<Value, Error>,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Serves,
        Old,
        Down,
        Lost,
    }

    /// What a Pitwall from before `mcp.*` answers `viewer.open` (its shape
    /// then: `beside`, no words).
    fn first_shape(req: &ViewerOpen) -> Result<Value, Error> {
        Ok(json!({"agentId":"a1","agentName":"api","window":"main","inBackground":false,
            "opened": req.files.iter().map(|f| json!({"path": format!("/w/{f}")})).collect::<Vec<_>>(),
            "place":"beside","shownIn":"a pane beside api's terminal"}))
    }

    fn fake(mode: Mode) -> Fake {
        Fake { mode: Mutex::new(mode), tools: Mutex::new(mcp::own()), calls: Mutex::default(), opens: Mutex::default(), old_answer: first_shape }
    }

    impl Upstream for Fake {
        fn tools(&self) -> Result<McpTools, Miss> {
            match *self.mode.lock().unwrap() {
                Mode::Serves | Mode::Lost => Ok(self.tools.lock().unwrap().clone()),
                Mode::Old => Err(Miss::Old),
                Mode::Down => Err(Miss::Down("no socket".into())),
            }
        }
        fn call(&self, c: &McpCall) -> Result<McpCalled, Miss> {
            match *self.mode.lock().unwrap() {
                Mode::Serves => {
                    self.calls.lock().unwrap().push(c.clone());
                    let text = format!("Opened by the app: {}", c.arguments);
                    Ok(McpCalled { result: mcp::result(&text, Some(json!({"place": "somewhereNew"})), false), stamp: self.tools.lock().unwrap().stamp.clone() })
                }
                Mode::Lost => Err(Miss::Lost("connection reset".into())),
                Mode::Old => Err(Miss::Old),
                Mode::Down => Err(Miss::Down("no socket".into())),
            }
        }
        fn open(&self, req: &ViewerOpen) -> Result<(Value, String), Error> {
            self.opens.lock().unwrap().push(req.clone());
            let v = (self.old_answer)(req).map_err(crate::open::said_older)?;
            let t = mcp::opened_text(&v);
            Ok((v, t))
        }
    }

    /// Lines in, lines out, as a client sees it over stdio.
    fn session(p: &Proxy, lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let mut out = Vec::new();
        serve(p, input.as_bytes(), &mut out, None).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn init() -> Vec<Value> {
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ]
    }

    fn call(id: u32, name: &str, args: Value) -> Value {
        json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}})
    }

    #[test]
    fn the_running_pitwall_answers_and_its_answer_passes_through() {
        let f = fake(Mode::Serves);
        f.tools.lock().unwrap().instructions = "the app's words".into();
        let p = Proxy::new(&f, Some("a1".into()));
        let mut lines = init();
        lines.push(call(3, "show_files", json!({"paths":["a.rs"],"where":"somewhereNew"})));
        lines.push(json!({"jsonrpc":"2.0","id":"x","method":"ping"}));
        let out = session(&p, &lines);
        assert_eq!(out.len(), 4, "{out:?}");
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(out[0]["result"]["capabilities"]["tools"]["listChanged"], true);
        assert_eq!(out[0]["result"]["instructions"], "the app's words");
        let names: Vec<&str> = out[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["show_files", "show_folder", "show_web", "show_settings", "show_device"]);
        // A `where` this build doesn't know goes to Pitwall as it is, and
        // its answer comes back as it is.
        assert_eq!(out[2]["result"]["isError"], false);
        assert_eq!(out[2]["result"]["content"][0]["text"], r#"Opened by the app: {"paths":["a.rs"],"where":"somewhereNew"}"#);
        assert_eq!(out[2]["result"]["structuredContent"]["place"], "somewhereNew");
        assert_eq!(out[3], json!({"jsonrpc":"2.0","id":"x","result":{}}));
        assert_eq!(f.calls.lock().unwrap()[0].agent.as_deref(), Some("a1"));
    }

    #[test]
    fn a_pitwall_with_other_tools_has_the_agent_list_them_again() {
        let f = fake(Mode::Serves);
        let p = Proxy::new(&f, None);
        session(&p, &init());
        // Updated and restarted: other tools.
        f.tools.lock().unwrap().stamp = "newer".into();
        let out = session(&p, &[call(3, "show_files", json!({"paths":["a"]})), call(4, "show_files", json!({"paths":["b"]}))]);
        assert_eq!(out.len(), 3, "told once: {out:?}");
        assert_eq!(out[0]["id"], 3);
        assert_eq!(out[1], json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}));
        assert_eq!(out[2]["id"], 4);
        // The watcher notices too, also with no call; once listed again,
        // nothing more.
        f.tools.lock().unwrap().stamp = "newest".into();
        assert_eq!(p.watch(), Some(list_changed()));
        assert_eq!(p.watch(), None);
        session(&p, &[json!({"jsonrpc":"2.0","id":5,"method":"tools/list"})]);
        assert_eq!(p.watch(), None);
        // The watcher runs in serve, too.
        f.tools.lock().unwrap().stamp = "again".into();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let reader = Lines(rx);
        let mut out = Vec::new();
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(400));
                tx.send(String::new()).unwrap();
            });
            serve(&p, reader, &mut out, Some(Duration::from_millis(50))).unwrap();
        });
        assert_eq!(String::from_utf8(out).unwrap().trim(), list_changed().to_string());
    }

    /// stdin that ends once a line comes (the watcher test).
    struct Lines(std::sync::mpsc::Receiver<String>);
    impl std::io::Read for Lines {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }
    impl BufRead for Lines {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            let _ = self.0.recv();
            Ok(&[])
        }
        fn consume(&mut self, _: usize) {}
    }

    #[test]
    fn pitwall_not_running_then_started() {
        let f = fake(Mode::Down);
        let p = Proxy::new(&f, None);
        let mut lines = init();
        lines.push(call(3, "show_files", json!({"paths":["a"]})));
        let out = session(&p, &lines);
        assert_eq!(out[0]["result"]["instructions"], mcp::all_instructions(), "this build's");
        assert_eq!(out[1]["result"]["tools"], mcp::tools());
        assert_eq!(out[2]["result"]["isError"], true);
        assert!(out[2]["result"]["content"][0]["text"].as_str().unwrap().contains("Pitwall isn't running"));
        assert_eq!(p.watch(), None, "nothing to say while it isn't");
        // Started, a newer one.
        *f.mode.lock().unwrap() = Mode::Serves;
        f.tools.lock().unwrap().stamp = "newer".into();
        assert_eq!(p.watch(), Some(list_changed()));
    }

    #[test]
    fn an_older_pitwall_gets_viewer_open_and_its_answer_is_read() {
        let f = fake(Mode::Old);
        let p = Proxy::new(&f, Some("a1".into()));
        let mut lines = init();
        lines.push(call(3, "show_files", json!({"paths":["src/a.rs"]})));
        let out = session(&p, &lines);
        assert_eq!(out.len(), 3, "{out:?}");
        let r = &out[2]["result"];
        assert_eq!(r["isError"], false, "it opened: {r}");
        assert_eq!(r["content"][0]["text"], "Opened in a pane beside api's terminal in Pitwall: /w/src/a.rs.");
        assert_eq!(r["structuredContent"]["place"], "beside");
        assert_eq!(f.opens.lock().unwrap()[0].agent_id.as_deref(), Some("a1"));
        // One that refuses a word it doesn't know: said to be older.
        let f = Fake { old_answer: |_| Err(Error::Server(ErrorBody::new(code::BAD_PARAMS, "unknown variant `left`, expected `beside` or `window`"))), ..fake(Mode::Old) };
        let p = Proxy::new(&f, None);
        let out = session(&p, &[call(1, "show_files", json!({"paths":["a"],"where":"left"}))]);
        assert_eq!(out[0]["result"]["isError"], true, "nothing opened");
        assert!(out[0]["result"]["content"][0]["text"].as_str().unwrap().contains("This Pitwall is older than its command-line tool"), "{out:?}");
        // Bad arguments never reach it.
        let out = session(&p, &[call(1, "show_files", json!({"paths":["a"],"where":"tab"})), call(2, "rm_rf", json!({}))]);
        assert!(out[0]["result"]["content"][0]["text"].as_str().unwrap().contains("right, left, above, below, replace or window"));
        assert!(out[1]["result"]["content"][0]["text"].as_str().unwrap().contains("no tool"));
        assert_eq!(f.opens.lock().unwrap().len(), 1, "only the `left` one reached it");
        // `show_web` from this build's list, to a Pitwall from before it:
        // told plainly that this Pitwall is older, nothing opened; a bad
        // address never gets there.
        let out = session(&p, &[call(1, "show_web", json!({"url": "example.com"})), call(2, "show_web", json!({"url": "javascript:alert(1)"}))]);
        assert_eq!(out[0]["result"]["isError"], true);
        assert!(out[0]["result"]["content"][0]["text"].as_str().unwrap().contains("older than its command-line tool and can't show web pages"), "{out:?}");
        assert!(out[1]["result"]["content"][0]["text"].as_str().unwrap().contains("Only web addresses"), "{out:?}");
    }

    /// A newer Pitwall's `show_web` goes through an older tool server: it
    /// passes any call on as it came (`mcp.call`), so the tool works with
    /// no change to the server the agent was started with.
    #[test]
    fn show_web_passes_through_like_any_tool() {
        let f = fake(Mode::Serves);
        let p = Proxy::new(&f, Some("a1".into()));
        let out = session(&p, &[call(1, "show_web", json!({"url": "example.com", "where": "left"}))]);
        assert_eq!(out[0]["result"]["isError"], false);
        let c = f.calls.lock().unwrap()[0].clone();
        assert_eq!((c.name.as_str(), c.arguments.clone()), ("show_web", json!({"url": "example.com", "where": "left"})));
        // Down: the fallback answer is the same as for the files tools.
        let f = fake(Mode::Down);
        let p = Proxy::new(&f, None);
        let out = session(&p, &[call(1, "show_web", json!({}))]);
        assert_eq!(out[0]["result"]["isError"], true);
        assert!(out[0]["result"]["content"][0]["text"].as_str().unwrap().contains("Pitwall isn't running"));
    }

    #[test]
    fn an_answer_that_doesnt_come_back_is_not_called_a_failure() {
        let f = fake(Mode::Lost);
        let p = Proxy::new(&f, None);
        let out = session(&p, &[call(1, "show_files", json!({"paths":["a"]}))]);
        assert_eq!(out[0]["result"]["isError"], false);
        assert!(out[0]["result"]["content"][0]["text"].as_str().unwrap().contains("may well have opened"));
        assert!(!failure(&Error::Protocol("cut".into())).1);
        assert!(failure(&Error::Connect { path: "/x".into(), source: std::io::ErrorKind::NotFound.into() }).1);
    }

    #[test]
    fn bad_lines_and_unknown_methods() {
        let input = "not json\n[1]\n{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"resources/list\"}\n";
        let mut out = Vec::new();
        let f = fake(Mode::Serves);
        serve(&Proxy::new(&f, None), input.as_bytes(), &mut out, None).unwrap();
        let out: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(out[0]["error"]["code"], -32700);
        assert_eq!(out[1]["error"]["code"], -32600);
        assert_eq!((out[2]["id"].clone(), out[2]["error"]["code"].clone()), (json!(5), json!(-32601)));
    }
}
