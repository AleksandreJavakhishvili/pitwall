//! The named pipe a Pitwall endpoint path maps to on Windows (Pitwall names
//! its sockets by path: `run/pitwall.sock`, `run/pitwalld.sock`, …). Pure
//! string logic, so it is tested on every OS.
//!
//! Frozen: used by pitwall-core (hook and CLI sockets), pitwall-client and
//! pitwall-hook; `pitwall-hold` (which has no Pitwall dependencies) keeps an
//! identical copy for its holders. Both copies share the test vectors below. The path is per user (`%APPDATA%\Pitwall\…`), so two users
//! never share a name; the stem keeps names readable in pipe listings.

/// `\\.\pipe\pitwall-<FNV-1a 64 of the normalised path>-<file stem>`.
pub fn pipe_name(path: &str) -> String {
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
    fn shared_test_vectors() {
        assert_eq!(pipe_name(r"C:\Users\Dev\AppData\Roaming\Pitwall\run\pitwall.sock"), r"\\.\pipe\pitwall-743ad10ab31a747c-pitwall");
        assert_eq!(pipe_name("C:/Users/Dev/AppData/Roaming/Pitwall/run/hold/a1.sock"), r"\\.\pipe\pitwall-98930f412d88a184-a1");
        // Case and separators don't matter (Windows paths are case-insensitive).
        assert_eq!(pipe_name(r"c:\users\dev\appdata\roaming\pitwall\run\hold\A1.sock"), pipe_name("C:/Users/Dev/AppData/Roaming/Pitwall/run/hold/a1.sock"));
    }
}
