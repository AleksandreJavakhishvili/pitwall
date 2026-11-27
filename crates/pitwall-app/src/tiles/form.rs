//! The small form the palette opens for a tile's command that needs
//! input (agent-tools.md §11.12), made from its input schema: one field
//! per top-level property of the checked subset (strings, numbers and
//! integers, booleans, enums; `required`, `maxLength`, `description`).
//! A schema with anything else (nested objects, arrays) gets no form, and
//! its command no palette entry. What the user typed becomes the call's
//! JSON input, checked as Pitwall checks an agent's.

use serde_json::{Map, Value};

/// What a field takes.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text { max: Option<usize> },
    Number { integer: bool },
    Bool,
    Choice(Vec<String>),
}

/// One field of the form.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The property's name.
    pub name: String,
    /// What the form calls it ("item_id" → "Item id").
    pub label: String,
    pub description: Option<String>,
    pub kind: Kind,
    pub required: bool,
}

/// What the user gave a field.
#[derive(Debug, Clone, PartialEq)]
pub enum Given {
    Text(String),
    Bool(bool),
    /// The option picked, by index (`None`: none yet).
    Choice(Option<usize>),
}

/// A property's name as a label: words, the first capitalised.
pub fn label_of(name: &str) -> String {
    let words = name.replace(['_', '-'], " ");
    let mut c = words.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The form for `schema`, in its properties' order (the manifest's);
/// `None` when it can't be asked for in a form.
pub fn fields(schema: &Value) -> Option<Vec<Field>> {
    let props = match schema.get("properties") {
        None => return Some(Vec::new()),
        Some(p) => p.as_object()?,
    };
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for (name, p) in props {
        let kind = if let Some(options) = p.get("enum") {
            let options: Option<Vec<String>> = options.as_array()?.iter().map(|o| o.as_str().map(str::to_string)).collect();
            Kind::Choice(options.filter(|o| !o.is_empty())?)
        } else {
            match p.get("type").and_then(Value::as_str)? {
                "string" => Kind::Text { max: p.get("maxLength").and_then(Value::as_u64).map(|n| n as usize) },
                "number" => Kind::Number { integer: false },
                "integer" => Kind::Number { integer: true },
                "boolean" => Kind::Bool,
                _ => return None,
            }
        };
        let description = p.get("description").and_then(Value::as_str).map(str::to_string).filter(|d| !d.trim().is_empty());
        out.push(Field { name: name.clone(), label: label_of(name), description, kind, required: required.contains(&name.as_str()) });
    }
    Some(out)
}

/// What a field starts with.
pub fn initial(f: &Field) -> Given {
    match &f.kind {
        Kind::Bool => Given::Bool(false),
        Kind::Choice(_) => Given::Choice(None),
        _ => Given::Text(String::new()),
    }
}

/// The call's input from what the user gave (`given[i]` for `fields[i]`):
/// an empty optional field is left out, a boolean is always given. Else,
/// each field that is wrong and why.
pub fn input(fields: &[Field], given: &[Given]) -> Result<Value, Vec<(usize, String)>> {
    let mut map = Map::new();
    let mut problems = Vec::new();
    for (i, (f, g)) in fields.iter().zip(given).enumerate() {
        let value = match (&f.kind, g) {
            (Kind::Bool, Given::Bool(b)) => Some(Value::Bool(*b)),
            (Kind::Choice(options), Given::Choice(pick)) => pick.and_then(|k| options.get(k)).map(|o| Value::String(o.clone())),
            (Kind::Text { max }, Given::Text(t)) => {
                if let Some(m) = max.filter(|m| t.chars().count() > *m) {
                    problems.push((i, format!("At most {m} characters.")));
                    continue;
                }
                (!t.is_empty()).then(|| Value::String(t.clone()))
            }
            (Kind::Number { integer }, Given::Text(t)) => {
                let t = t.trim();
                if t.is_empty() {
                    None
                } else if *integer {
                    match t.parse::<i64>() {
                        Ok(n) => Some(Value::from(n)),
                        Err(_) => {
                            problems.push((i, "A whole number.".into()));
                            continue;
                        }
                    }
                } else {
                    match t.parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
                        Some(n) => Some(Value::Number(n)),
                        None => {
                            problems.push((i, "A number.".into()));
                            continue;
                        }
                    }
                }
            }
            _ => None,
        };
        match value {
            Some(v) => {
                map.insert(f.name.clone(), v);
            }
            None if f.required => problems.push((i, "Needed.".into())),
            None => {}
        }
    }
    if problems.is_empty() {
        Ok(Value::Object(map))
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "item_id": { "type": "string", "description": "The diagram to export", "maxLength": 8 },
                "scale": { "type": "number" },
                "copies": { "type": "integer" },
                "dark": { "type": "boolean" },
                "format": { "enum": ["svg", "png"] }
            },
            "required": ["item_id", "format"]
        })
    }

    #[test]
    fn a_form_has_a_field_per_property_of_the_checked_subset() {
        let f = fields(&schema()).unwrap();
        let names: Vec<_> = f.iter().map(|f| (f.label.as_str(), f.required)).collect();
        assert_eq!(names, [("Item id", true), ("Scale", false), ("Copies", false), ("Dark", false), ("Format", true)]);
        assert_eq!(f[0].kind, Kind::Text { max: Some(8) });
        assert_eq!(f[0].description.as_deref(), Some("The diagram to export"));
        assert_eq!(f[2].kind, Kind::Number { integer: true });
        assert_eq!(f[4].kind, Kind::Choice(vec!["svg".into(), "png".into()]));
        // Nothing nested.
        assert!(fields(&json!({ "properties": { "a": { "type": "object" } } })).is_none());
        assert!(fields(&json!({ "properties": { "a": { "type": "array" } } })).is_none());
        assert_eq!(fields(&json!({ "type": "object" })), Some(vec![]));
    }

    #[test]
    fn what_was_given_becomes_the_input_or_says_whats_wrong() {
        let f = fields(&schema()).unwrap();
        let mut given: Vec<Given> = f.iter().map(initial).collect();
        let err = input(&f, &given).unwrap_err();
        assert_eq!(err, vec![(0, "Needed.".into()), (4, "Needed.".into())]);
        given[0] = Given::Text("auth-flow".into());
        given[1] = Given::Text("x".into());
        given[2] = Given::Text("1.5".into());
        given[4] = Given::Choice(Some(1));
        let err = input(&f, &given).unwrap_err();
        assert_eq!(err, vec![(0, "At most 8 characters.".into()), (1, "A number.".into()), (2, "A whole number.".into())]);
        given[0] = Given::Text("auth".into());
        given[1] = Given::Text(" 2.5 ".into());
        given[2] = Given::Text(String::new());
        given[3] = Given::Bool(true);
        assert_eq!(input(&f, &given).unwrap(), json!({ "item_id": "auth", "scale": 2.5, "dark": true, "format": "png" }));
    }
}
