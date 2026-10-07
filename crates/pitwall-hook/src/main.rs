//! `pitwall-hook`: Pitwall's hook relay. Agents (Claude Code `--settings`
//! hooks, Codex `hooks.json`) run it with the hook payload on stdin; it posts
//! that payload to `http://pitwall/hook/<PITWALL_AGENT_ID>` over the local
//! socket in `PITWALL_SOCKET` (a Unix socket, or on Windows the named pipe
//! derived from that path).
//!
//! Behaves exactly like the sh + curl script it replaces on Windows: never
//! prints, always exits 0, does nothing outside Pitwall (either variable
//! unset or nothing listening), and gives up after 2 seconds.

use std::io::{Read, Write};
use std::time::Duration;

/// The whole relay may take this long; then it leaves quietly.
const DEADLINE: Duration = Duration::from_secs(2);

fn main() {
    std::thread::spawn(|| {
        std::thread::sleep(DEADLINE);
        std::process::exit(0);
    });
    let mut body = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut body);
    let (id, socket) = (std::env::var("PITWALL_AGENT_ID").unwrap_or_default(), std::env::var_os("PITWALL_SOCKET").unwrap_or_default());
    if let Some(request) = request(&id, &body) {
        if !socket.is_empty() {
            let _ = post(std::path::Path::new(&socket), &request);
        }
    }
    std::process::exit(0);
}

/// The HTTP request for agent `id`, or `None` when `id` can't be one.
fn request(id: &str, body: &[u8]) -> Option<Vec<u8>> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let mut req = format!(
        "POST /hook/{id} HTTP/1.1\r\nHost: pitwall\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    req.extend_from_slice(body);
    Some(req)
}

fn post(socket: &std::path::Path, request: &[u8]) -> std::io::Result<()> {
    let mut conn = connect(socket)?;
    conn.write_all(request)?;
    conn.flush()?;
    // Wait for the answer, so Pitwall has the whole request before we go.
    let mut reply = [0u8; 256];
    let _ = conn.read(&mut reply);
    Ok(())
}

#[cfg(unix)]
fn connect(socket: &std::path::Path) -> std::io::Result<std::os::unix::net::UnixStream> {
    std::os::unix::net::UnixStream::connect(socket)
}

#[cfg(windows)]
fn connect(
    socket: &std::path::Path,
) -> std::io::Result<interprocess::os::windows::named_pipe::DuplexPipeStream<interprocess::os::windows::named_pipe::pipe_mode::Bytes>> {
    use interprocess::os::windows::named_pipe::{pipe_mode, DuplexPipeStream};
    let name = pipe_name(&socket.to_string_lossy());
    DuplexPipeStream::<pipe_mode::Bytes>::connect_by_path_with_wait_mode(name.as_str(), interprocess::ConnectWaitMode::Timeout(DEADLINE))
}

/// A copy of `pitwall_proto::pipe::pipe_name` (kept dependency-free; same
/// test vectors).
#[cfg_attr(not(windows), allow(dead_code))]
fn pipe_name(path: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_request() {
        let req = String::from_utf8(request("a1-b2", b"{\"x\":1}").unwrap()).unwrap();
        assert!(req.starts_with("POST /hook/a1-b2 HTTP/1.1\r\n"));
        assert!(req.contains("Content-Length: 7\r\n"));
        assert!(req.ends_with("\r\n\r\n{\"x\":1}"));
        assert_eq!(request("", b"{}"), None);
        assert_eq!(request("../x", b"{}"), None);
        assert_eq!(request("a b", b"{}"), None);
    }

    #[test]
    fn shared_pipe_name_vectors() {
        assert_eq!(pipe_name(r"C:\Users\Dev\AppData\Roaming\Pitwall\run\pitwall.sock"), r"\\.\pipe\pitwall-743ad10ab31a747c-pitwall");
        assert_eq!(pipe_name("C:/Users/Dev/AppData/Roaming/Pitwall/run/hold/a1.sock"), r"\\.\pipe\pitwall-98930f412d88a184-a1");
    }
}
