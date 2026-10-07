//! Starting a new agent on a machine: the form its provider describes.
//!
//! This Mac's form is Pitwall's own (a kind and a project folder). A
//! platform such as agw has its own choices — which workspace, which user,
//! which session template — so its provider lists them as [`CreateField`]s
//! with the options read from the platform. Clients render the fields as
//! data; they never ask which provider it is (architecture.md §3). The
//! chosen values go back as `options` (field id → value) and the provider
//! turns them into its own command.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The choice value that means "make a new one" (a new workspace, a new
/// agent user). Platform names never contain `+`.
pub const CREATE_NEW: &str = "+new";

/// What a name may look like.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
pub struct NameRule {
    /// A regular expression a name must match (JavaScript syntax).
    pub pattern: String,
    pub max_len: u32,
    /// The rule in words, shown when a name doesn't fit.
    pub hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum FieldInput {
    /// One of `choices`.
    Select,
    /// Free text (a name).
    Text,
}

/// Shown only while another field has this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FieldWhen {
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateChoice {
    pub value: String,
    pub label: String,
    /// A short note next to the label (a template's description).
    pub detail: Option<String>,
    /// How the choice reads in the form's summary ("in workspace work").
    /// `{name}` is the new agent's name, `{<field id>}` another field's value.
    pub phrase: Option<String>,
    /// Choosing it also makes this on the machine ("workspace {workspaceName}"),
    /// which the summary points out.
    pub creates: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateField {
    /// The key in `options`.
    pub id: String,
    pub label: String,
    pub input: FieldInput,
    pub choices: Vec<CreateChoice>,
    pub default: Option<String>,
    pub placeholder: Option<String>,
    pub hint: Option<String>,
    pub when: Option<FieldWhen>,
    /// Text: left empty, the platform uses the new agent's name.
    pub defaults_to_name: bool,
    /// Text: what it may look like.
    pub rule: Option<NameRule>,
}

impl CreateField {
    pub fn select(id: &str, label: &str, choices: Vec<CreateChoice>, default: Option<String>) -> CreateField {
        CreateField {
            id: id.into(),
            label: label.into(),
            input: FieldInput::Select,
            choices,
            default,
            placeholder: None,
            hint: None,
            when: None,
            defaults_to_name: false,
            rule: None,
        }
    }

    pub fn text(id: &str, label: &str, rule: NameRule) -> CreateField {
        CreateField { input: FieldInput::Text, rule: Some(rule), ..CreateField::select(id, label, vec![], None) }
    }

    pub fn when(mut self, field: &str, value: &str) -> CreateField {
        self.when = Some(FieldWhen { field: field.into(), value: value.into() });
        self
    }
}

/// How a new agent is made on one machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateForm {
    pub provider: String,
    pub machine: String,
    pub machine_label: String,
    /// Pitwall's own form: pick a kind and a project folder (worktree and
    /// rules by the kind's caps). Otherwise the platform decides what runs
    /// and where, from `fields`.
    pub folder: bool,
    /// The agent's name; on a platform it is also the name made there.
    pub name: NameRule,
    pub fields: Vec<CreateField>,
    /// What will be made, in words, with `{name}` and `{<field id>}` filled
    /// in ("Creates session {name} {workspace} on vm {runAs}").
    pub summary: Option<String>,
    /// The button ("Start", "Create").
    pub submit: String,
    /// Some choices couldn't be listed (the rest still work).
    pub error: Option<String>,
}

/// `machine.form`: the form for one machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct FormRequest {
    pub provider: String,
    pub machine: String,
}

/// The summary line and what else gets made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Summary {
    pub text: String,
    pub creates: Vec<String>,
}

impl CreateForm {
    /// The name rule for agents on this Mac (and the UI's default).
    pub fn pitwall_name() -> NameRule {
        NameRule {
            pattern: "^[a-z][a-z0-9_-]{0,31}$".into(),
            max_len: 32,
            hint: "Lowercase letters, digits, - or _; starts with a letter; max 32".into(),
        }
    }

    /// Pitwall's own form: a kind and a folder.
    pub fn folder(provider: &str, machine: &str, label: &str) -> CreateForm {
        CreateForm {
            provider: provider.into(),
            machine: machine.into(),
            machine_label: label.into(),
            folder: true,
            name: CreateForm::pitwall_name(),
            fields: vec![],
            summary: None,
            submit: "Start".into(),
            error: None,
        }
    }

    pub fn field(&self, id: &str) -> Option<&CreateField> {
        self.fields.iter().find(|f| f.id == id)
    }

    fn shown(f: &CreateField, values: &BTreeMap<String, String>) -> bool {
        f.when.as_ref().is_none_or(|w| values.get(&w.field) == Some(&w.value))
    }

    /// The options to create with: what was chosen, else each field's
    /// default, for the fields that apply (a field whose `when` doesn't hold
    /// is left out). Empty text stays out (the platform's default). Unknown
    /// keys, values that aren't a choice, and options for fields that don't
    /// apply are errors.
    pub fn values(&self, chosen: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, String> {
        if let Some(k) = chosen.keys().find(|k| self.field(k).is_none()) {
            let known: Vec<_> = self.fields.iter().map(|f| f.id.as_str()).collect();
            return Err(if known.is_empty() {
                format!("{} takes no options (got \"{k}\")", self.machine_label)
            } else {
                format!("unknown option \"{k}\" (options here: {})", known.join(", "))
            });
        }
        let mut out = BTreeMap::new();
        for f in &self.fields {
            let given = chosen.get(&f.id).map(|v| v.trim()).filter(|v| !v.is_empty());
            if !CreateForm::shown(f, &out) {
                if given.is_some() {
                    let w = f.when.as_ref().expect("hidden fields have a condition");
                    return Err(format!("{} only applies when {} is \"{}\"", f.label, w.field, w.value));
                }
                continue;
            }
            let value = given.map(str::to_string).or_else(|| f.default.clone());
            match (f.input, value) {
                (FieldInput::Select, Some(v)) => {
                    if !f.choices.iter().any(|c| c.value == v) {
                        let list: Vec<_> = f.choices.iter().map(|c| c.value.as_str()).collect();
                        return Err(format!("\"{v}\" isn't a choice for {} (choices: {})", f.label, list.join(", ")));
                    }
                    out.insert(f.id.clone(), v);
                }
                (FieldInput::Select, None) => return Err(format!("choose {}", f.label)),
                (FieldInput::Text, Some(v)) => {
                    out.insert(f.id.clone(), v);
                }
                (FieldInput::Text, None) => {}
            }
        }
        Ok(out)
    }

    /// The summary and the extra things made, for `name` and `values`
    /// (the result of [`values`](Self::values)).
    pub fn summarize(&self, name: &str, values: &BTreeMap<String, String>) -> Summary {
        let creates = self
            .fields
            .iter()
            .filter(|f| CreateForm::shown(f, values))
            .filter_map(|f| self.choice(f, values)?.creates.as_deref())
            .map(|t| self.fill(t, name, values, 0))
            .collect();
        let text = self.summary.as_deref().map(|t| self.fill(t, name, values, 0)).unwrap_or_default();
        Summary { text, creates }
    }

    fn choice<'a>(&'a self, f: &'a CreateField, values: &BTreeMap<String, String>) -> Option<&'a CreateChoice> {
        let v = values.get(&f.id)?;
        f.choices.iter().find(|c| c.value == *v)
    }

    /// `{name}` and `{<field id>}` in `t`; spacing tidied.
    fn fill(&self, t: &str, name: &str, values: &BTreeMap<String, String>, depth: u8) -> String {
        let mut out = String::new();
        let mut rest = t;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let Some(len) = rest[start..].find('}') else { break };
            let key = &rest[start + 1..start + len];
            out.push_str(&self.value_text(key, name, values, depth));
            rest = &rest[start + len + 1..];
        }
        out.push_str(rest);
        let words: Vec<_> = out.split_whitespace().collect();
        words.join(" ").replace(" ,", ",").replace(" .", ".")
    }

    fn value_text(&self, key: &str, name: &str, values: &BTreeMap<String, String>, depth: u8) -> String {
        if key == "name" {
            return name.to_string();
        }
        let Some(f) = self.field(key).filter(|f| CreateForm::shown(f, values)) else { return String::new() };
        match f.input {
            FieldInput::Text => match values.get(key) {
                Some(v) => v.clone(),
                None if f.defaults_to_name => name.to_string(),
                None => String::new(),
            },
            FieldInput::Select => match self.choice(f, values) {
                Some(c) => match &c.phrase {
                    Some(p) if depth < 2 => self.fill(p, name, values, depth + 1),
                    _ => c.value.clone(),
                },
                None => String::new(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(value: &str, phrase: &str, creates: Option<&str>) -> CreateChoice {
        CreateChoice { value: value.into(), label: value.into(), detail: None, phrase: Some(phrase.into()), creates: creates.map(Into::into) }
    }

    fn form() -> CreateForm {
        let rule = NameRule { pattern: "^[a-z]+$".into(), max_len: 10, hint: "letters".into() };
        let mut ws_name = CreateField::text("workspaceName", "Workspace name", rule).when("workspace", CREATE_NEW);
        ws_name.defaults_to_name = true;
        CreateForm {
            fields: vec![
                CreateField::select(
                    "workspace",
                    "Workspace",
                    vec![
                        choice("work", "in workspace work", None),
                        choice(CREATE_NEW, "in a new workspace {workspaceName}", Some("workspace {workspaceName} ({workspaceTemplate})")),
                    ],
                    Some("work".into()),
                ),
                ws_name,
                CreateField::select("workspaceTemplate", "Workspace template", vec![choice("default", "default", None)], Some("default".into()))
                    .when("workspace", CREATE_NEW),
                CreateField::select("runAs", "Runs as", vec![choice("admin", "as admin", None)], Some("admin".into())),
            ],
            summary: Some("Creates session {name} {workspace} on vm {runAs}.".into()),
            ..CreateForm::folder("p", "vm", "vm")
        }
    }

    fn map(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn values_fill_defaults_and_check_choices() {
        let f = form();
        assert_eq!(f.values(&map(&[])).unwrap(), map(&[("workspace", "work"), ("runAs", "admin")]));
        let new = f.values(&map(&[("workspace", CREATE_NEW)])).unwrap();
        assert_eq!(new, map(&[("workspace", CREATE_NEW), ("workspaceTemplate", "default"), ("runAs", "admin")]));
        assert!(f.values(&map(&[("workspace", "nope")])).unwrap_err().contains("isn't a choice for Workspace"));
        assert!(f.values(&map(&[("color", "red")])).unwrap_err().contains("unknown option \"color\""));
        assert!(f.values(&map(&[("workspaceName", "x")])).unwrap_err().contains("only applies when workspace is \"+new\""));
        let folder = CreateForm::folder("local", "this-mac", "This Mac");
        assert!(folder.values(&map(&[])).unwrap().is_empty());
        assert!(folder.values(&map(&[("workspace", "w")])).unwrap_err().contains("takes no options"));
    }

    #[test]
    fn summaries_read_as_a_sentence() {
        let f = form();
        let s = f.summarize("api", &f.values(&map(&[])).unwrap());
        assert_eq!(s, Summary { text: "Creates session api in workspace work on vm as admin.".into(), creates: vec![] });
        let v = f.values(&map(&[("workspace", CREATE_NEW)])).unwrap();
        let s = f.summarize("api", &v);
        assert_eq!(s.text, "Creates session api in a new workspace api on vm as admin.");
        assert_eq!(s.creates, ["workspace api (default)"]);
        let v = f.values(&map(&[("workspace", CREATE_NEW), ("workspaceName", "scratch")])).unwrap();
        assert_eq!(f.summarize("api", &v).creates, ["workspace scratch (default)"]);
        assert_eq!(CreateForm::folder("l", "m", "M").summarize("x", &BTreeMap::new()), Summary::default());
    }
}
