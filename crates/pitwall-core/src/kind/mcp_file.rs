//! Pitwall's entry in a tool's own MCP file ([`McpFile`]), for tools with
//! no launch-time way to add a server: added and removed only when the
//! user asks (Settings → Agents, `pitwall-cli show-files`). The edit is
//! made in the text itself, so everything else in the file (order,
//! spacing, comments) stays byte for byte; a backup of the file is kept
//! next to it before the first change.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::McpFile;
use crate::provider::McpWiring;

const SERVERS: &str = "mcpServers";

/// Pitwall's entry in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Entry {
    /// Not there (or no file).
    Absent,
    /// There, running this Pitwall's CLI.
    Added,
    /// There, but running another command (an older Pitwall's).
    Stale,
}

/// Where the file is (`~/` expanded).
pub fn location(f: &McpFile) -> PathBuf {
    crate::paths::expand_tilde(&f.path)
}

/// Pitwall's entry in the file at `at`. Err: the file can't be read or
/// isn't a JSON object.
pub fn status_at(at: &Path, f: &McpFile, m: &McpWiring) -> Result<Entry, String> {
    let text = match read(at)? {
        None => return Ok(Entry::Absent),
        Some(t) => t,
    };
    let doc = Doc::parse(&text)?;
    let Some(servers) = doc.servers(&text)? else { return Ok(Entry::Absent) };
    let Some(mine) = servers.members.iter().find(|x| x.key == crate_name()) else { return Ok(Entry::Absent) };
    let value: serde_json::Value = json5ish(&text[mine.value.0..mine.value.1]);
    let same = value.get("command").and_then(|c| c.as_str()) == Some(m.command.as_str())
        && value.get("args") == Some(&serde_json::json!(m.args))
        && f.entry.iter().all(|(k, v)| value.get(k) == Some(v));
    Ok(if same { Entry::Added } else { Entry::Stale })
}

/// Add (or bring up to date) Pitwall's entry. Returns the file changed.
pub fn add_at(at: &Path, f: &McpFile, m: &McpWiring) -> Result<(), String> {
    let entry = entry_json(f, m);
    let old = read(at)?;
    let text = match &old {
        None => fresh(&entry),
        Some(t) if t.trim().is_empty() => fresh(&entry),
        Some(t) => with_entry(t, &entry)?,
    };
    write(at, old.as_deref(), &text)
}

/// Take Pitwall's entry out again, and `mcpServers` when Pitwall added it
/// (the backup has none); the file itself when Pitwall made it (no backup)
/// and nothing else is in it.
pub fn remove_at(at: &Path) -> Result<(), String> {
    let Some(old) = read(at)? else { return Ok(()) };
    let Some(mut text) = without_entry(&old)? else { return Ok(()) };
    let backup = read(&backup_of(at)).ok().flatten();
    let had_servers = backup.as_deref().map(|b| Doc::parse(b).is_ok_and(|d| d.top.members.iter().any(|x| x.key == SERVERS)));
    if had_servers != Some(true) {
        text = without_empty_servers(&text)?;
    }
    if backup.is_none() && serde_json::from_str::<serde_json::Value>(&text).ok() == Some(serde_json::json!({})) {
        return std::fs::remove_file(at).map_err(|e| format!("couldn't remove {}: {e}", at.display()));
    }
    write(at, Some(&old), &text)
}

fn backup_of(at: &Path) -> PathBuf {
    let mut name = at.file_name().unwrap_or_default().to_os_string();
    name.push(".pitwall-backup");
    at.with_file_name(name)
}

fn crate_name() -> &'static str {
    pitwall_proto::show::MCP_SERVER
}

/// `{"command": …, "args": […], <the kind's own fields>}` on one line.
pub fn entry_json(f: &McpFile, m: &McpWiring) -> String {
    let mut parts = vec![
        format!(r#""command": {}"#, serde_json::to_string(&m.command).unwrap_or_default()),
        format!(r#""args": {}"#, serde_json::to_string(&m.args).unwrap_or_default()),
    ];
    for (k, v) in &f.entry {
        parts.push(format!("{}: {}", serde_json::to_string(k).unwrap_or_default(), v));
    }
    format!("{{{}}}", parts.join(", "))
}

fn fresh(entry: &str) -> String {
    format!("{{\n  \"{SERVERS}\": {{\n    \"{}\": {entry}\n  }}\n}}\n", crate_name())
}

fn read(at: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(at) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", at.display())),
    }
}

/// Writes in place (a symlinked dotfile stays a link), after a backup of
/// what was there.
fn write(at: &Path, old: Option<&str>, text: &str) -> Result<(), String> {
    if let Some(old) = old {
        if old == text {
            return Ok(());
        }
        std::fs::write(backup_of(at), old).map_err(|e| format!("couldn't back up {}: {e}", at.display()))?;
    } else if let Some(dir) = at.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    }
    std::fs::write(at, text).map_err(|e| format!("couldn't write {}: {e}", at.display()))
}

/// The file with Pitwall's entry in `mcpServers` (replacing an older one).
fn with_entry(text: &str, entry: &str) -> Result<String, String> {
    let doc = Doc::parse(text)?;
    let key = format!("\"{}\"", crate_name());
    let mut out = text.to_string();
    match doc.servers(text)? {
        Some(servers) => match servers.members.iter().find(|x| x.key == crate_name()) {
            Some(mine) => out.replace_range(mine.value.0..mine.value.1, entry),
            None => {
                let at = servers.members.last().map_or(servers.open + 1, |l| l.value.1);
                let indent = member_indent(text, &servers, &doc.top);
                let sep = if servers.members.is_empty() { "" } else { "," };
                let close = if servers.members.is_empty() { format!("\n{}", line_indent(text, servers.close)) } else { String::new() };
                out.insert_str(at, &format!("{sep}\n{indent}{key}: {entry}{close}"));
            }
        },
        None => {
            let top = &doc.top;
            let indent = top.members.first().map_or("  ".to_string(), |x| line_indent(text, x.start));
            let at = top.members.last().map_or(top.open + 1, |l| l.value.1);
            let sep = if top.members.is_empty() { "" } else { "," };
            let close = if top.members.is_empty() { "\n" } else { "" };
            out.insert_str(at, &format!("{sep}\n{indent}\"{SERVERS}\": {{\n{indent}{indent}{key}: {entry}\n{indent}}}{close}"));
        }
    }
    Ok(out)
}

/// The file without Pitwall's entry; None: it isn't there.
fn without_entry(text: &str) -> Result<Option<String>, String> {
    let doc = Doc::parse(text)?;
    let Some(servers) = doc.servers(text)? else { return Ok(None) };
    let Some(i) = servers.members.iter().position(|x| x.key == crate_name()) else { return Ok(None) };
    let (from, to) = span(&servers, i);
    let mut out = text.to_string();
    out.replace_range(from..to, "");
    Ok(Some(out))
}

/// The file without an empty `mcpServers`.
fn without_empty_servers(text: &str) -> Result<String, String> {
    let doc = Doc::parse(text)?;
    let mut out = text.to_string();
    if doc.servers(text)?.is_some_and(|s| s.members.is_empty()) {
        let j = doc.top.members.iter().position(|x| x.key == SERVERS).unwrap_or_default();
        let (from, to) = span(&doc.top, j);
        out.replace_range(from..to, "");
    }
    Ok(out)
}

/// The text to cut for member `i` and its comma: from the end of the one
/// before it, else up to the start of the one after it, else everything
/// between the braces (what adding it put there).
fn span(obj: &Obj, i: usize) -> (usize, usize) {
    let m = &obj.members[i];
    if i > 0 {
        (obj.members[i - 1].value.1, m.value.1)
    } else if let Some(next) = obj.members.get(1) {
        (m.start, next.start)
    } else {
        (obj.open + 1, obj.close)
    }
}

fn line_indent(text: &str, at: usize) -> String {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    text[start..at].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// The indent for a new member of `servers`: its members', else one step
/// in from `mcpServers`'s own line.
fn member_indent(text: &str, servers: &Obj, top: &Obj) -> String {
    if let Some(first) = servers.members.first() {
        return line_indent(text, first.start);
    }
    let step = top.members.first().map_or("  ".to_string(), |x| line_indent(text, x.start));
    format!("{}{step}", line_indent(text, servers.open))
}

/// Parse a JSON (or JSON with comments) value for comparing; anything else
/// is Null.
fn json5ish(src: &str) -> serde_json::Value {
    serde_json::from_str(src).unwrap_or(serde_json::Value::Null)
}

/// One `"key": value` of an object: where its key starts, the value's
/// span.
#[derive(Debug)]
struct Member {
    key: String,
    start: usize,
    value: (usize, usize),
}

#[derive(Debug)]
struct Obj {
    open: usize,
    close: usize,
    members: Vec<Member>,
}

struct Doc {
    top: Obj,
}

impl Doc {
    fn parse(text: &str) -> Result<Doc, String> {
        let b = text.as_bytes();
        let i = skip_ws(b, 0);
        if b.get(i) != Some(&b'{') {
            return Err("isn't a JSON object".into());
        }
        let top = object(b, i)?;
        if skip_ws(b, top.close + 1) != b.len() {
            return Err("has more after its JSON object".into());
        }
        Ok(Doc { top })
    }

    /// `mcpServers`, when there is one. Err: it isn't an object.
    fn servers(&self, text: &str) -> Result<Option<Obj>, String> {
        let Some(m) = self.top.members.iter().find(|x| x.key == SERVERS) else { return Ok(None) };
        let b = text.as_bytes();
        if b[m.value.0] != b'{' {
            return Err(format!("its \"{SERVERS}\" isn't an object"));
        }
        object(b, m.value.0).map(Some)
    }
}

const BAD: &str = "isn't valid JSON";

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    loop {
        match b.get(i) {
            Some(b' ' | b'\t' | b'\n' | b'\r') => i += 1,
            Some(b'/') if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            Some(b'/') if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
            }
            _ => return i,
        }
    }
}

/// The string starting at `i` (a quote): its text and the index after it.
fn string(b: &[u8], i: usize) -> Result<(String, usize), String> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => {
                let raw = std::str::from_utf8(&b[i..=j]).map_err(|_| BAD)?;
                let s: String = serde_json::from_str(raw).map_err(|_| BAD)?;
                return Ok((s, j + 1));
            }
            _ => j += 1,
        }
    }
    Err(BAD.into())
}

/// The value starting at `i`: the index after it.
fn value(b: &[u8], i: usize) -> Result<usize, String> {
    match b.get(i) {
        Some(b'"') => string(b, i).map(|(_, e)| e),
        Some(b'{') => object(b, i).map(|o| o.close + 1),
        Some(b'[') => {
            let mut j = skip_ws(b, i + 1);
            if b.get(j) == Some(&b']') {
                return Ok(j + 1);
            }
            loop {
                j = skip_ws(b, value(b, j)?);
                match b.get(j) {
                    Some(b',') => {
                        j = skip_ws(b, j + 1);
                        if b.get(j) == Some(&b']') {
                            return Ok(j + 1);
                        }
                    }
                    Some(b']') => return Ok(j + 1),
                    _ => return Err(BAD.into()),
                }
            }
        }
        Some(_) => {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'-' | b'+' | b'.')) {
                j += 1;
            }
            if j == i {
                Err(BAD.into())
            } else {
                Ok(j)
            }
        }
        None => Err(BAD.into()),
    }
}

fn object(b: &[u8], open: usize) -> Result<Obj, String> {
    let mut members = vec![];
    let mut j = skip_ws(b, open + 1);
    loop {
        match b.get(j) {
            Some(b'}') => return Ok(Obj { open, close: j, members }),
            Some(b'"') => {
                let (key, after) = string(b, j)?;
                let k = skip_ws(b, after);
                if b.get(k) != Some(&b':') {
                    return Err(BAD.into());
                }
                let v = skip_ws(b, k + 1);
                let end = value(b, v)?;
                members.push(Member { key, start: j, value: (v, end) });
                j = skip_ws(b, end);
                match b.get(j) {
                    Some(b',') => j = skip_ws(b, j + 1),
                    Some(b'}') => {}
                    _ => return Err(BAD.into()),
                }
            }
            _ => return Err(BAD.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiring() -> McpWiring {
        McpWiring { command: "/h/.pitwall/bin/pitwall-cli".into(), args: vec!["mcp".into()], tools: Vec::new() }
    }

    fn file(extra: &str) -> McpFile {
        toml::from_str(&format!("path = \"~/x.json\"\n{extra}")).unwrap()
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pw-mcpfile-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("mcp.json")
    }

    #[test]
    fn a_new_file_is_only_pitwalls_and_goes_again_when_removed() {
        let at = tmp("new");
        let f = file("entry = { type = \"stdio\" }");
        assert_eq!(status_at(&at, &f, &wiring()).unwrap(), Entry::Absent);
        add_at(&at, &f, &wiring()).unwrap();
        let text = std::fs::read_to_string(&at).unwrap();
        assert_eq!(text, "{\n  \"mcpServers\": {\n    \"pitwall\": {\"command\": \"/h/.pitwall/bin/pitwall-cli\", \"args\": [\"mcp\"], \"type\": \"stdio\"}\n  }\n}\n");
        assert_eq!(status_at(&at, &f, &wiring()).unwrap(), Entry::Added);
        remove_at(&at).unwrap();
        assert!(!at.exists());
    }

    #[test]
    fn the_rest_of_the_file_is_kept_byte_for_byte() {
        let cases = [
            // Other servers, two-space indent.
            "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"github\": {\"command\": \"gh\"}\n  }\n}\n",
            // Tabs, comments, no trailing newline.
            "{\n\t// mine\n\t\"mcpServers\": {\n\t\t\"a\": {\"command\": \"a\", \"args\": [\"x\", 1, true]},\n\t\t\"b\": {}\n\t}\n}",
            // No mcpServers yet.
            "{\n    \"general\": {\"vimMode\": true}\n}\n",
            // An empty object.
            "{}\n",
            // mcpServers empty.
            "{\"mcpServers\": {}}",
        ];
        let f = file("entry = { trust = true }");
        for (n, before) in cases.iter().enumerate() {
            let at = tmp(&format!("keep{n}"));
            std::fs::write(&at, before).unwrap();
            add_at(&at, &f, &wiring()).unwrap();
            let added = std::fs::read_to_string(&at).unwrap();
            let v: serde_json::Value = serde_json::from_str(&strip_comments(&added)).unwrap_or_else(|e| panic!("case {n}: {e}\n{added}"));
            assert_eq!(v[SERVERS]["pitwall"]["command"], "/h/.pitwall/bin/pitwall-cli", "case {n}");
            assert_eq!(v[SERVERS]["pitwall"]["trust"], true, "case {n}");
            assert_eq!(status_at(&at, &f, &wiring()).unwrap(), Entry::Added, "case {n}");
            // Adding again changes nothing.
            add_at(&at, &f, &wiring()).unwrap();
            assert_eq!(std::fs::read_to_string(&at).unwrap(), added, "case {n}");
            assert_eq!(std::fs::read_to_string(at.with_file_name("mcp.json.pitwall-backup")).unwrap(), *before, "case {n}");
            remove_at(&at).unwrap();
            assert_eq!(std::fs::read_to_string(&at).unwrap(), *before, "case {n}");
        }
    }

    #[test]
    fn an_older_entry_is_brought_up_to_date_in_place() {
        let at = tmp("stale");
        let before = "{\"mcpServers\": {\"pitwall\": {\"command\": \"/Applications/Pitwall.app/x\", \"args\": [\"mcp\"]}, \"z\": {}}}";
        std::fs::write(&at, before).unwrap();
        let f = file("");
        assert_eq!(status_at(&at, &f, &wiring()).unwrap(), Entry::Stale);
        add_at(&at, &f, &wiring()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&at).unwrap(),
            "{\"mcpServers\": {\"pitwall\": {\"command\": \"/h/.pitwall/bin/pitwall-cli\", \"args\": [\"mcp\"]}, \"z\": {}}}"
        );
        remove_at(&at).unwrap();
        assert_eq!(std::fs::read_to_string(&at).unwrap(), "{\"mcpServers\": {\"z\": {}}}");
    }

    #[test]
    fn a_file_that_isnt_an_object_is_left_alone() {
        for (n, bad) in ["[1]", "{\"mcpServers\": []}", "{\"a\": ", "not json"].iter().enumerate() {
            let at = tmp(&format!("bad{n}"));
            std::fs::write(&at, bad).unwrap();
            assert!(add_at(&at, &file(""), &wiring()).is_err(), "case {n}");
            assert_eq!(std::fs::read_to_string(&at).unwrap(), *bad);
        }
    }

    fn strip_comments(s: &str) -> String {
        s.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
    }
}
