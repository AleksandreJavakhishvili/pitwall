//! The `net` permission's hosts (docs/spec/tile-sdk/permissions.md §5.1):
//! what a manifest may list, and whether a request's address is one of
//! them. The tile host parses each URL (IDNA, lower case) before asking
//! [`allows`]; this module only compares.
//!
//! - A host is an exact name (`api.github.com`) or every name under one
//!   (`*.github.com`: `a.github.com`, `a.b.github.com`, not `github.com`
//!   itself), maybe with a port (`api.example.com:8443`). Without a port,
//!   only the scheme's own (443 for https, 80 for http).
//! - Names are written as a browser sends them: lower case, and a name in
//!   another script in its `xn--` form.
//! - No IP addresses, except this computer's (`127.0.0.1`, `[::1]`).
//! - `https` to any listed host; plain `http` only to this computer
//!   (`localhost`, `127.0.0.1`, `[::1]`).

/// The most hosts one `net` permission lists.
pub const MAX_HOSTS: usize = 32;

/// One host a manifest's `net` permission lists, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPattern {
    /// `*.example.com`: names under `host`, not `host` itself.
    pub wildcard: bool,
    /// The name, lower case, without `*.`; an IPv6 address without its
    /// brackets.
    pub host: String,
    /// The port, when it names one.
    pub port: Option<u16>,
}

impl HostPattern {
    /// As the manifest wrote it.
    pub fn written(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let star = if self.wildcard { "*." } else { "" };
        match self.port {
            Some(p) => format!("{star}{host}:{p}"),
            None => format!("{star}{host}"),
        }
    }
}

/// Whether `host` (lower case, an IPv6 address without brackets) is this
/// computer.
pub fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// Why `s` can't be one of `net`'s hosts.
pub fn parse_host(s: &str) -> Result<HostPattern, String> {
    let bad = |why: &str| Err(format!("“{s}” {why}"));
    if s.is_empty() || s.len() > 253 {
        return bad("isn't a host name");
    }
    if s.contains("://") || s.contains('/') {
        return bad("must be a host name alone, without a scheme or a path (like api.example.com)");
    }
    let (wildcard, rest) = match s.strip_prefix("*.") {
        Some(r) => (true, r),
        None => (false, s),
    };
    // [::1]:port, [::1], name:port, name.
    let (host, port) = if let Some(r) = rest.strip_prefix('[') {
        let Some((h, after)) = r.split_once(']') else { return bad("has a [ without its ]") };
        let port = match after {
            "" => None,
            p => match p.strip_prefix(':') {
                Some(p) => Some(p),
                None => return bad("has something after its ]"),
            },
        };
        (h.to_string(), port)
    } else {
        match rest.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (rest.to_string(), None),
        }
    };
    let port = match port {
        None => None,
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 && p.chars().all(|c| c.is_ascii_digit()) => Some(n),
            _ => return bad("has a port that isn't a number from 1 to 65535"),
        },
    };
    if host.contains(':') || host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        // An IP address: only this computer's.
        if wildcard || !is_loopback(&host) {
            return bad("is an IP address: list host names (only this computer's 127.0.0.1 and [::1] may be addresses)");
        }
        return Ok(HostPattern { wildcard, host, port });
    }
    if host.chars().any(|c| c.is_ascii_uppercase()) {
        return bad("must be lower case");
    }
    if !host.is_ascii() {
        return bad("must be written as browsers send it: a name in another script in its xn-- form");
    }
    let labels: Vec<&str> = host.split('.').collect();
    let ok_label = |l: &str| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !labels.iter().all(|l| ok_label(l)) {
        return bad("isn't a host name (letters, digits and -, in parts joined by dots)");
    }
    if labels.len() < 2 && host != "localhost" {
        return bad("must be a whole name, like api.example.com");
    }
    if wildcard && labels.len() < 2 {
        return bad("covers too much: *. must come before a whole name, like *.example.com");
    }
    Ok(HostPattern { wildcard, host, port })
}

/// The hosts of a `net` permission's `hosts` list, checked: at least one,
/// at most [`MAX_HOSTS`], none twice.
pub fn parse_hosts(list: &[&str]) -> Result<Vec<HostPattern>, String> {
    if list.is_empty() {
        return Err("permission “net” needs at least one host in \"hosts\"".into());
    }
    if list.len() > MAX_HOSTS {
        return Err(format!("permission “net” lists more than {MAX_HOSTS} hosts"));
    }
    let mut out: Vec<HostPattern> = Vec::new();
    for s in list {
        let p = parse_host(s).map_err(|e| format!("permission “net”: {e}"))?;
        if out.contains(&p) {
            return Err(format!("permission “net” lists “{s}” twice"));
        }
        out.push(p);
    }
    Ok(out)
}

/// Whether a request to `scheme://host:port` may go out under `hosts`
/// (the URL already parsed: `host` lower case, IDNA applied, an IPv6
/// address without brackets). `https` to a listed host; `http` only to
/// this computer, and only when listed too.
pub fn allows(hosts: &[HostPattern], scheme: &str, host: &str, port: u16) -> bool {
    let default = match scheme {
        "https" => 443,
        "http" if is_loopback(host) => 80,
        _ => return false,
    };
    hosts.iter().any(|p| {
        let name = if p.wildcard { host.strip_suffix(p.host.as_str()).is_some_and(|sub| sub.len() > 1 && sub.ends_with('.')) } else { host == p.host };
        name && p.port.unwrap_or(default) == port
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(list: &[&str]) -> Vec<HostPattern> {
        parse_hosts(list).unwrap()
    }

    #[test]
    fn host_names_ports_and_wildcards() {
        assert_eq!(parse_host("api.github.com").unwrap(), HostPattern { wildcard: false, host: "api.github.com".into(), port: None });
        assert_eq!(parse_host("*.example.com:8443").unwrap(), HostPattern { wildcard: true, host: "example.com".into(), port: Some(8443) });
        assert_eq!(parse_host("[::1]:8790").unwrap().written(), "[::1]:8790");
        assert_eq!(parse_host("localhost:3000").unwrap().port, Some(3000));
        assert_eq!(parse_host("127.0.0.1").unwrap().host, "127.0.0.1");
        for bad in ["https://api.github.com", "api.github.com/v3", "API.github.com", "10.0.0.1", "*.127.0.0.1", "[fe80::1]", "*.com", "com", "bücher.de", "a..b", "-a.com", "a.com:0", "a.com:99999", "a.com:x", ""] {
            assert!(parse_host(bad).is_err(), "{bad} should be refused");
        }
        assert!(parse_host("xn--bcher-kva.de").is_ok(), "punycode is how it is written");
        assert!(parse_hosts(&[]).is_err() && parse_hosts(&["a.com", "a.com"]).is_err());
    }

    #[test]
    fn what_a_request_may_reach() {
        let h = hosts(&["api.github.com", "*.example.com", "localhost:8790", "127.0.0.1"]);
        assert!(allows(&h, "https", "api.github.com", 443));
        assert!(!allows(&h, "https", "api.github.com", 8443), "another port");
        assert!(!allows(&h, "http", "api.github.com", 80), "plain http only to this computer");
        assert!(!allows(&h, "https", "github.com", 443));
        assert!(!allows(&h, "https", "evilapi.github.com", 443));
        assert!(allows(&h, "https", "a.b.example.com", 443));
        assert!(!allows(&h, "https", "example.com", 443), "*. is the names under it");
        assert!(!allows(&h, "https", "badexample.com", 443));
        assert!(allows(&h, "http", "localhost", 8790));
        assert!(!allows(&h, "http", "localhost", 8791));
        assert!(allows(&h, "http", "127.0.0.1", 80));
        assert!(!allows(&h, "ftp", "api.github.com", 21));
        assert!(!allows(&h, "http", "::1", 80), "not listed");
    }
}
