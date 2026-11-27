//! An extension's skill for agents (docs/spec/tile-sdk/agent-tools.md
//! §11.11): `agents/SKILL.md` in its folder, Markdown with a `name` and a
//! `description` up front, telling agents when to use its tiles' tools.
//! Text only, at most 16 KiB; its name is forced to the extension's id; it
//! may name only the extension's own tools (`<id>__…`). Pitwall gives
//! agents its description through the MCP server's instructions.

use std::path::Path;

/// Where it is, in the extension's folder.
pub const FILE: &str = "agents/SKILL.md";

/// The largest skill.
pub const MAX: usize = 16 << 10;

/// A skill read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Always the extension's id.
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Read `text` as extension `id`'s skill, or why it can't be used.
pub fn parse(id: &str, text: &str) -> Result<Skill, String> {
    if text.len() > MAX {
        return Err(format!("{FILE} is {} KiB, over the 16 KiB a skill may be", text.len() / 1024));
    }
    let rest = text.strip_prefix("---\n").ok_or_else(|| format!("{FILE} must start with --- and a name and a description"))?;
    let (head, body) = rest.split_once("\n---").ok_or_else(|| format!("{FILE}: its front matter has no closing ---"))?;
    let field = |k: &str| head.lines().find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix(':'))).map(|v| v.trim().trim_matches('"').to_string());
    let description = field("description").filter(|d| !d.is_empty()).ok_or_else(|| format!("{FILE} has no description"))?;
    if description.chars().count() > 300 {
        return Err(format!("{FILE}: its description is over 300 characters"));
    }
    if let Some(n) = field("name").filter(|n| n != id) {
        return Err(format!("{FILE} is named “{n}”: a skill is named after its extension, “{id}”"));
    }
    // Only its own tools: another extension's would steer agents to a
    // stronger tool (decision 7).
    let own = format!("{id}__");
    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')) {
        if let Some(i) = word.find("__") {
            if i > 0 && !word.starts_with(&own) {
                return Err(format!("{FILE} names {word}, a tool of another extension: a skill may only describe its own tools"));
            }
        }
    }
    Ok(Skill { name: id.to_string(), description, body: body.trim_start_matches('\n').to_string() })
}

/// Extension `id`'s skill in folder `dir`: `None` when it has none.
pub fn read(dir: &Path, id: &str) -> Option<Result<Skill, String>> {
    let text = std::fs::read_to_string(dir.join(FILE)).ok()?;
    Some(parse(id, &text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skill_is_named_after_its_extension_and_names_its_own_tools() {
        let ok = "---\nname: diagrams\ndescription: Show flowcharts in Pitwall's Diagrams tile.\n---\nCall `diagrams__show_diagram` with Mermaid text.\n";
        let s = parse("diagrams", ok).unwrap();
        assert_eq!((s.name.as_str(), s.description.as_str()), ("diagrams", "Show flowcharts in Pitwall's Diagrams tile."));
        assert!(parse("diagrams", &ok.replace("name: diagrams", "name: other")).unwrap_err().contains("named after its extension"));
        assert!(parse("diagrams", &ok.replace("diagrams__show_diagram", "shell__run")).unwrap_err().contains("another extension"));
        assert!(parse("diagrams", "no front matter").is_err());
        assert!(parse("diagrams", &format!("{ok}{}", "x".repeat(MAX))).unwrap_err().contains("16 KiB"));
        // No name: it gets the extension's.
        assert_eq!(parse("diagrams", &ok.replace("name: diagrams\n", "")).unwrap().name, "diagrams");
    }
}
