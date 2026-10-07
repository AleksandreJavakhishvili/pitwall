//! The real `pitwall-hook` binary: silent and exit 0 outside Pitwall, and
//! inside it the payload reaches the hook socket as `POST /hook/<id>`.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_pitwall-hook");

fn run(env: &[(&str, &str)], stdin: &[u8]) -> std::process::Output {
    let mut cmd = Command::new(BIN);
    cmd.env_remove("PITWALL_AGENT_ID").env_remove("PITWALL_SOCKET");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn silent_outside_pitwall_and_without_a_listener() {
    for env in [vec![], vec![("PITWALL_AGENT_ID", "a1")], vec![("PITWALL_AGENT_ID", "a1"), ("PITWALL_SOCKET", "/nonexistent/pw.sock")]] {
        let out = run(&env, br#"{"hook_event_name":"Stop"}"#);
        assert!(out.status.success(), "{env:?}");
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{env:?}");
    }
}

fn socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("pwk-{}.sock", std::process::id()))
}

/// Accept one connection, answer like Pitwall, return what was sent.
fn serve_one(path: &std::path::Path) -> std::thread::JoinHandle<Vec<u8>> {
    #[cfg(unix)]
    let listener = {
        let _ = std::fs::remove_file(path);
        std::os::unix::net::UnixListener::bind(path).unwrap()
    };
    #[cfg(windows)]
    let listener = {
        use interprocess::os::windows::named_pipe::{pipe_mode, PipeListenerOptions, PipeMode};
        let name = windows_pipe_name(&path.to_string_lossy());
        PipeListenerOptions::new().path(name.as_str()).mode(PipeMode::Bytes).create_duplex::<pipe_mode::Bytes>().unwrap()
    };
    std::thread::spawn(move || {
        #[cfg(unix)]
        let mut conn = listener.accept().unwrap().0;
        #[cfg(windows)]
        let mut conn = listener.accept().unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while !String::from_utf8_lossy(&got).contains("\"Stop\"}") {
            let n = conn.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        conn.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n").unwrap();
        got
    })
}

#[cfg(windows)]
fn windows_pipe_name(path: &str) -> String {
    let norm = path.replace('/', "\\").to_lowercase();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in norm.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let stem = norm.rsplit('\\').next().unwrap_or("");
    let stem = stem.strip_suffix(".sock").unwrap_or(stem);
    let stem: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    format!(r"\\.\pipe\pitwall-{h:016x}-{stem}")
}

#[test]
fn posts_the_payload_to_the_hook_socket() {
    let path = socket_path();
    let server = serve_one(&path);
    let socket = path.to_string_lossy().into_owned();
    let out = run(&[("PITWALL_AGENT_ID", "agent-7"), ("PITWALL_SOCKET", &socket)], br#"{"hook_event_name":"Stop"}"#);
    let got = String::from_utf8(server.join().unwrap()).unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success() && out.stdout.is_empty());
    assert!(got.starts_with("POST /hook/agent-7 HTTP/1.1\r\n"), "{got}");
    assert!(got.ends_with(r#"{"hook_event_name":"Stop"}"#), "{got}");
}
