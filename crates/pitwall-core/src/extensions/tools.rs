//! Tools a tile offers to agents (docs/spec/tile-sdk/agent-tools.md
//! §11.3): the `tools` of a manifest's tile entry, read and checked, and
//! the JSON Schema subset their input (and a tile's `"input"`) is written
//! in, with the check Pitwall makes of an agent's input before the tile
//! sees it ([`check_input`]). Phase 2 has the show-only kinds, `show` and
//! `present`.

use serde_json::{Map, Value};

/// The most tools an extension declares.
pub const MAX_TOOLS: usize = 16;
/// The longest tool name.
pub const MAX_NAME: usize = 32;
/// The longest description.
pub const MAX_DESCRIPTION: usize = 1000;
/// The largest schema, as JSON.
pub const MAX_SCHEMA: usize = 16 << 10;
/// The longest `<extension-id>__<tool>` (the id's `.` as `-`): with the
/// `mcp__pitwall__` agents put before it, the 64 characters Claude Code
/// and Codex take for a tool name.
pub const MAX_MCP_NAME: usize = 64 - "mcp__pitwall__".len();

/// Why extension `id`'s tool `tool` (`None`: its `__open`) has a name
/// agents can't take: too long once it is `<id>__<tool>`.
pub fn check_mcp_name(id: &str, tool: Option<&str>) -> Result<(), String> {
    let name = format!("{}__{}", id.replace('.', "-"), tool.unwrap_or("open"));
    if name.len() > MAX_MCP_NAME {
        return Err(format!("the tool's name for agents, {name}, is longer than {MAX_MCP_NAME} characters: shorten the extension's id or the tool's name"));
    }
    Ok(())
}

/// Property names Pitwall adds to every tile tool's input: a declared
/// input can't use them.
pub const RESERVED: [&str; 2] = ["where", "of"];

/// What a tool does (agent-tools.md §11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Opens or focuses the tile.
    Show,
    /// Puts content into the tile or changes content the same agent put
    /// there.
    Present,
}

impl ToolKind {
    pub fn word(self) -> &'static str {
        match self {
            ToolKind::Show => "show",
            ToolKind::Present => "present",
        }
    }
}

/// A tool a tile declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDef {
    /// `[a-z0-9_]`, at most [`MAX_NAME`], unique in the extension.
    pub name: String,
    pub kind: ToolKind,
    /// What an agent reads to choose it (at most [`MAX_DESCRIPTION`]).
    pub description: String,
    /// Its input's JSON Schema, as compact JSON text (an object schema).
    pub input: String,
}

impl ToolDef {
    /// The input schema as JSON.
    pub fn schema(&self) -> Value {
        serde_json::from_str(&self.input).unwrap_or(Value::Null)
    }
}

/// A tile entry's `tools`.
pub fn parse_tools(tile: &str, v: &Value) -> Result<Vec<ToolDef>, String> {
    let Value::Array(list) = v else { return Err(format!("tile “{tile}”: \"tools\" must be a list")) };
    let mut out: Vec<ToolDef> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let at = |what: &str| format!("tile “{tile}”, tool {}: {what}", i + 1);
        let Value::Object(o) = item else { return Err(at("must be an object")) };
        let name = o.get("name").and_then(Value::as_str).ok_or_else(|| at("needs \"name\""))?;
        if name.is_empty() || name.len() > MAX_NAME || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(at(&format!("\"name\" “{name}” must be lowercase letters, digits and _, at most {MAX_NAME}")));
        }
        if name == "open" {
            return Err(at("\"open\" is Pitwall's own tool for every tile: name it something else"));
        }
        if out.iter().any(|t| t.name == name) {
            return Err(at(&format!("“{name}” is there twice")));
        }
        let kind = match o.get("kind").and_then(Value::as_str) {
            Some("show") => ToolKind::Show,
            Some("present") => ToolKind::Present,
            Some(k @ ("read" | "act")) => return Err(at(&format!("kind “{k}” needs a newer Pitwall (this one has show and present)"))),
            Some(k) => return Err(at(&format!("kind “{k}” is show or present"))),
            None => return Err(at("needs \"kind\": show or present")),
        };
        let description = o.get("description").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if description.is_empty() || description.chars().count() > MAX_DESCRIPTION {
            return Err(at(&format!("needs a \"description\" of at most {MAX_DESCRIPTION} characters: what an agent reads to choose it")));
        }
        let schema = o.get("input").cloned().unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
        check_schema(&schema).map_err(|e| at(&format!("\"input\" {e}")))?;
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(at("\"input\" must be an object schema (\"type\": \"object\")"));
        }
        if let Some(p) = schema.get("properties").and_then(Value::as_object).and_then(|p| RESERVED.iter().find(|r| p.contains_key(**r))) {
            return Err(at(&format!("\"input\" can't have a property “{p}”: Pitwall adds it")));
        }
        out.push(ToolDef { name: name.to_string(), kind, description: description.to_string(), input: schema.to_string() });
    }
    Ok(out)
}

/// Why `schema` isn't in the subset tools are written in (draft 2020-12:
/// object, array, string, number, integer, boolean, enum, required,
/// maxLength, description), or too large.
pub fn check_schema(schema: &Value) -> Result<(), String> {
    if schema.to_string().len() > MAX_SCHEMA {
        return Err(format!("is over {} KiB", MAX_SCHEMA >> 10));
    }
    fn walk(s: &Value, at: &str) -> Result<(), String> {
        let Value::Object(o) = s else { return Err(format!("at {at} must be a schema object")) };
        for (k, v) in o {
            match k.as_str() {
                "type" => {
                    let ok = |t: &str| ["object", "array", "string", "number", "integer", "boolean", "null"].contains(&t);
                    let good = match v {
                        Value::String(t) => ok(t),
                        Value::Array(ts) => ts.iter().all(|t| t.as_str().is_some_and(ok)),
                        _ => false,
                    };
                    if !good {
                        return Err(format!("at {at}: \"type\" {v} isn't one Pitwall checks"));
                    }
                }
                "properties" => {
                    let Value::Object(p) = v else { return Err(format!("at {at}: \"properties\" must be an object")) };
                    for (name, sub) in p {
                        walk(sub, &format!("{at}/{name}"))?;
                    }
                }
                "items" => walk(v, &format!("{at}/items"))?,
                "required" if v.as_array().is_some_and(|r| r.iter().all(Value::is_string)) => {}
                "enum" if v.is_array() => {}
                "maxLength" | "minLength" | "maxItems" | "minItems" if v.is_u64() => {}
                "minimum" | "maximum" if v.is_number() => {}
                "additionalProperties" if v.is_boolean() => {}
                "description" | "title" | "$schema" | "$comment" if v.is_string() => {}
                "default" | "examples" => {}
                other => return Err(format!("at {at}: “{other}” isn't in the schema subset Pitwall checks (type, properties, items, required, enum, maxLength, minLength, maxItems, minItems, minimum, maximum, additionalProperties, description)")),
            }
        }
        Ok(())
    }
    walk(schema, "the top")
}

/// What is wrong with `input` against `schema` (the subset of
/// [`check_schema`]), in words an agent can act on ("`text` is required",
/// "`title` is longer than 200 characters"); empty when it fits.
pub fn check_input(schema: &Value, input: &Value) -> Vec<String> {
    let mut out = Vec::new();
    check(schema, input, "", &mut out);
    out
}

fn name(path: &str) -> String {
    if path.is_empty() {
        "the input".to_string()
    } else {
        format!("`{path}`")
    }
}

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn check(schema: &Value, v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(s) = schema.as_object() else { return };
    let types: Vec<&str> = match s.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    if !types.is_empty() && !types.iter().any(|t| type_ok(t, v)) {
        out.push(format!("{} must be {}", name(path), types.iter().map(|t| match *t {
            "object" => "an object",
            "array" => "a list",
            "string" => "a string",
            "number" => "a number",
            "integer" => "a whole number",
            "boolean" => "true or false",
            _ => "null",
        }).collect::<Vec<_>>().join(" or ")));
        return;
    }
    if let Some(Value::Array(allowed)) = s.get("enum") {
        if !allowed.contains(v) {
            let words: Vec<String> = allowed.iter().map(Value::to_string).collect();
            out.push(format!("{} must be one of {}", name(path), words.join(", ")));
        }
    }
    match v {
        Value::String(text) => {
            let n = text.chars().count() as u64;
            if let Some(max) = s.get("maxLength").and_then(Value::as_u64).filter(|m| n > *m) {
                out.push(format!("{} is longer than {max} characters", name(path)));
            }
            if let Some(min) = s.get("minLength").and_then(Value::as_u64).filter(|m| n < *m) {
                out.push(format!("{} is shorter than {min} characters", name(path)));
            }
        }
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.);
            if let Some(max) = s.get("maximum").and_then(Value::as_f64).filter(|m| x > *m) {
                out.push(format!("{} is over {max}", name(path)));
            }
            if let Some(min) = s.get("minimum").and_then(Value::as_f64).filter(|m| x < *m) {
                out.push(format!("{} is under {min}", name(path)));
            }
        }
        Value::Array(items) => {
            let n = items.len() as u64;
            if let Some(max) = s.get("maxItems").and_then(Value::as_u64).filter(|m| n > *m) {
                out.push(format!("{} has more than {max} items", name(path)));
            }
            if let Some(min) = s.get("minItems").and_then(Value::as_u64).filter(|m| n < *m) {
                out.push(format!("{} has fewer than {min} items", name(path)));
            }
            if let Some(each) = s.get("items") {
                for (i, item) in items.iter().enumerate() {
                    check(each, item, &format!("{path}[{i}]"), out);
                }
            }
        }
        Value::Object(o) => object(s, o, path, out),
        _ => {}
    }
}

fn object(s: &Map<String, Value>, o: &Map<String, Value>, path: &str, out: &mut Vec<String>) {
    let join = |k: &str| if path.is_empty() { k.to_string() } else { format!("{path}.{k}") };
    if let Some(Value::Array(req)) = s.get("required") {
        for r in req.iter().filter_map(Value::as_str) {
            if !o.contains_key(r) {
                out.push(format!("`{}` is required", join(r)));
            }
        }
    }
    let props = s.get("properties").and_then(Value::as_object);
    for (k, v) in o {
        match props.and_then(|p| p.get(k)) {
            Some(sub) => check(sub, v, &join(k), out),
            None if s.get("additionalProperties") == Some(&Value::Bool(false)) => {
                let known: Vec<&str> = props.map(|p| p.keys().map(String::as_str).collect()).unwrap_or_default();
                out.push(format!("`{}` isn't something it takes ({})", join(k), if known.is_empty() { "it takes nothing".to_string() } else { format!("it takes {}", known.join(", ")) }));
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tools_are_read_and_checked() {
        let ok = parse_tools("main", &json!([{ "name": "show_diagram", "kind": "present", "description": "Show a flowchart.", "input": { "type": "object", "required": ["mermaid"], "properties": { "item": { "type": "string" }, "mermaid": { "type": "string", "maxLength": 200000 } } } }])).unwrap();
        assert_eq!((ok[0].name.as_str(), ok[0].kind), ("show_diagram", ToolKind::Present));
        assert_eq!(ok[0].schema()["required"], json!(["mermaid"]));
        let err = |v: Value| parse_tools("main", &v).unwrap_err();
        assert!(err(json!([{ "name": "Show", "kind": "show", "description": "x" }])).contains("lowercase"));
        assert!(err(json!([{ "name": "open", "kind": "show", "description": "x" }])).contains("Pitwall's own"));
        assert!(err(json!([{ "name": "a", "kind": "read", "description": "x" }])).contains("newer Pitwall"));
        assert!(err(json!([{ "name": "a", "kind": "show" }])).contains("description"));
        assert!(err(json!([{ "name": "a", "kind": "show", "description": "x", "input": { "type": "object", "properties": { "where": { "type": "string" } } } }])).contains("Pitwall adds it"));
        assert!(err(json!([{ "name": "a", "kind": "show", "description": "x", "input": { "type": "object", "oneOf": [] } }])).contains("oneOf"));
        assert!(err(json!([{ "name": "a", "kind": "show", "description": "x" }, { "name": "a", "kind": "show", "description": "y" }])).contains("twice"));
        // The name agents get fits what they take.
        assert!(check_mcp_name("diagrams", Some("show_diagram")).is_ok());
        let long = "a".repeat(40);
        assert!(check_mcp_name(&long, None).is_ok());
        assert!(check_mcp_name(&long, Some("show_the_whole_diagram")).unwrap_err().contains("longer than 50"));
        let m = |tool: &str| crate::extensions::parse(&format!(r#"{{"v": 2, "id": "{long}", "name": "X", "version": "1.0.0", "contributes": {{"tiles": [{{"id": "main", "title": "X", "wasm": "t.wasm", "tools": [{{"name": "{tool}", "kind": "show", "description": "x"}}]}}]}}}}"#));
        assert!(m("show").is_ok(), "{:?}", m("show"));
        assert!(m("show_the_whole_diagram").unwrap_err().contains("shorten"));
    }

    #[test]
    fn input_is_checked_in_words() {
        let schema = json!({ "type": "object", "required": ["text"], "additionalProperties": false, "properties": {
            "title": { "type": "string", "maxLength": 5 },
            "text": { "type": "string" },
            "mode": { "type": "string", "enum": ["a", "b"] },
            "tags": { "type": "array", "items": { "type": "string" } } } });
        assert!(check_input(&schema, &json!({ "text": "hi" })).is_empty());
        let p = check_input(&schema, &json!({ "title": "too long", "mode": "c", "tags": [1], "x": 1 }));
        assert!(p.contains(&"`text` is required".to_string()), "{p:?}");
        assert!(p.contains(&"`title` is longer than 5 characters".to_string()), "{p:?}");
        assert!(p.iter().any(|m| m.starts_with("`mode` must be one of")), "{p:?}");
        assert!(p.contains(&"`tags[0]` must be a string".to_string()), "{p:?}");
        assert!(p.iter().any(|m| m.starts_with("`x` isn't something it takes")), "{p:?}");
        assert_eq!(check_input(&schema, &json!("x")), vec!["the input must be an object".to_string()]);
    }
}
