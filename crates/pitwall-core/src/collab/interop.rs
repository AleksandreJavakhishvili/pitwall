//! Black-box clients against the integrated core (plan.md §12 "A2A
//! conformance", "MCP conformance"): the conversation core (`Collab`) with
//! a scripted session host that answers every message, behind the real
//! fronts, served on a loopback port as one caller (bob, as the Fake
//! network proves him). Opt-in (`#[ignore]`): each needs a program that
//! isn't part of the build.
//!
//! - `interop_claude_code_on_the_core` (and `…_streaming_…`, a slow turn
//!   that waits for the owner on the way): plain Claude Code (`claude -p`,
//!   `--model haiku`, an HTTP MCP config of its own; the user's config is
//!   untouched). Needs `claude` on PATH and a signed-in account (it runs
//!   one small turn on that account): `cargo test -p pitwall-core --lib
//!   interop_claude_code -- --ignored --nocapture`.
//! - `interop_a2a_sdk`: the official A2A Python SDK (`a2a-sdk`) resolves the
//!   share's Agent Card and sends a message over the card's HTTP+JSON
//!   interface. Needs a Python with it installed, named by
//!   `PITWALL_A2A_PYTHON`, e.g. a throwaway venv:
//!   `uv venv /tmp/a2a && VIRTUAL_ENV=/tmp/a2a uv pip install a2a-sdk httpx`,
//!   then `PITWALL_A2A_PYTHON=/tmp/a2a/bin/python cargo test -p
//!   pitwall-core --lib interop_a2a_sdk -- --ignored --nocapture`.
//! - `interop_mcp_inspector`: the MCP Inspector's CLI lists the tools and
//!   calls `ask_api_expert`. Needs it installed somewhere, named by
//!   `PITWALL_MCP_INSPECTOR` (its launcher), e.g.
//!   `npm install --prefix /tmp/inspector @modelcontextprotocol/inspector`,
//!   then `PITWALL_MCP_INSPECTOR=/tmp/inspector/node_modules/.bin/mcp-inspector`.

use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::core::tests::World;
use super::front::tests::serve_locally_on;
use super::front::{Fronts, Options};
use super::session::SessionEvent;
use super::turn::Reply;
use crate::net::{ClientAuth, Endpoint};
use pitwall_proto::collab::ReplySource;

const REPLY: &str = "Refresh tokens are rotated in src/auth/refresh.rs, in rotate_refresh_token().";

/// The core with a live share, `api-expert`, open to everyone on the
/// network, whose sessions answer every message with [`REPLY`]; served on
/// a loopback port. Returns the world (keep it alive), the base URL and
/// the request log.
fn serve_the_core() -> (World, String, Arc<std::sync::Mutex<Vec<String>>>) {
    serve_the_core_with(false)
}

/// `slow`: each turn takes a few seconds and waits for the owner on the
/// way (the caller sees "waiting for alice"), as a session that stopped
/// to ask would.
fn serve_the_core_with(slow: bool) -> (World, String, Arc<std::sync::Mutex<Vec<String>>>) {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    // Every message typed into a session gets the reply, as a hook would.
    let host = w.host.clone();
    thread::spawn(move || {
        let mut answered = 0;
        loop {
            let sent = host.sent.lock().unwrap().clone();
            for (agent, _) in sent.iter().skip(answered) {
                thread::sleep(Duration::from_millis(300));
                host.emit(agent, SessionEvent::Working);
                if slow {
                    thread::sleep(Duration::from_secs(2));
                    host.emit(agent, SessionEvent::Blocked { detail: Some("Allow edit?".into()) });
                    thread::sleep(Duration::from_secs(1));
                    host.emit(agent, SessionEvent::Working);
                }
                host.emit(agent, SessionEvent::TurnEnded(Some(Reply { text: REPLY.into(), source: ReplySource::Hook })));
            }
            answered = sent.len();
            thread::sleep(Duration::from_millis(50));
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let endpoint = Endpoint { base_url: base.clone(), client_auth: ClientAuth::None };
    let fronts = Fronts::new(w.collab.clone(), w.teams.clone(), move |_| Some(endpoint.clone()), Options::default());
    w.net.device("bob@example.com", "bob-mbp");
    let caller = w.net.caller("bob-mbp").unwrap();
    let (_, log) = serve_locally_on(listener, fronts, caller);
    (w, base, log)
}

fn print_log(log: &std::sync::Mutex<Vec<String>>) {
    eprintln!("--- requests\n{}", log.lock().unwrap().join("\n"));
}

#[test]
#[ignore = "runs real Claude Code (`claude -p --model haiku`) on the signed-in account"]
fn interop_claude_code_on_the_core() {
    claude_code(false);
}

/// The same with a turn that takes a few seconds and waits for the owner
/// on the way: the answer comes over SSE, with progress first.
#[test]
#[ignore = "runs real Claude Code (`claude -p --model haiku`) on the signed-in account"]
fn interop_claude_code_streaming_on_the_core() {
    claude_code(true);
}

fn claude_code(slow: bool) {
    let (w, base, log) = serve_the_core_with(slow);
    let dir = crate::testing::TempDir::new("interop-claude");
    let config = dir.path().join("mcp.json");
    std::fs::write(&config, json!({"mcpServers": {"alice": {"type": "http", "url": format!("{base}/mcp")}}}).to_string()).unwrap();
    let out = std::process::Command::new("claude")
        .current_dir(dir.path())
        .args(["-p", "Call the ask_api_expert tool once with the message \"Where are refresh tokens rotated?\" and then repeat its reply word for word."])
        .args(["--mcp-config", config.to_str().unwrap(), "--strict-mcp-config"])
        .args(["--allowedTools", "mcp__alice__ask_api_expert", "--model", "haiku", "--output-format", "text"])
        .output()
        .expect("claude on PATH");
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("--- claude stdout\n{stdout}\n--- stderr\n{}", String::from_utf8_lossy(&out.stderr));
    print_log(&log);
    // The core ran a real conversation: a sandboxed launch, the message
    // typed in with its From line, the turn in the audit log.
    let launch = w.host.last_launch();
    assert_eq!(launch.shared.share, "api-expert");
    let sent = w.host.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "Claude Code called the tool once");
    assert!(sent[0].1.starts_with("From bob@example.com (Claude Code over MCP):") || sent[0].1.starts_with("From bob@example.com"), "{}", sent[0].1);
    assert!(w.collab.audit_log().lines().iter().any(|l| l.method == "turn" && l.outcome == "done"));
    assert!(stdout.contains("refresh.rs"), "the reply came back");
}

#[test]
#[ignore = "needs the A2A Python SDK: PITWALL_A2A_PYTHON=<a venv's python with a2a-sdk>"]
fn interop_a2a_sdk() {
    let python = std::env::var("PITWALL_A2A_PYTHON").expect("PITWALL_A2A_PYTHON: a python with a2a-sdk installed");
    let (w, base, log) = serve_the_core();
    let dir = crate::testing::TempDir::new("interop-a2a");
    let script = dir.path().join("client.py");
    std::fs::write(&script, A2A_CLIENT).unwrap();
    let card_url = format!("{base}/a2a/api-expert");
    let mut results = Vec::new();
    for streaming in ["stream", "send"] {
        let out = std::process::Command::new(&python)
            .arg("-I")
            .arg(&script)
            .args([card_url.as_str(), streaming, "Where are refresh tokens rotated?"])
            .output()
            .expect("run the A2A client");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        eprintln!("--- a2a-sdk ({streaming}) stdout\n{stdout}\n--- stderr\n{}", String::from_utf8_lossy(&out.stderr));
        results.push((streaming, out.status.success(), stdout));
    }
    print_log(&log);
    for (mode, ok, stdout) in &results {
        assert!(ok, "the {mode} client failed");
        let last: Value = serde_json::from_str(stdout.lines().last().unwrap_or("{}")).unwrap_or_default();
        assert_eq!(last["state"], "TASK_STATE_COMPLETED", "{mode}: {stdout}");
        assert!(last["reply"].as_str().unwrap_or_default().contains("refresh.rs"), "{mode}: {stdout}");
    }
    assert_eq!(w.host.sent.lock().unwrap().len(), 2, "two messages reached the core");
}

/// An A2A client written against the official SDK only: resolve the card
/// (`<interface>/.well-known/agent-card.json`), make a client from it (the
/// card's own HTTP+JSON interface), send one message, print the events and
/// finally `{"state", "reply"}` as JSON.
const A2A_CLIENT: &str = r#"
import asyncio, json, sys, uuid
import httpx
from a2a.client import A2ACardResolver, ClientConfig, ClientFactory
from a2a.types import Message, Part, Role, SendMessageRequest
from google.protobuf.json_format import MessageToDict

async def main(base, mode, text):
    async with httpx.AsyncClient(timeout=120) as http:
        card = await A2ACardResolver(http, base).get_agent_card()
        print("card:", json.dumps(MessageToDict(card))[:400])
        config = ClientConfig(streaming=(mode == "stream"), httpx_client=http, supported_protocol_bindings=["HTTP+JSON"])
        client = ClientFactory(config).create(card)
        msg = Message(role=Role.ROLE_USER, message_id=str(uuid.uuid4()), parts=[Part(text=text)])
        state, reply = None, None
        async for event in client.send_message(SendMessageRequest(message=msg)):
            d = MessageToDict(event)
            print("event:", json.dumps(d)[:400])
            task = d.get("task") or {}
            status = (d.get("statusUpdate") or {}).get("status") or task.get("status") or {}
            state = status.get("state", state)
            for a in task.get("artifacts", []) + [(d.get("artifactUpdate") or {}).get("artifact") or {}]:
                for p in a.get("parts", []):
                    if "text" in p:
                        reply = p["text"]
            if d.get("message"):
                for p in d["message"].get("parts", []):
                    if "text" in p:
                        reply = p["text"]
        if state != "TASK_STATE_COMPLETED" and task.get("id"):
            t = MessageToDict(await client.get_task(__import__("a2a.types", fromlist=["GetTaskRequest"]).GetTaskRequest(id=task["id"])))
            state = (t.get("status") or {}).get("state", state)
        print(json.dumps({"state": state, "reply": reply}))

asyncio.run(main(sys.argv[1], sys.argv[2], sys.argv[3]))
"#;

#[test]
#[ignore = "needs the MCP Inspector: PITWALL_MCP_INSPECTOR=<its node_modules/.bin/mcp-inspector>"]
fn interop_mcp_inspector() {
    let cli = std::env::var("PITWALL_MCP_INSPECTOR").expect("PITWALL_MCP_INSPECTOR: the Inspector's launcher (node_modules/.bin/mcp-inspector)");
    let (w, base, log) = serve_the_core();
    let url = format!("{base}/mcp");
    let dir = crate::testing::TempDir::new("interop-inspector");
    // Its catalog in a folder of its own, not under the user's home.
    let catalog = dir.path().join("catalog.json");
    let run = |args: &[&str]| {
        let out = std::process::Command::new("node")
            .arg(&cli)
            .args(["--cli", url.as_str(), "--transport", "http"])
            .args(args)
            .env("MCP_CATALOG_PATH", &catalog)
            .env("HOME", dir.path())
            .output()
            .expect("node on PATH");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        eprintln!("--- inspector {args:?}\n{stdout}\n--- stderr\n{}", String::from_utf8_lossy(&out.stderr));
        (out.status.success(), stdout)
    };
    let (ok, tools) = run(&["--method", "tools/list"]);
    let (ok2, called) = run(&["--method", "tools/call", "--tool-name", "ask_api_expert", "--tool-arg", "message=Where are refresh tokens rotated?"]);
    print_log(&log);
    assert!(ok && tools.contains("ask_api_expert"), "tools/list: {tools}");
    assert!(ok2 && called.contains("refresh.rs"), "tools/call: {called}");
    assert_eq!(w.host.sent.lock().unwrap().len(), 1);
}
