//! Screen-based status detection: data-driven, per-agent TOML rules.
//!
//! Each agent kind has a rule file (`crates/pitwall-detect/detect/<kind>.toml`, embedded
//! at build time). A user file at `~/.pitwall/detect/<kind>.toml` replaces the
//! built-in rules for that kind (and can add rules for new kinds). Files are
//! loaded lazily once per app run; restart Pitwall to pick up edits (there
//! is no live reload: nothing in the app could trigger one, so an unused
//! `reload()` and the lock it needed were removed).
//!
//! Rule model (the approach and most built-in rules are adapted from Herdr,
//! Apache-2.0 — see NOTICE):
//!
//! ```toml
//! [[rules]]
//! id = "live_prompt_box"
//! state = "idle"            # working | blocked | idle | unknown
//! priority = 950            # higher is evaluated first; first match wins
//! region = "prompt_box_body"
//! contains = ["..."]        # all must appear (case-insensitive)
//! regex = ['...']           # all must match somewhere in the region
//! line_regex = ['...']      # each must match at least one line
//! all = [ { ... } ]         # nested gates: all must match
//! any = [ { ... } ]         # at least one must match
//! not = [ { ... } ]         # none may match
//! detail = "Trust this folder?"   # optional fixed hint (blocked only)
//! detail_line = ['...']     # optional; overrides the file-level `detail_line`
//! ```
//!
//! A matching `unknown` rule stops evaluation and yields no state (used for
//! viewers/menus where the screen says nothing about the agent's status).
//!
//! Evidence principle: rules match invariant UI chrome (footers, dialog
//! controls, the prompt box) in narrow regions near the bottom of the screen,
//! never arbitrary pane text, so output the user or agent prints cannot
//! impersonate a state.

use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detected {
    Working,
    Blocked,
    Idle,
}

#[derive(Debug, Clone, Default)]
pub struct Detection {
    pub state: Option<Detected>,
    pub detail: Option<String>,
}

/// kind_id is "claude" | "codex" | "shell" | "custom" | user-defined.
/// `None` state = no rule matched (backend falls back to activity).
pub fn detect(kind_id: &str, screen_text: &str, title: Option<&str>) -> Detection {
    let Some(rules) = rules_for(kind_id) else {
        return Detection::default();
    };
    rules.evaluate(screen_text, title.unwrap_or(""))
}

// ---------------------------------------------------------------------------
// Loading

const BUILTIN: &[(&str, &str)] = &[
    ("claude", include_str!("../detect/claude.toml")),
    ("codex", include_str!("../detect/codex.toml")),
    ("gemini", include_str!("../detect/gemini.toml")),
    ("opencode", include_str!("../detect/opencode.toml")),
    ("cursor", include_str!("../detect/cursor.toml")),
    ("copilot", include_str!("../detect/copilot.toml")),
    ("qwen", include_str!("../detect/qwen.toml")),
    ("amp", include_str!("../detect/amp.toml")),
    ("aider", include_str!("../detect/aider.toml")),
];

/// Kinds that have built-in rules (the app checks each has an agent definition).
pub fn builtin_rule_kinds() -> impl Iterator<Item = &'static str> {
    BUILTIN.iter().map(|(kind, _)| *kind)
}

type Cache = HashMap<String, Arc<RuleSet>>;

static CACHE: OnceLock<Cache> = OnceLock::new();

fn cache() -> &'static Cache {
    CACHE.get_or_init(load_all)
}

fn rules_for(kind: &str) -> Option<Arc<RuleSet>> {
    cache().get(kind).cloned()
}

fn user_dir() -> Option<PathBuf> {
    // Pitwall's data folder: `$PITWALL_HOME`, else ~/.pitwall (pitwall-core paths).
    if let Some(root) = std::env::var_os("PITWALL_HOME").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(root).join("detect"));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".pitwall").join("detect"))
}

fn load_all() -> Cache {
    let mut map = Cache::new();
    for (kind, src) in BUILTIN {
        match RuleSet::parse(src) {
            Ok(set) => {
                map.insert((*kind).to_string(), Arc::new(set));
            }
            // Built-ins are covered by tests; never crash the app over one.
            Err(err) => eprintln!("pitwall: built-in detect rules for {kind} invalid: {err}"),
        }
    }
    if let Some(dir) = user_dir() {
        load_overrides(&dir, &mut map);
    }
    map
}

fn load_overrides(dir: &std::path::Path, map: &mut Cache) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(kind) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|src| RuleSet::parse(&src));
        match parsed {
            Ok(set) => {
                map.insert(kind.to_string(), Arc::new(set));
            }
            // A broken override keeps the built-in rules for that kind.
            Err(err) => eprintln!("pitwall: ignoring {}: {err}", path.display()),
        }
    }
}

// ---------------------------------------------------------------------------
// Rule files

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(default)]
    #[allow(dead_code)]
    id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<String>,
    /// Default patterns for extracting a blocked-state hint.
    #[serde(default)]
    detail_line: Vec<String>,
    #[serde(default)]
    rules: Vec<RuleDef>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleDef {
    id: String,
    state: StateDef,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_region")]
    region: String,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    detail_line: Option<Vec<String>>,
    // Top-level gate (same keys as `GateDef`; not `flatten`ed because serde
    // can't combine that with `deny_unknown_fields`, and typos must error).
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
    #[serde(default)]
    all: Vec<GateDef>,
    #[serde(default)]
    any: Vec<GateDef>,
    #[serde(default, rename = "not")]
    not_: Vec<GateDef>,
}

fn default_region() -> String {
    "screen".to_string()
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StateDef {
    Working,
    Blocked,
    Idle,
    Unknown,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct GateDef {
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
    #[serde(default)]
    all: Vec<GateDef>,
    #[serde(default)]
    any: Vec<GateDef>,
    #[serde(default, rename = "not")]
    not_: Vec<GateDef>,
}

// ---------------------------------------------------------------------------
// Compiled rules

struct RuleSet {
    rules: Vec<Rule>,
}

struct Rule {
    #[allow(dead_code)]
    id: String,
    state: StateDef,
    priority: i32,
    region: Region,
    gate: Gate,
    detail: Option<String>,
    detail_line: Vec<Regex>,
}

struct Gate {
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
    all: Vec<Gate>,
    any: Vec<Gate>,
    not_: Vec<Gate>,
}

const MAX_GATE_DEPTH: usize = 8;
const DETAIL_SEARCH_LINES: usize = 30;
const MAX_DETAIL_CHARS: usize = 160;

impl RuleSet {
    fn parse(src: &str) -> Result<RuleSet, String> {
        let file: RuleFile = toml::from_str(src).map_err(|e| e.to_string())?;
        let default_detail = compile_regexes(&file.detail_line)?;
        let mut rules = Vec::with_capacity(file.rules.len());
        for def in file.rules {
            let rule = Rule::compile(def, &default_detail)?;
            rules.push(rule);
        }
        // Stable: equal priorities keep file order.
        rules.sort_by_key(|r| std::cmp::Reverse(r.priority));
        Ok(RuleSet { rules })
    }

    fn evaluate(&self, screen: &str, title: &str) -> Detection {
        let screen = trim_trailing_blank_lines(screen);
        for rule in &self.rules {
            let text = rule.region.select(screen, title);
            if !rule.gate.matches(text, &text.to_lowercase()) {
                continue;
            }
            let state = match rule.state {
                StateDef::Working => Detected::Working,
                StateDef::Blocked => Detected::Blocked,
                StateDef::Idle => Detected::Idle,
                StateDef::Unknown => return Detection::default(),
            };
            let detail = (state == Detected::Blocked)
                .then(|| rule.detail(screen))
                .flatten();
            return Detection {
                state: Some(state),
                detail,
            };
        }
        Detection::default()
    }
}

impl Rule {
    fn compile(def: RuleDef, default_detail: &[Regex]) -> Result<Rule, String> {
        let ctx = |e: String| format!("rule {}: {e}", def.id);
        let region = Region::parse(&def.region).map_err(ctx)?;
        let top = GateDef {
            contains: def.contains,
            regex: def.regex,
            line_regex: def.line_regex,
            all: def.all,
            any: def.any,
            not_: def.not_,
        };
        let gate = Gate::compile(&top, 0).map_err(ctx)?;
        let detail_line = match &def.detail_line {
            Some(patterns) => compile_regexes(patterns).map_err(ctx)?,
            None => default_detail.to_vec(),
        };
        Ok(Rule {
            id: def.id,
            state: def.state,
            priority: def.priority,
            region,
            gate,
            detail: def.detail,
            detail_line,
        })
    }

    /// Short human hint for a blocked state: fixed text, or the first
    /// `detail_line` pattern found near the bottom of the screen (searching
    /// bottom-up, so the live dialog wins over older scrollback).
    fn detail(&self, screen: &str) -> Option<String> {
        if let Some(fixed) = &self.detail {
            return Some(fixed.clone());
        }
        let lines: Vec<&str> = screen
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(DETAIL_SEARCH_LINES)
            .collect();
        for re in &self.detail_line {
            for line in &lines {
                if let Some(caps) = re.captures(line) {
                    let text = caps.get(1).or_else(|| caps.get(0))?.as_str().trim();
                    if text.is_empty() {
                        continue;
                    }
                    return Some(truncate(text, MAX_DETAIL_CHARS));
                }
            }
        }
        None
    }
}

impl Gate {
    fn compile(def: &GateDef, depth: usize) -> Result<Gate, String> {
        if depth > MAX_GATE_DEPTH {
            return Err("gates nested too deeply".into());
        }
        let nested = |list: &[GateDef]| -> Result<Vec<Gate>, String> {
            list.iter().map(|g| Gate::compile(g, depth + 1)).collect()
        };
        Ok(Gate {
            contains: def.contains.iter().map(|s| s.to_lowercase()).collect(),
            regex: compile_regexes(&def.regex)?,
            line_regex: compile_regexes(&def.line_regex)?,
            all: nested(&def.all)?,
            any: nested(&def.any)?,
            not_: nested(&def.not_)?,
        })
    }

    fn matches(&self, text: &str, lower: &str) -> bool {
        self.contains.iter().all(|n| lower.contains(n.as_str()))
            && self.regex.iter().all(|r| r.is_match(text))
            && self
                .line_regex
                .iter()
                .all(|r| text.lines().any(|line| r.is_match(line)))
            && self.all.iter().all(|g| g.matches(text, lower))
            && (self.any.is_empty() || self.any.iter().any(|g| g.matches(text, lower)))
            && !self.not_.iter().any(|g| g.matches(text, lower))
    }
}

fn compile_regexes(patterns: &[String]) -> Result<Vec<Regex>, String> {
    patterns
        .iter()
        .map(|p| Regex::new(p).map_err(|e| format!("bad regex {p:?}: {e}")))
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn trim_trailing_blank_lines(s: &str) -> &str {
    let mut end = s.len();
    for line in s.lines().rev() {
        if line.trim().is_empty() {
            end = end.saturating_sub(line.len() + 1);
        } else {
            break;
        }
    }
    // `lines()` drops a final empty segment after a trailing '\n'.
    let trimmed = &s[..end.min(s.len())];
    trimmed.trim_end_matches(['\n', '\r', ' '])
}

// ---------------------------------------------------------------------------
// Regions

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    /// The OSC 0/2 window title.
    Title,
    /// Whole visible screen.
    Screen,
    BottomLines(usize),
    BottomNonEmptyLines(usize),
    TopNonEmptyLines(usize),
    /// Lines after the last `────` rule (empty if there is none).
    AfterLastHorizontalRule,
    /// Lines between the last two `────` rules (Claude's prompt box).
    PromptBoxBody,
    /// Everything above the prompt box.
    AbovePromptBox,
    /// Last N non-empty lines above the prompt box.
    AbovePromptBoxBottom(usize),
    LastNonEmptyAbovePromptBox,
    /// Codex: lines after the last `›` prompt line.
    AfterLastPromptMarker,
    /// Codex: lines before the current composer `›` line.
    BeforeCurrentPromptMarker,
    /// Codex: whole screen, or empty when a current composer is visible.
    WholeWithoutCurrentPromptMarker,
    /// Codex: the current composer line itself (empty if none).
    CurrentPromptLine,
}

impl Region {
    fn parse(spec: &str) -> Result<Region, String> {
        let spec = spec.trim();
        let simple = match spec {
            "title" | "osc_title" => Some(Region::Title),
            "screen" | "whole_recent" => Some(Region::Screen),
            "after_last_horizontal_rule" => Some(Region::AfterLastHorizontalRule),
            "prompt_box_body" => Some(Region::PromptBoxBody),
            "above_prompt_box" => Some(Region::AbovePromptBox),
            "last_non_empty_above_prompt_box" => Some(Region::LastNonEmptyAbovePromptBox),
            "after_last_prompt_marker" => Some(Region::AfterLastPromptMarker),
            "before_current_prompt_marker" => Some(Region::BeforeCurrentPromptMarker),
            "whole_recent_without_current_prompt_marker" => {
                Some(Region::WholeWithoutCurrentPromptMarker)
            }
            "current_prompt_line" => Some(Region::CurrentPromptLine),
            _ => None,
        };
        if let Some(r) = simple {
            return Ok(r);
        }
        let counted = |name: &str| -> Option<usize> {
            spec.strip_prefix(name)?
                .strip_prefix('(')?
                .strip_suffix(')')?
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n > 0)
        };
        if let Some(n) = counted("bottom_lines") {
            return Ok(Region::BottomLines(n));
        }
        if let Some(n) = counted("bottom_non_empty_lines") {
            return Ok(Region::BottomNonEmptyLines(n));
        }
        if let Some(n) = counted("top_non_empty_lines") {
            return Ok(Region::TopNonEmptyLines(n));
        }
        if let Some(n) = counted("above_prompt_box_bottom") {
            return Ok(Region::AbovePromptBoxBottom(n));
        }
        Err(format!("unknown region {spec:?}"))
    }

    fn select<'a>(self, screen: &'a str, title: &'a str) -> &'a str {
        match self {
            Region::Title => title,
            Region::Screen => screen,
            Region::BottomLines(n) => {
                let lines = line_spans(screen);
                let start = lines.len().saturating_sub(n);
                from_line(screen, &lines, start)
            }
            Region::BottomNonEmptyLines(n) => bottom_non_empty(screen, n),
            Region::TopNonEmptyLines(n) => {
                let lines = line_spans(screen);
                match lines
                    .iter()
                    .enumerate()
                    .filter(|(_, (s, e))| !screen[*s..*e].trim().is_empty())
                    .take(n)
                    .last()
                {
                    Some((_, (_, end))) => &screen[..*end],
                    None => "",
                }
            }
            Region::AfterLastHorizontalRule => {
                let lines = line_spans(screen);
                match lines
                    .iter()
                    .rposition(|(s, e)| is_horizontal_rule(&screen[*s..*e]))
                {
                    Some(i) => from_line(screen, &lines, i + 1),
                    None => "",
                }
            }
            Region::PromptBoxBody => {
                let lines = line_spans(screen);
                match prompt_box_top(screen, &lines) {
                    Some(top) => {
                        let start = lines.get(top + 1).map(|l| l.0).unwrap_or(screen.len());
                        let end = lines[top + 1..]
                            .iter()
                            .find(|(s, e)| is_horizontal_rule(&screen[*s..*e]))
                            .map(|l| l.0)
                            .unwrap_or(screen.len());
                        &screen[start..end.max(start)]
                    }
                    None => "",
                }
            }
            Region::AbovePromptBox => above_prompt_box(screen),
            Region::AbovePromptBoxBottom(n) => bottom_non_empty(above_prompt_box(screen), n),
            Region::LastNonEmptyAbovePromptBox => bottom_non_empty(above_prompt_box(screen), 1),
            Region::AfterLastPromptMarker => {
                let lines = line_spans(screen);
                match lines
                    .iter()
                    .rposition(|(s, e)| codex_prompt_line(&screen[*s..*e]))
                {
                    Some(i) => from_line(screen, &lines, i + 1),
                    None => screen,
                }
            }
            Region::BeforeCurrentPromptMarker => {
                let lines = line_spans(screen);
                match current_codex_prompt(screen, &lines) {
                    Some(i) => &screen[..lines[i].0],
                    None => screen,
                }
            }
            Region::WholeWithoutCurrentPromptMarker => {
                let lines = line_spans(screen);
                match current_codex_prompt(screen, &lines) {
                    Some(_) => "",
                    None => screen,
                }
            }
            Region::CurrentPromptLine => {
                let lines = line_spans(screen);
                match current_codex_prompt(screen, &lines) {
                    Some(i) => &screen[lines[i].0..lines[i].1],
                    None => "",
                }
            }
        }
    }
}

/// Byte spans (start, end-exclusive, without '\n') of each line.
fn line_spans(s: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (i, b) in s.bytes().enumerate() {
        if b == b'\n' {
            spans.push((start, i));
            start = i + 1;
        }
    }
    if start < s.len() {
        spans.push((start, s.len()));
    }
    spans
}

fn from_line<'a>(s: &'a str, lines: &[(usize, usize)], index: usize) -> &'a str {
    match lines.get(index) {
        Some((start, _)) => &s[*start..],
        None => "",
    }
}

fn bottom_non_empty(s: &str, n: usize) -> &str {
    let lines = line_spans(s);
    match lines
        .iter()
        .rev()
        .filter(|(a, b)| !s[*a..*b].trim().is_empty())
        .take(n)
        .last()
    {
        Some((start, _)) => &s[*start..],
        None => "",
    }
}

fn above_prompt_box(s: &str) -> &str {
    let lines = line_spans(s);
    match prompt_box_top(s, &lines) {
        Some(top) => &s[..lines[top].0],
        None => s,
    }
}

/// Index of the second-to-last horizontal rule (the prompt box's top border).
fn prompt_box_top(s: &str, lines: &[(usize, usize)]) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, (a, b))| is_horizontal_rule(&s[*a..*b]))
        .nth(1)
        .map(|(i, _)| i)
}

/// A line made of `─` (optionally followed by a label after >= 3 of them).
fn is_horizontal_rule(line: &str) -> bool {
    let t = line.trim();
    let rule_chars = t.chars().take_while(|&c| c == '─').count();
    if rule_chars == 0 {
        return false;
    }
    let rest = &t[rule_chars * '─'.len_utf8()..];
    rest.trim_start().is_empty() || rule_chars >= 3
}

fn codex_prompt_line(line: &str) -> bool {
    line == "›" || line.starts_with("› ")
}

fn codex_block_marker_line(line: &str) -> bool {
    line.starts_with('•') || line.starts_with('■') || line.starts_with('✗') || line.starts_with('✓')
}

/// The last `›` line, provided no response block starts below it.
fn current_codex_prompt(s: &str, lines: &[(usize, usize)]) -> Option<usize> {
    let i = lines
        .iter()
        .rposition(|(a, b)| codex_prompt_line(&s[*a..*b]))?;
    if lines[i + 1..]
        .iter()
        .any(|(a, b)| codex_block_marker_line(&s[*a..*b]))
    {
        return None;
    }
    Some(i)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_more_agents;
