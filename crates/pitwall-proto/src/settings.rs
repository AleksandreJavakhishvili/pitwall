//! The settings registry: every user setting Pitwall has, in one typed list
//! that the Settings pages, the settings file (`ui.json`), its JSON Schema
//! (`docs/settings.schema.json`) and `pitwall settings` all read.
//!
//! A setting has a dotted key (`appearance.theme`), a type with its allowed
//! values, a default, a one-line description, the Settings page it is on,
//! and a sensitivity: [`Sensitivity::Safe`] settings only change Pitwall's
//! own look and behaviour and apply directly; [`Sensitivity::Approval`] ones
//! change something outside Pitwall (a file of another tool, a link on
//! PATH, running downloaded code), so when an agent asks for them the user
//! approves in Pitwall first.
//!
//! Most settings live in `ui.json` under the keys the v0.1 app used
//! ([`Stored::Ui`]); a few are state Pitwall reads from elsewhere
//! ([`Stored::Pitwall`]: installed hooks, the CLI link, rulesync's npx
//! opt-in) and are only changed by a running Pitwall.
//!
//! Serde only: reading and writing the file is the caller's.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use ts_rs::TS;

/// The Settings page a setting is on (the sidebar's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Page {
    General,
    Agents,
    Rules,
    Appearance,
    FolderAccess,
    About,
}

impl Page {
    pub const ALL: [Page; 6] = [Page::General, Page::Agents, Page::Rules, Page::Appearance, Page::FolderAccess, Page::About];

    /// The id in `ui.json` (`settingsPage`) and the CLI.
    pub fn id(self) -> &'static str {
        match self {
            Page::General => "general",
            Page::Agents => "agents",
            Page::Rules => "rules",
            Page::Appearance => "appearance",
            Page::FolderAccess => "folderAccess",
            Page::About => "about",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Agents => "Agents",
            Page::Rules => "Rules",
            Page::Appearance => "Appearance",
            Page::FolderAccess => "Folder access",
            Page::About => "About",
        }
    }

    pub fn parse(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }
}

/// How a change is applied when an agent (not the user) asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Sensitivity {
    /// Pitwall's own look and behaviour: applied directly.
    Safe,
    /// Changes something outside Pitwall: the user approves it in Pitwall
    /// first (and it needs Pitwall running).
    Approval,
}

/// A setting's type and allowed values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// One of these strings.
    Choice(&'static [&'static str]),
    /// A whole number in `min..=max`.
    Int { min: i64, max: i64 },
    /// One line of text, up to `max` characters.
    Text { max: usize },
}

/// A default value (const-friendly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultValue {
    Bool(bool),
    Str(&'static str),
    Int(i64),
}

impl DefaultValue {
    pub fn json(self) -> Value {
        match self {
            DefaultValue::Bool(b) => Value::Bool(b),
            DefaultValue::Str(s) => Value::String(s.into()),
            DefaultValue::Int(n) => Value::from(n),
        }
    }
}

/// Where a setting's value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    /// `ui.json`, under this (v0.1) key; `invert`: the file keeps the
    /// opposite (`hideElsewhere` for `general.showElsewhere`).
    Ui { key: &'static str, invert: bool },
    /// Read and changed by a running Pitwall (outside `ui.json`).
    Pitwall,
}

/// One setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    pub key: &'static str,
    pub kind: Kind,
    pub default: DefaultValue,
    /// One line, for the CLI and the schema.
    pub description: &'static str,
    pub page: Page,
    pub sensitivity: Sensitivity,
    pub stored: Stored,
    /// It can only be turned on from Pitwall (installing; removing is done
    /// by hand, as the description says).
    pub only_on: bool,
}

pub const THEMES: &[&str] = &["system", "dark", "light"];
pub const LOOKS: &[&str] = &["flat", "glass"];
pub const DENSITIES: &[&str] = &["comfortable", "compact", "dense"];
/// The Race Engineer's first prompt, by default (`engineer.greeting`).
pub const ENGINEER_GREETING: &str =
    "Introduce yourself in two lines and offer: set up agw, arrange spaces, create agents per project, set up rules, tune settings.";
pub const FONT_MIN: i64 = 9;
pub const FONT_MAX: i64 = 22;
pub const FONT_DEFAULT: i64 = 13;

/// The `ui.json` key that remembers the last Settings page (not a setting:
/// the dialog's own state).
pub const PAGE_KEY: &str = "settingsPage";

/// Every setting, in the order of the Settings pages.
pub const SETTINGS: &[Setting] = &[
    Setting {
        key: "general.showElsewhere",
        kind: Kind::Bool,
        default: DefaultValue::Bool(true),
        description: "Show agents running in other terminal apps as a sidebar group (read-only, checked every ~10 s).",
        page: Page::General,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "hideElsewhere", invert: true },
        only_on: false,
    },
    Setting {
        key: "general.cliTool",
        kind: Kind::Bool,
        default: DefaultValue::Bool(false),
        description: "The `pitwall` command-line tool is installed (a link in a folder on PATH). true installs it; delete the link to remove it.",
        page: Page::General,
        sensitivity: Sensitivity::Approval,
        stored: Stored::Pitwall,
        only_on: true,
    },
    Setting {
        key: "agents.codexHooks",
        kind: Kind::Bool,
        default: DefaultValue::Bool(false),
        description: "Codex hooks are installed in ~/.codex/hooks.json for exact Codex status (backed up first). true installs them; remove them by hand.",
        page: Page::Agents,
        sensitivity: Sensitivity::Approval,
        stored: Stored::Pitwall,
        only_on: true,
    },
    Setting {
        key: "engineer.agent",
        kind: Kind::Text { max: 64 },
        default: DefaultValue::Str("auto"),
        description: "What the Race Engineer (Pitwall's optional assistant) runs on: auto (Claude Code if installed, else the first installed agent), an agent id from New agent (claude, codex, gemini, opencode, aider, …) or custom (engineer.command). Applies the next time it starts.",
        page: Page::Agents,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "engineerAgent", invert: false },
        only_on: false,
    },
    Setting {
        key: "engineer.command",
        kind: Kind::Text { max: 500 },
        default: DefaultValue::Str(""),
        description: "The command the Race Engineer runs when engineer.agent is custom (a command line, run by your login shell).",
        page: Page::Agents,
        sensitivity: Sensitivity::Approval,
        stored: Stored::Ui { key: "engineerCommand", invert: false },
        only_on: false,
    },
    Setting {
        key: "engineer.greeting",
        kind: Kind::Text { max: 1000 },
        default: DefaultValue::Str(ENGINEER_GREETING),
        description: "The prompt queued for a new Race Engineer when it first opens (empty: none).",
        page: Page::Agents,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "engineerGreeting", invert: false },
        only_on: false,
    },
    Setting {
        key: "rules.allowNpx",
        kind: Kind::Bool,
        default: DefaultValue::Bool(false),
        description: "When rulesync isn't installed, run it with `npx -y rulesync` (downloads and runs it from npm).",
        page: Page::Rules,
        sensitivity: Sensitivity::Approval,
        stored: Stored::Pitwall,
        only_on: false,
    },
    Setting {
        key: "appearance.theme",
        kind: Kind::Choice(THEMES),
        default: DefaultValue::Str("system"),
        description: "Light or dark; system follows the OS.",
        page: Page::Appearance,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "theme", invert: false },
        only_on: false,
    },
    Setting {
        key: "appearance.look",
        kind: Kind::Choice(LOOKS),
        default: DefaultValue::Str("flat"),
        description: "Flat, or Glass: translucent chrome over the window material (Liquid Glass where the OS has it). Terminals stay solid.",
        page: Page::Appearance,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "look", invert: false },
        only_on: false,
    },
    Setting {
        key: "appearance.reduceMotion",
        kind: Kind::Bool,
        default: DefaultValue::Bool(false),
        description: "Fewer animations (the OS setting also applies).",
        page: Page::Appearance,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "reduceMotion", invert: false },
        only_on: false,
    },
    Setting {
        key: "appearance.density",
        kind: Kind::Choice(DENSITIES),
        default: DefaultValue::Str("compact"),
        description: "Smallest tile before extra agents fold into chips (spaces may override it).",
        page: Page::Appearance,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "density", invert: false },
        only_on: false,
    },
    Setting {
        key: "appearance.terminalFontSize",
        kind: Kind::Int { min: FONT_MIN, max: FONT_MAX },
        default: DefaultValue::Int(FONT_DEFAULT),
        description: "Base terminal font size in px; tiles may shrink below it to fit.",
        page: Page::Appearance,
        sensitivity: Sensitivity::Safe,
        stored: Stored::Ui { key: "fontSize", invert: false },
        only_on: false,
    },
];

/// The setting called `key`.
pub fn find(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

/// `find`, or a not-found message that lists the keys.
pub fn lookup(key: &str) -> Result<&'static Setting, String> {
    find(key).ok_or_else(|| {
        let keys: Vec<&str> = SETTINGS.iter().map(|s| s.key).collect();
        format!("no setting \"{key}\"; the settings are: {}", keys.join(", "))
    })
}

impl Setting {
    pub fn default_value(&self) -> Value {
        self.default.json()
    }

    /// The `ui.json` key, for settings kept there.
    pub fn file_key(&self) -> Option<&'static str> {
        match self.stored {
            Stored::Ui { key, .. } => Some(key),
            Stored::Pitwall => None,
        }
    }

    /// "true or false", "one of system, dark, light", "a whole number 9–22".
    pub fn allowed(&self) -> String {
        match self.kind {
            Kind::Bool if self.only_on => "true (turning it off isn't done from Pitwall)".into(),
            Kind::Bool => "true or false".into(),
            Kind::Choice(c) => format!("one of {}", c.join(", ")),
            Kind::Int { min, max } => format!("a whole number {min}–{max}"),
            Kind::Text { max } => format!("one line of text, up to {max} characters"),
        }
    }

    /// `v` if it is an allowed value (whole floats like 14.0 count as 14).
    pub fn validate(&self, v: &Value) -> Result<Value, String> {
        let bad = || format!("{}: {} isn't allowed; expected {}", self.key, v, self.allowed());
        match self.kind {
            Kind::Bool => v.as_bool().map(Value::Bool).ok_or_else(bad),
            Kind::Choice(c) => match v.as_str() {
                Some(s) if c.contains(&s) => Ok(Value::String(s.into())),
                _ => Err(bad()),
            },
            Kind::Int { min, max } => {
                let n = v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64));
                match n {
                    Some(n) if (min..=max).contains(&n) => Ok(Value::from(n)),
                    _ => Err(bad()),
                }
            }
            Kind::Text { max } => match v.as_str() {
                Some(s) if s.chars().count() <= max && !s.contains(['\n', '\r']) => Ok(Value::String(s.into())),
                _ => Err(bad()),
            },
        }
    }

    /// [`validate`](Self::validate), and refuse turning off an `only_on`
    /// setting.
    pub fn validate_change(&self, v: &Value) -> Result<Value, String> {
        let v = self.validate(v)?;
        if self.only_on && v == Value::Bool(false) {
            return Err(format!("{} can't be turned off from Pitwall: {}", self.key, self.description));
        }
        Ok(v)
    }

    /// A value typed on the command line: `true`/`false` (also on/off,
    /// yes/no, 1/0) for switches, a choice in any case, a number, or JSON.
    pub fn parse_text(&self, text: &str) -> Result<Value, String> {
        let t = text.trim();
        let v = match self.kind {
            Kind::Bool => match t.to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" | "1" => Value::Bool(true),
                "false" | "off" | "no" | "0" => Value::Bool(false),
                _ => Value::String(t.into()),
            },
            Kind::Choice(_) => Value::String(t.trim_matches('"').to_ascii_lowercase()),
            Kind::Int { .. } => serde_json::from_str::<Value>(t.trim_end_matches("px")).unwrap_or_else(|_| Value::String(t.into())),
            // As typed (surrounding spaces dropped).
            Kind::Text { .. } => Value::String(t.into()),
        };
        self.validate(&v)
    }

    /// The value in `ui.json`: the default when the key is missing, an
    /// error when it is there but not allowed. `None` for settings kept
    /// elsewhere.
    pub fn read_file(&self, ui: &Value) -> Option<Result<Value, String>> {
        let Stored::Ui { key, invert } = self.stored else { return None };
        let Some(raw) = ui.get(key) else { return Some(Ok(self.default_value())) };
        let raw = match (invert, raw, self.kind) {
            (true, Value::Bool(b), _) => Value::Bool(!b),
            // The app keeps a fractional font it was given (v0.1 files).
            (_, Value::Number(n), Kind::Int { .. }) if n.as_i64().is_none() => n.as_f64().map_or(Value::Null, |f| Value::from(f.round() as i64)),
            (_, other, _) => other.clone(),
        };
        Some(self.validate(&raw).map_err(|e| format!("ui.json \"{key}\": {e}")))
    }

    /// Put `v` (already validated) into `ui.json`'s object.
    pub fn write_file(&self, ui: &mut Map<String, Value>, v: &Value) {
        let Stored::Ui { key, invert } = self.stored else { return };
        let v = match (invert, v) {
            (true, Value::Bool(b)) => Value::Bool(!b),
            _ => v.clone(),
        };
        ui.insert(key.into(), v);
    }

    /// What `settings.list` shows for it, with its current value.
    pub fn view(&self, value: Value) -> SettingView {
        let (kind, choices, min, max) = match self.kind {
            Kind::Bool => ("bool", None, None, None),
            Kind::Choice(c) => ("choice", Some(c.iter().map(|s| s.to_string()).collect()), None, None),
            Kind::Int { min, max } => ("int", None, Some(min), Some(max)),
            Kind::Text { max } => ("text", None, None, Some(max as i64)),
        };
        SettingView {
            key: self.key.into(),
            value,
            default: self.default_value(),
            kind: kind.into(),
            choices,
            min,
            max,
            allowed: self.allowed(),
            description: self.description.into(),
            page: self.page,
            sensitivity: self.sensitivity,
            file_key: self.file_key().map(String::from),
            only_on: self.only_on,
        }
    }

    /// Its property in the JSON Schema.
    fn schema(&self) -> Value {
        let mut p = match self.kind {
            Kind::Bool => json!({ "type": "boolean" }),
            Kind::Choice(c) => json!({ "type": "string", "enum": c }),
            Kind::Int { min, max } => json!({ "type": "number", "minimum": min, "maximum": max }),
            Kind::Text { max } => json!({ "type": "string", "maxLength": max }),
        };
        let mut default = self.default_value();
        if let (Stored::Ui { invert: true, .. }, Value::Bool(b)) = (self.stored, &default) {
            default = Value::Bool(!b);
        }
        let desc = match self.stored {
            Stored::Ui { invert: true, .. } => format!("The inverse of `{}`: {}", self.key, self.description),
            _ => format!("`{}`: {}", self.key, self.description),
        };
        p["default"] = default;
        p["description"] = Value::String(desc);
        p
    }
}

/// One setting as `settings.list` / `get` / `set` return it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingView {
    pub key: String,
    /// The current value (`null` when it can't be read now, e.g. a setting
    /// only a running Pitwall knows).
    #[ts(type = "boolean | string | number | null")]
    pub value: Value,
    #[ts(type = "boolean | string | number")]
    pub default: Value,
    /// "bool", "choice", "int" or "text" (`max`: its length).
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | undefined")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | undefined")]
    pub max: Option<i64>,
    /// The allowed values in words.
    pub allowed: String,
    pub description: String,
    pub page: Page,
    pub sensitivity: Sensitivity,
    /// Its key in `ui.json` (none: kept by Pitwall elsewhere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_key: Option<String>,
    #[serde(default)]
    pub only_on: bool,
}

/// `settings.get` / `settings.reset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingKey {
    pub key: String,
}

/// `settings.set`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct SettingSet {
    pub key: String,
    #[ts(type = "boolean | string | number")]
    pub value: Value,
}

/// The JSON Schema of `ui.json`'s settings (for editors), generated from
/// [`SETTINGS`]; `docs/settings.schema.json` is this, kept in sync by a test.
pub fn json_schema() -> Value {
    let mut props = Map::new();
    props.insert("v".into(), json!({ "const": 1, "description": "The file's version (always 1)." }));
    for s in SETTINGS {
        if let Some(key) = s.file_key() {
            props.insert(key.into(), s.schema());
        }
    }
    let pages: Vec<&str> = Page::ALL.iter().map(|p| p.id()).collect();
    props.insert(PAGE_KEY.into(), json!({ "type": "string", "enum": pages, "description": "The Settings page shown last." }));
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Pitwall settings (ui.json)",
        "description": "The settings in Pitwall's ui.json (~/.pitwall/ui.json; $PITWALL_HOME/ui.json). Pitwall applies edits live; an invalid value is reported and the last good one kept. The file also holds spaces and layouts, which are not described here. `pitwall settings list` shows every setting, including the ones kept outside this file.",
        "type": "object",
        "required": ["v"],
        "properties": Value::Object(props),
        "additionalProperties": true
    })
}

/// The schema as written to `docs/settings.schema.json`.
pub fn json_schema_text() -> String {
    let mut s = serde_json::to_string_pretty(&json_schema()).unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_dotted_and_on_their_page() {
        for (i, s) in SETTINGS.iter().enumerate() {
            assert!(SETTINGS[i + 1..].iter().all(|o| o.key != s.key), "{} twice", s.key);
            let (page, name) = s.key.split_once('.').expect("dotted");
            assert!(!name.is_empty() && !name.contains('.'));
            // A feature with its own settings on a page keeps its name
            // (`engineer.*` on Agents); everything else is named after its page.
            let named_for = match page {
                "engineer" => Page::Agents.id(),
                other => other,
            };
            assert_eq!(named_for, s.page.id(), "{} is named after its page", s.key);
            assert_eq!(s.validate(&s.default_value()).unwrap(), s.default_value(), "{}: the default is allowed", s.key);
            assert!(!s.description.is_empty() && !s.description.contains('\n'));
        }
        for p in Page::ALL {
            assert_eq!(Page::parse(p.id()), Some(p));
        }
    }

    #[test]
    fn text_settings() {
        let agent = find("engineer.agent").unwrap();
        assert_eq!(agent.parse_text(" codex ").unwrap(), json!("codex"));
        assert_eq!(agent.default_value(), json!("auto"));
        assert!(agent.validate(&json!("a\nb")).is_err());
        assert!(agent.validate(&json!("x".repeat(65))).is_err());
        assert!(agent.validate(&json!(3)).is_err());
        assert_eq!(find("engineer.command").unwrap().sensitivity, Sensitivity::Approval, "it runs a command");
        let v = agent.view(json!("gemini"));
        assert_eq!((v.kind.as_str(), v.max), ("text", Some(64)));
        assert_eq!(json_schema()["properties"]["engineerAgent"]["maxLength"], json!(64));
        assert_eq!(find("engineer.greeting").unwrap().default_value(), json!(ENGINEER_GREETING));
    }

    #[test]
    fn validation_and_cli_text() {
        let theme = find("appearance.theme").unwrap();
        assert_eq!(theme.parse_text("Dark").unwrap(), json!("dark"));
        assert!(theme.parse_text("purple").unwrap_err().contains("one of system, dark, light"));
        let font = find("appearance.terminalFontSize").unwrap();
        assert_eq!(font.parse_text("15").unwrap(), json!(15));
        assert_eq!(font.parse_text("15px").unwrap(), json!(15));
        assert_eq!(font.validate(&json!(14.0)).unwrap(), json!(14));
        assert!(font.validate(&json!(14.5)).is_err());
        assert!(font.parse_text("40").unwrap_err().contains("9–22"));
        let motion = find("appearance.reduceMotion").unwrap();
        assert_eq!(motion.parse_text("on").unwrap(), json!(true));
        assert_eq!(motion.parse_text("false").unwrap(), json!(false));
        assert!(motion.parse_text("maybe").is_err());
        assert!(motion.validate(&json!("true")).is_err(), "JSON strings aren't switches");
        let hooks = find("agents.codexHooks").unwrap();
        assert!(hooks.validate_change(&json!(false)).unwrap_err().contains("can't be turned off"));
        assert_eq!(hooks.validate_change(&json!(true)).unwrap(), json!(true));
        assert!(lookup("appearance.nope").unwrap_err().contains("appearance.theme"));
    }

    #[test]
    fn reads_and_writes_the_v01_file_keys() {
        let ui = json!({"v": 1, "theme": "light", "hideElsewhere": true, "fontSize": 13.4, "look": 3, "spaces": []});
        let get = |k: &str| find(k).unwrap().read_file(&ui).unwrap();
        assert_eq!(get("appearance.theme").unwrap(), json!("light"));
        assert_eq!(get("general.showElsewhere").unwrap(), json!(false), "inverted");
        assert_eq!(get("appearance.terminalFontSize").unwrap(), json!(13));
        assert!(get("appearance.look").unwrap_err().contains("\"look\""));
        assert_eq!(get("appearance.density").unwrap(), json!("compact"), "missing: the default");
        assert!(find("agents.codexHooks").unwrap().read_file(&ui).is_none());

        let mut obj = ui.as_object().unwrap().clone();
        find("general.showElsewhere").unwrap().write_file(&mut obj, &json!(true));
        find("appearance.density").unwrap().write_file(&mut obj, &json!("dense"));
        find("agents.codexHooks").unwrap().write_file(&mut obj, &json!(true));
        assert_eq!(obj["hideElsewhere"], json!(false));
        assert_eq!(obj["density"], json!("dense"));
        assert_eq!(obj["spaces"], json!([]), "the rest stays");
        assert!(!obj.contains_key("agents.codexHooks"));
    }

    #[test]
    fn views_round_trip() {
        let v = find("appearance.density").unwrap().view(json!("dense"));
        let wire = serde_json::to_value(&v).unwrap();
        assert_eq!(wire["kind"], "choice");
        assert_eq!(wire["choices"], json!(["comfortable", "compact", "dense"]));
        assert_eq!(wire["fileKey"], "density");
        assert_eq!(wire["sensitivity"], "safe");
        assert_eq!(wire["page"], "appearance");
        assert!(wire.get("min").is_none());
        assert_eq!(serde_json::from_value::<SettingView>(wire).unwrap(), v);
        let set: SettingSet = serde_json::from_value(json!({"key": "appearance.theme", "value": "dark"})).unwrap();
        assert_eq!(set.value, json!("dark"));
        assert_eq!(find("rules.allowNpx").unwrap().view(Value::Null).sensitivity, Sensitivity::Approval);
    }

    /// `docs/settings.schema.json` is generated from the registry. After
    /// changing a setting: `PITWALL_WRITE_SCHEMA=1 cargo test -p pitwall-proto`.
    #[test]
    fn the_schema_file_is_in_sync() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/settings.schema.json");
        let want = json_schema_text();
        if std::env::var_os("PITWALL_WRITE_SCHEMA").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        // Compared as JSON: key order depends on serde_json's features in
        // the build (`preserve_order` in the workspace).
        let got: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or(Value::Null);
        assert!(got == json_schema(), "docs/settings.schema.json is stale: run PITWALL_WRITE_SCHEMA=1 cargo test -p pitwall-proto");
        let schema = json_schema();
        assert_eq!(schema["properties"]["hideElsewhere"]["default"], json!(false));
        assert_eq!(schema["properties"]["fontSize"]["maximum"], json!(22));
        assert_eq!(schema["properties"]["theme"]["enum"], json!(["system", "dark", "light"]));
    }
}
