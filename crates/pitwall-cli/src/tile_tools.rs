//! `pitwall extension tools|commands|call|run` (agent-tools.md §11.12):
//! tiles' tools from the shell, over the same `mcp.tools` and `mcp.call`
//! agents' tool servers use. Who calls is who runs the command, as Pitwall
//! identifies the process: an agent, the user in a plain Pitwall terminal
//! (`read` runs, `act` asks them in Pitwall), anyone else only `show`.

use std::io::Read;

use pitwall_client::{Client, Error};
use pitwall_proto::mcp::{McpCall, McpCalled, McpTools};
use pitwall_proto::tile_tools::TileTool;
use pitwall_proto::{caps, method, ErrorBody};
use serde_json::{json, Map, Value};

use crate::Output;

/// A tool the tile answered with an error.
pub const TILE_ERROR: &str = "tile_error";
/// Refused: the user denied it, a switch is off, or it isn't offered here.
pub const REFUSED: &str = "refused";
/// The extension or its tile isn't on, installed or running.
pub const UNAVAILABLE: &str = "unavailable";
/// The tile didn't answer in time.
pub const TIMED_OUT: &str = "timed_out";

fn tiles(c: &mut Client) -> Result<Vec<TileTool>, Error> {
    if !c.has(caps::MCP) {
        return Err(Error::Server(ErrorBody::new(UNAVAILABLE, "This Pitwall is older than its command-line tool: quit Pitwall and open it again to update it.")));
    }
    let v = c.call(method::MCP_TOOLS, json!({}))?;
    let t: McpTools = serde_json::from_value(v).map_err(|e| Error::Server(ErrorBody::new(UNAVAILABLE, e.to_string())))?;
    Ok(t.tiles)
}

fn of(list: Vec<TileTool>, id: Option<&str>, place: &str) -> Vec<TileTool> {
    list.into_iter().filter(|t| id.is_none_or(|i| t.extension == i) && t.exposed(place)).collect()
}

fn row(t: &TileTool) -> Value {
    json!({
        "name": t.name(),
        "extension": t.extension,
        "tool": t.tool,
        "kind": t.kind,
        "title": t.title,
        "description": pitwall_proto::tile_tools::description(t),
        "input": pitwall_proto::tile_tools::input_schema(t),
        "expose": if t.expose.is_empty() { vec!["agents".to_string(), "cli".to_string()] } else { t.expose.clone() },
    })
}

/// `tools [id]`.
pub fn tools(c: &mut Client, id: Option<&str>) -> Result<Output, Error> {
    let list = of(tiles(c)?, id, "agents");
    let human: String = list.iter().map(|t| format!("{} ({})\n  {}\n", t.name(), t.kind, pitwall_proto::tile_tools::description(t))).collect();
    let rows: Vec<Value> = list.iter().map(row).collect();
    Ok(Output { json: json!({ "tools": rows }), human: if human.is_empty() { "No tile tools.\n".into() } else { human }, failed: None })
}

/// The `--<property>` arguments of `t`'s input, in words.
fn args_of(t: &TileTool) -> Vec<String> {
    let schema = pitwall_proto::tile_tools::input_schema(t);
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let Some(props) = schema.get("properties").and_then(Value::as_object) else { return Vec::new() };
    props
        .iter()
        .map(|(k, p)| {
            let ty = p.get("type").and_then(Value::as_str).unwrap_or("value");
            let need = if required.contains(&k.as_str()) { " (required)" } else { "" };
            format!("--{k} <{ty}>{need}")
        })
        .collect()
}

/// `commands [id]`.
pub fn commands(c: &mut Client, id: Option<&str>) -> Result<Output, Error> {
    let list = of(tiles(c)?, id, "cli");
    let mut human = String::new();
    let rows: Vec<Value> = list
        .iter()
        .filter_map(|t| {
            let tool = t.tool.clone()?;
            let args = args_of(t);
            human.push_str(&format!("pitwall extension run {} {tool}{}{}\n  {} ({})\n", t.extension, if args.is_empty() { "" } else { " -- " }, args.join(" "), t.description.trim(), t.kind));
            Some(json!({ "extension": t.extension, "command": tool, "kind": t.kind, "title": t.title, "description": t.description, "args": args }))
        })
        .collect();
    Ok(Output { json: json!({ "commands": rows }), human: if human.is_empty() { "No tile commands.\n".into() } else { human }, failed: None })
}

/// Text, or stdin for `-`.
fn text_or_stdin(s: &str) -> Result<String, Error> {
    if s != "-" {
        return Ok(s.to_string());
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).map_err(Error::Io)?;
    Ok(buf)
}

fn bad(msg: String) -> Error {
    Error::Server(ErrorBody::new(crate::BAD_ARGS, msg))
}

/// JSON text read as an object.
fn object(text: &str) -> Result<Value, Error> {
    match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Object(_)) => Ok(v),
        Ok(_) => Err(bad("the input must be a JSON object".into())),
        Err(e) => Err(bad(format!("the input isn't JSON: {e}"))),
    }
}

/// `--k v` pairs, each value typed by `t`'s schema (a number, a boolean,
/// JSON for arrays and objects, text otherwise).
pub fn props_input(t: &TileTool, props: &[String]) -> Result<Value, Error> {
    let schema = pitwall_proto::tile_tools::input_schema(t);
    let mut out = Map::new();
    let mut it = props.iter();
    while let Some(k) = it.next() {
        let Some(key) = k.strip_prefix("--") else { return Err(bad(format!("expected --<property>, got {k}"))) };
        let v = it.next().ok_or_else(|| bad(format!("--{key} needs a value")))?;
        let ty = schema.pointer(&format!("/properties/{key}/type")).and_then(Value::as_str).unwrap_or("string");
        let value = match ty {
            "string" => Value::String(v.clone()),
            _ => serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.clone())),
        };
        out.insert(key.to_string(), value);
    }
    Ok(Value::Object(out))
}

/// Find tool `tool` (its name or `<ext>__<name>`) of extension `id`.
fn find(c: &mut Client, id: &str, tool: &str) -> Result<TileTool, Error> {
    let list = tiles(c)?;
    let name = if tool.contains("__") { tool.to_string() } else { pitwall_proto::tile_tools::mcp_name(id, Some(tool)) };
    if !list.iter().any(|t| t.extension == id) {
        return Err(Error::Server(ErrorBody::new(UNAVAILABLE, format!("No extension “{id}” with tiles for agents is on in Pitwall (see `pitwall extension list`)."))));
    }
    list.into_iter().find(|t| t.name() == name).ok_or_else(|| Error::Server(ErrorBody::new(UNAVAILABLE, format!("{id} has no tool “{tool}” (see `pitwall extension tools {id}`)."))))
}

/// Call `t` with `input` as the CLI; the MCP result.
fn call(c: &mut Client, t: &TileTool, input: Value, timeout: Option<u64>) -> Result<Value, Error> {
    let req = McpCall { name: t.name(), arguments: input, agent: crate::open::env_agent(), cli: true, timeout_ms: timeout.map(|s| s.saturating_mul(1000)) };
    let v = c.call(method::MCP_CALL, req)?;
    let called: McpCalled = serde_json::from_value(v).map_err(|e| Error::Server(ErrorBody::new(UNAVAILABLE, e.to_string())))?;
    Ok(called.result)
}

/// The words of an MCP result, and its error code when it failed.
fn outcome(result: &Value) -> (String, Option<&'static str>) {
    let text = result.pointer("/content/0/text").and_then(Value::as_str).unwrap_or("").to_string();
    if !result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
        return (text, None);
    }
    let code = if text.contains("denied") || text.contains("turned off") || text.contains("isn't offered") || text.contains("Nobody answered") || text.contains("only agents") || text.starts_with("Nothing was done: the input") {
        REFUSED
    } else if text.contains("didn't answer within") {
        TIMED_OUT
    } else if text.contains("is off or not installed") || text.contains("has no tool") || text.contains("can't take it now") || text.contains("Stopped") {
        UNAVAILABLE
    } else {
        TILE_ERROR
    };
    (text, Some(code))
}

fn finish(result: Value, json_out: Value) -> Output {
    let (text, code) = outcome(&result);
    let failed = code.map(|c| ErrorBody::new(c, text.clone()));
    Output { json: json_out, human: if text.ends_with('\n') { text } else { format!("{text}\n") }, failed }
}

/// `call <id> <tool> --input <json|->`.
pub fn call_tool(c: &mut Client, id: &str, tool: &str, input: &str, timeout: Option<u64>) -> Result<Output, Error> {
    let t = find(c, id, tool)?;
    let input = object(&text_or_stdin(input)?)?;
    let result = call(c, &t, input, timeout)?;
    Ok(finish(result.clone(), result))
}

/// `run <id> <command> [--json <text|->] [-- --k v …]`.
pub fn run(c: &mut Client, id: &str, command: &str, json_in: Option<&str>, props: &[String], timeout: Option<u64>) -> Result<Output, Error> {
    let t = find(c, id, command)?;
    let input = match json_in {
        Some(j) => object(&text_or_stdin(j)?)?,
        None => props_input(&t, props)?,
    };
    let result = call(c, &t, input, timeout)?;
    Ok(run_output(result))
}

/// [`run`]'s output for an MCP result.
fn run_output(result: Value) -> Output {
    // The result's JSON (`structuredContent`, or a JSON second block),
    // else its text.
    let data = result.get("structuredContent").cloned().or_else(|| result.pointer("/content/1/text").and_then(Value::as_str).and_then(|s| serde_json::from_str(s).ok())).unwrap_or_else(|| json!({ "text": result.pointer("/content/0/text").cloned().unwrap_or(Value::Null) }));
    finish(result, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send() -> TileTool {
        TileTool { extension: "hello".into(), extension_name: "Hello".into(), tool: Some("send_message".into()), kind: "act".into(), expose: vec!["cli".into()], input: Some(json!({ "type": "object", "required": ["text"], "properties": { "text": { "type": "string" }, "wait_ms": { "type": "integer" } } })), ..Default::default() }
    }

    #[test]
    fn properties_take_their_schemas_types() {
        let v = props_input(&send(), &["--text".into(), "42".into(), "--wait_ms".into(), "5".into()]).unwrap();
        assert_eq!(v, json!({ "text": "42", "wait_ms": 5 }));
        assert!(props_input(&send(), &["--text".into()]).is_err());
        assert_eq!(args_of(&send()), ["--text <string> (required)", "--wait_ms <integer>"]);
    }

    /// Exit statuses: 0 ok, 1 the tile's error, 3 refused, 4 not
    /// available, 5 timed out.
    #[test]
    fn results_map_to_exit_statuses() {
        let r = |text: &str, err: bool| json!({ "isError": err, "content": [{ "type": "text", "text": text }] });
        let status = |o: Output| o.failed.map_or(0, |b| crate::exit_status(&b.code));
        assert_eq!(status(run_output(r("Sent “hi”", false))), 0);
        assert_eq!(status(run_output(r("line 1, column 2: bad", true))), 1);
        assert_eq!(status(run_output(r("The user denied hello__send_message. Nothing was done.", true))), 3);
        assert_eq!(status(run_output(r("The extension “x” is off or not installed in Pitwall. Nothing was done.", true))), 4);
        assert_eq!(status(run_output(r("hello__send_message didn't answer within 120 seconds; it was told to stop.", true))), 5);
        let o = run_output(json!({ "isError": false, "structuredContent": { "a": 1 }, "content": [{ "type": "text", "text": "1" }] }));
        assert_eq!(o.json, json!({ "a": 1 }));
    }
}
