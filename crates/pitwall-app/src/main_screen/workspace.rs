//! Spaces, their layouts and the window's view state: the typed `ui.json`
//! (a port of `src/state/workspace.ts` and `src/layout/density.ts`). Same
//! file, same JSON shape as the Tauri app; fields this module doesn't use
//! (theme, look, …) are kept as they are when it writes the file.
//!
//! All edits are pure: they take a state and return the next one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use pitwall_proto::AgentView;

use super::tree::{
    self, build_grid, build_preset, fit_layout, pane_rects, preset_for, remove_pane, split_pane,
    MinSize, Node, Preset, Rect, Side,
};

pub const MAIN: &str = "main";
pub const ALL_SPACE: &str = "all";
pub const DEFAULT_FONT: f64 = 13.0;

pub use crate::theme::Density;

/// Below this a pane folds into a chip: the minimum cells at the floor
/// font (tiles shrink their font before folding), plus pane chrome
/// ([`Density::fold_min`]).
pub fn fold_min(density: Density, base_font: f64) -> MinSize {
    let (w, h) = density.fold_min(crate::theme::clamp_font(base_font.round() as i32));
    MinSize {
        min_w: w as f64,
        min_h: h as f64,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceKind {
    All,
    Project,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub name: String,
    pub kind: SpaceKind,
    /// Project spaces: the project path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Custom spaces: agents that belong here (panes or chips).
    #[serde(default)]
    pub members: Vec<String>,
    pub layout: Node,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub maximized_pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<Density>,
}

impl Space {
    fn tidy(mut self) -> Space {
        let ls = self.layout.leaves();
        let focused_ok = self
            .focused_pane_id
            .as_deref()
            .is_some_and(|id| ls.iter().any(|p| p.id == id));
        if !focused_ok {
            self.focused_pane_id = ls.first().map(|p| p.id.clone());
        }
        let max_ok = self
            .maximized_pane_id
            .as_deref()
            .is_some_and(|id| ls.iter().any(|p| p.id == id));
        if !max_ok || ls.len() < 2 {
            self.maximized_pane_id = None;
        }
        self
    }

    fn with_member(mut self, agent: &str) -> Space {
        if self.kind != SpaceKind::All && !self.members.iter().any(|m| m == agent) {
            self.members.push(agent.to_string());
        }
        self
    }

    fn without_member(mut self, agent: &str) -> Space {
        self.members.retain(|m| m != agent);
        self
    }

    /// The agent shown in the focused pane.
    pub fn focused_agent(&self) -> Option<String> {
        self.layout
            .find_pane(self.focused_pane_id.as_deref()?)
            .and_then(|p| p.agent_id)
    }

    /// Agents that belong here (shown in panes or offered as chips).
    pub fn members_of<'a>(&self, agents: &'a [AgentView]) -> Vec<&'a AgentView> {
        let shown = self.layout.agents();
        agents
            .iter()
            .filter(|a| match self.kind {
                SpaceKind::All => true,
                SpaceKind::Project => {
                    Some(&a.project) == self.project.as_ref()
                        || self.members.contains(&a.id)
                        || shown.contains(&a.id)
                }
                SpaceKind::Custom => self.members.contains(&a.id) || shown.contains(&a.id),
            })
            .collect()
    }
}

fn empty_layout() -> Node {
    Node::pane(None)
}

fn default_font() -> f64 {
    DEFAULT_FONT
}

fn default_density() -> Density {
    Density::Compact
}

fn default_v() -> u32 {
    1
}

/// A field that falls back to its default when the file has something else
/// there (as `sanitize` in `workspace.ts` does field by field), so one bad
/// value never resets the whole file.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

fn lenient_font<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_f64().filter(|n| n.is_finite()).unwrap_or(DEFAULT_FONT))
}

/// `true` only for a JSON `true` (`r.reduceMotion === true`).
fn yes() -> bool {
    true
}

/// `true` unless the value is `false` (`usePersistentFlag` with a `true`
/// default).
fn not_false<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Value::deserialize(d)? != Value::Bool(false))
}

fn strict_true<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Value::deserialize(d)? == Value::Bool(true))
}

/// `ui.json`: the whole UI state, in the Tauri app's keys and values
/// (`UiState` in `src/state/workspace.ts`), so the switch keeps the user's
/// spaces, layout and settings. One copy per app ([`crate::ui_state`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    #[serde(default = "default_v")]
    pub v: u32,
    #[serde(default)]
    pub spaces: Vec<Space>,
    /// spaceId → window label; missing means "main".
    #[serde(default, deserialize_with = "lenient")]
    pub window_of: BTreeMap<String, String>,
    /// Window labels in Wall mode.
    #[serde(default, deserialize_with = "lenient")]
    pub wall: Vec<String>,
    /// Settings → Terminal font (px); tiles may shrink below it.
    #[serde(default = "default_font", deserialize_with = "lenient_font")]
    pub font_size: f64,
    /// Settings → Tiles → Density (the default; spaces may override it).
    #[serde(default = "default_density", deserialize_with = "lenient")]
    pub density: Density,
    /// Per-agent tile font chosen with ⌘+ / ⌘− (⌘0 removes it).
    #[serde(default, deserialize_with = "lenient")]
    pub tile_font: BTreeMap<String, f64>,
    /// Collapsed project groups in the sidebar (group keys).
    #[serde(default, deserialize_with = "lenient")]
    pub collapsed: Vec<String>,
    /// Collapsed project groups on the Wall.
    #[serde(default, deserialize_with = "lenient")]
    pub wall_collapsed: Vec<String>,
    /// Settings: hide the sidebar's "Elsewhere" group.
    #[serde(default, deserialize_with = "strict_true")]
    pub hide_elsewhere: bool,
    /// Settings → Appearance → Theme.
    #[serde(default, deserialize_with = "lenient")]
    pub theme: crate::theme::ThemePref,
    /// Settings → Appearance → Look (Flat or Glass).
    #[serde(default, deserialize_with = "lenient")]
    pub look: crate::theme::Look,
    /// Settings → Appearance → Reduce motion (the OS setting applies too).
    #[serde(default, deserialize_with = "strict_true")]
    pub reduce_motion: bool,
    /// Files: "Ignored" (list what .gitignore leaves out). The Tauri app
    /// keeps this in localStorage and drops unknown keys when it rewrites
    /// the file, so it costs it nothing.
    #[serde(default, deserialize_with = "strict_true")]
    pub explorer_show_ignored: bool,
    /// Review: diffs side by side (`true`) or inline; unset until chosen
    /// (`crate::review::ops` then reads the old `review.json`). The Tauri
    /// app keeps it in localStorage (`pitwall.review.sideBySide`).
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub review_side_by_side: Option<bool>,
    /// The right panel shows Files instead of Changes (the Tauri app's
    /// localStorage `pitwall.right.files`).
    #[serde(default, deserialize_with = "strict_true")]
    pub right_files: bool,
    /// The sidebar is collapsed (`pitwall.sidebarCollapsed`), read when a
    /// window opens.
    #[serde(default, deserialize_with = "strict_true")]
    pub sidebar_collapsed: bool,
    /// The one-time Full Disk Access hint was shown
    /// (`pitwall.accessHintShown`).
    #[serde(default, deserialize_with = "strict_true")]
    pub access_hint_shown: bool,
    /// The docked right panel is shown (`pitwall.rightOpen`), read when a
    /// window opens.
    #[serde(default = "yes", deserialize_with = "not_false")]
    pub right_open: bool,
    /// Everything else in the file (`focusRequest`, keys of later
    /// versions), written back untouched.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

impl UiState {
    /// The appearance inputs (Settings → Appearance and Tiles).
    pub fn appearance_inputs(&self) -> crate::theme::Inputs {
        crate::theme::Inputs {
            pref: self.theme,
            look: self.look,
            reduce_motion: self.reduce_motion,
            density: self.density,
            font_size: crate::theme::clamp_font(self.font_size.round() as i32),
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        let layout = empty_layout();
        UiState {
            v: 1,
            spaces: vec![Space {
                id: ALL_SPACE.into(),
                name: "All".into(),
                kind: SpaceKind::All,
                project: None,
                members: vec![],
                focused_pane_id: Some(layout.id().to_string()),
                layout,
                maximized_pane_id: None,
                density: None,
            }],
            window_of: BTreeMap::new(),
            wall: vec![],
            font_size: DEFAULT_FONT,
            density: Density::Compact,
            tile_font: BTreeMap::new(),
            collapsed: vec![],
            wall_collapsed: vec![],
            hide_elsewhere: false,
            theme: Default::default(),
            look: Default::default(),
            reduce_motion: false,
            explorer_show_ignored: false,
            review_side_by_side: None,
            right_files: false,
            sidebar_collapsed: false,
            access_hint_shown: false,
            right_open: true,
            rest: serde_json::Map::new(),
        }
    }
}

/// Where an agent is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub space_id: String,
    pub space_name: String,
    pub pane_id: String,
    pub window: String,
}

/// How a drop went (`DropOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropOutcome {
    Noop,
    Placed,
    Swapped,
    Replaced,
    /// No room at this density: it joined the chip strip.
    Chip,
}

/// What a drop measures against: the space area and the tile minimum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub area: (f64, f64),
    pub min: MinSize,
}

impl UiState {
    /// Whatever came from disk, with defaults for anything malformed.
    pub fn sanitize(raw: Value) -> UiState {
        if raw.get("v").and_then(Value::as_u64) != Some(1) {
            return UiState::default();
        }
        // Drop spaces that don't parse, keep the rest.
        let mut raw = raw;
        let spaces: Vec<Space> = raw
            .get_mut("spaces")
            .and_then(|s| s.as_array_mut())
            .map(|list| {
                std::mem::take(list)
                    .into_iter()
                    .filter_map(|s| serde_json::from_value::<Space>(s).ok())
                    .collect()
            })
            .unwrap_or_default();
        let mut s: UiState = serde_json::from_value(raw).unwrap_or_default();
        s.spaces = spaces.into_iter().map(Space::tidy).collect();
        if !s.spaces.iter().any(|sp| sp.id == ALL_SPACE) {
            s.spaces.insert(0, UiState::default().spaces.remove(0));
        }
        s
    }

    pub fn window_of_space(&self, space_id: &str) -> &str {
        self.window_of
            .get(space_id)
            .map(String::as_str)
            .unwrap_or(MAIN)
    }

    pub fn spaces_of(&self, label: &str) -> Vec<&Space> {
        self.spaces
            .iter()
            .filter(|s| self.window_of_space(&s.id) == label)
            .collect()
    }

    pub fn space(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn density_of(&self, space: &Space) -> Density {
        space.density.unwrap_or(self.density)
    }

    pub fn locate(&self, agent: &str) -> Option<Location> {
        self.spaces.iter().find_map(|sp| {
            sp.layout.find_by_agent(agent).map(|p| Location {
                space_id: sp.id.clone(),
                space_name: sp.name.clone(),
                pane_id: p.id,
                window: self.window_of_space(&sp.id).to_string(),
            })
        })
    }

    fn map_space(mut self, id: &str, f: impl FnOnce(Space) -> Space) -> UiState {
        if let Some(i) = self.spaces.iter().position(|s| s.id == id) {
            let sp = self.spaces.remove(i);
            self.spaces.insert(i, f(sp));
        }
        self
    }

    /// Take the agent out of every pane except in `except`.
    fn unplace(mut self, agent: &str, except: Option<&str>) -> UiState {
        for sp in &mut self.spaces {
            if Some(sp.id.as_str()) == except || sp.layout.find_by_agent(agent).is_none() {
                continue;
            }
            sp.layout = tree::drop_agent(&sp.layout, agent);
            *sp = sp.clone().tidy();
        }
        self
    }

    /// Move semantics: out of every other space (pane closes, membership ends).
    fn detach(self, agent: &str, except: &str) -> UiState {
        let mut next = self.unplace(agent, Some(except));
        for sp in &mut next.spaces {
            if sp.id != except {
                *sp = sp.clone().without_member(agent);
            }
        }
        next
    }

    /// Show an agent in a space: in `pane` if given, else the first empty
    /// pane, else the focused one; with `side`, split that pane instead.
    pub fn place_agent(
        self,
        space_id: &str,
        agent: &str,
        pane: Option<&str>,
        side: Option<Side>,
    ) -> UiState {
        let next = self.unplace(agent, Some(space_id));
        next.map_space(space_id, |sp| {
            let mut sp = if sp.kind == SpaceKind::Custom {
                sp.with_member(agent)
            } else {
                sp
            };
            let first = sp.layout.leaves()[0].id.clone();
            let reference = pane
                .map(str::to_string)
                .or_else(|| sp.focused_pane_id.clone())
                .unwrap_or(first.clone());
            let already = sp.layout.find_by_agent(agent);
            if let Some(side) = side {
                let anchor = if sp.layout.find_pane(&reference).is_some() {
                    reference
                } else {
                    first
                };
                let (t, id) = split_pane(&sp.layout, &anchor, side, Some(agent));
                sp.layout = t;
                sp.focused_pane_id = Some(id);
                sp.maximized_pane_id = None;
                return sp;
            }
            if let (Some(a), None) = (&already, pane) {
                sp.focused_pane_id = Some(a.id.clone());
                return sp;
            }
            let empty = sp
                .layout
                .leaves()
                .into_iter()
                .find(|p| p.agent_id.is_none());
            let target = match pane {
                Some(p) if sp.layout.find_pane(p).is_some() => p.to_string(),
                _ => empty.map(|p| p.id).unwrap_or(reference),
            };
            if let Some(a) = already.filter(|a| a.id != target) {
                let displaced = sp.layout.find_pane(&target).and_then(|p| p.agent_id);
                sp.layout = sp.layout.set_pane_agent(&a.id, displaced.as_deref());
            }
            sp.layout = sp.layout.set_pane_agent(&target, Some(agent));
            sp.focused_pane_id = Some(target);
            sp
        })
    }

    pub fn focus_pane(self, space_id: &str, pane_id: &str) -> UiState {
        self.map_space(space_id, |mut sp| {
            sp.focused_pane_id = Some(pane_id.to_string());
            sp
        })
    }

    pub fn toggle_maximize(self, space_id: &str, pane_id: Option<&str>) -> UiState {
        self.map_space(space_id, |mut sp| {
            let id = pane_id
                .map(str::to_string)
                .or_else(|| sp.focused_pane_id.clone());
            match id {
                Some(id) if sp.layout.leaves().len() >= 2 => {
                    sp.maximized_pane_id = if sp.maximized_pane_id.as_deref() == Some(&id) {
                        None
                    } else {
                        Some(id.clone())
                    };
                    sp.focused_pane_id = Some(id);
                }
                _ => sp.maximized_pane_id = None,
            }
            sp
        })
    }

    pub fn close_pane(self, space_id: &str, pane_id: &str) -> UiState {
        self.map_space(space_id, |mut sp| {
            sp.layout = remove_pane(&sp.layout, pane_id).unwrap_or_else(empty_layout);
            if sp.maximized_pane_id.as_deref() == Some(pane_id) {
                sp.maximized_pane_id = None;
            }
            sp.tidy()
        })
    }

    pub fn set_layout(self, space_id: &str, layout: Node) -> UiState {
        self.map_space(space_id, |mut sp| {
            sp.layout = layout;
            sp.tidy()
        })
    }

    /// Re-tile with a preset: what's shown first (focused first), then
    /// members not shown anywhere; the rest stay chips. `auto_rows` gives
    /// the auto grid's rows for a count.
    pub fn apply_preset(
        self,
        space_id: &str,
        preset: Preset,
        agents: &[AgentView],
        auto_rows: impl Fn(usize) -> Vec<usize>,
    ) -> UiState {
        let Some(sp) = self.space(space_id) else {
            return self;
        };
        let n = preset.count();
        let focused = sp.focused_agent();
        let mut ordered: Vec<String> = focused.iter().cloned().collect();
        ordered.extend(
            sp.layout
                .agents()
                .into_iter()
                .filter(|a| Some(a) != focused.as_ref()),
        );
        for a in sp.members_of(agents) {
            if ordered.len() >= n {
                break;
            }
            if !ordered.contains(&a.id) && self.locate(&a.id).is_none() {
                ordered.push(a.id.clone());
            }
        }
        let layout = if preset == Preset::Auto {
            let rows = auto_rows(ordered.len());
            let total: usize = rows.iter().sum();
            ordered.truncate(total);
            build_grid(&rows, &ordered)
        } else {
            ordered.truncate(n);
            build_preset(preset, &ordered)
        };
        self.map_space(space_id, |mut sp| {
            sp.focused_pane_id = Some(layout.leaves()[0].id.clone());
            sp.layout = layout;
            sp.maximized_pane_id = None;
            sp
        })
    }

    /// A space tiling one project's agents (reused if it exists), owned by `label`.
    pub fn open_project_space(
        self,
        project: &str,
        name: &str,
        agents: &[AgentView],
        label: &str,
    ) -> (UiState, String) {
        if let Some(sp) = self
            .spaces
            .iter()
            .find(|s| s.kind == SpaceKind::Project && s.project.as_deref() == Some(project))
        {
            let id = sp.id.clone();
            return (self, id);
        }
        let ids: Vec<String> = agents
            .iter()
            .filter(|a| a.project == project)
            .map(|a| a.id.clone())
            .collect();
        let preset = preset_for(ids.len());
        let chosen: Vec<String> = ids.into_iter().take(preset.count()).collect();
        let mut next = self;
        for id in &chosen {
            next = next.unplace(id, None);
        }
        let layout = build_preset(preset_for(chosen.len()), &chosen);
        let id = tree::new_id("space");
        next.spaces.push(Space {
            id: id.clone(),
            name: name.to_string(),
            kind: SpaceKind::Project,
            project: Some(project.to_string()),
            members: vec![],
            focused_pane_id: Some(layout.leaves()[0].id.clone()),
            layout,
            maximized_pane_id: None,
            density: None,
        });
        next.window_of.insert(id.clone(), label.to_string());
        (next, id)
    }

    /// Tile `agents` (then what the space shows already) into `space_id`,
    /// at most `max` of them (`tileAgents`: the onboarding hand-over).
    pub fn tile_agents(self, space_id: &str, agents: &[String], max: usize) -> UiState {
        let Some(sp) = self.space(space_id) else {
            return self;
        };
        if agents.is_empty() {
            return self;
        }
        let cap = max.clamp(1, tree::Preset::G3x2.count());
        let shown: Vec<String> =
            sp.layout.agents().into_iter().filter(|a| !agents.contains(a)).collect();
        let mut chosen: Vec<String> = Vec::new();
        for id in agents.iter().chain(shown.iter()) {
            if !chosen.contains(id) {
                chosen.push(id.clone());
            }
        }
        chosen.truncate(cap);
        let mut next = self;
        for id in &chosen {
            next = next.unplace(id, Some(space_id));
        }
        let layout = build_preset(preset_for(chosen.len()), &chosen);
        next.map_space(space_id, |mut x| {
            x.focused_pane_id = Some(layout.leaves()[0].id.clone());
            x.layout = layout;
            x.maximized_pane_id = None;
            x
        })
    }

    /// "Space N" (custom), owned by `label`.
    pub fn create_custom_space(mut self, label: &str) -> (UiState, String) {
        let n = self
            .spaces
            .iter()
            .filter(|s| s.kind == SpaceKind::Custom)
            .count()
            + 1;
        let layout = empty_layout();
        let id = tree::new_id("space");
        self.spaces.push(Space {
            id: id.clone(),
            name: format!("Space {n}"),
            kind: SpaceKind::Custom,
            project: None,
            members: vec![],
            focused_pane_id: Some(layout.id().to_string()),
            layout,
            maximized_pane_id: None,
            density: None,
        });
        self.window_of.insert(id.clone(), label.to_string());
        (self, id)
    }

    /// An empty name keeps the old one.
    pub fn rename_space(self, id: &str, name: &str) -> UiState {
        let name = name.trim().to_string();
        self.map_space(id, |mut sp| {
            if !name.is_empty() && sp.kind != SpaceKind::All {
                sp.name = name;
            }
            sp
        })
    }

    /// Its agents just stop being shown there. "All" can't be closed.
    pub fn close_space(mut self, id: &str) -> UiState {
        if id == ALL_SPACE {
            return self;
        }
        self.spaces.retain(|s| s.id != id);
        self.window_of.remove(id);
        self
    }

    /// Drop references to agents that no longer exist.
    pub fn prune_agents(self, alive: &[String]) -> UiState {
        let mut next = self;
        let gone: Vec<String> = next
            .spaces
            .iter()
            .flat_map(|s| s.layout.agents())
            .filter(|a| !alive.contains(a))
            .collect();
        for a in gone {
            next = next.unplace(&a, None);
        }
        next.tile_font.retain(|a, _| alive.contains(a));
        for sp in &mut next.spaces {
            sp.members.retain(|m| alive.contains(m));
        }
        next
    }

    /// Global density, or a space's own (`None` there: use the global one).
    pub fn set_density(mut self, density: Option<Density>, space: Option<&str>) -> UiState {
        match space {
            None => {
                if let Some(d) = density {
                    self.density = d;
                }
                self
            }
            Some(id) => self.map_space(id, |mut sp| {
                sp.density = density;
                sp
            }),
        }
    }

    pub fn set_wall(mut self, label: &str, on: bool) -> UiState {
        let has = self.wall.iter().any(|w| w == label);
        if on && !has {
            self.wall.push(label.to_string());
        } else if !on && has {
            self.wall.retain(|w| w != label);
        }
        self
    }

    pub fn toggle_collapsed(mut self, key: &str) -> UiState {
        if self.collapsed.iter().any(|k| k == key) {
            self.collapsed.retain(|k| k != key);
        } else {
            self.collapsed.push(key.to_string());
        }
        self
    }

    /// Drop `agent` on pane `pane_id`: a side splits it (its old pane closes
    /// first), the centre (`None`) swaps with the pane's agent, or takes the
    /// pane when the agent wasn't shown. With `fit`, a split leaving a tile
    /// below the minimum adds a chip instead.
    pub fn drop_on_pane(
        self,
        space_id: &str,
        agent: &str,
        pane_id: &str,
        zone: Option<Side>,
        fit: Option<Fit>,
    ) -> (UiState, DropOutcome, Option<String>) {
        let Some(target) = self
            .space(space_id)
            .and_then(|s| s.layout.find_pane(pane_id))
        else {
            return self.drop_on_space(space_id, agent, fit);
        };
        if target.agent_id.as_deref() == Some(agent) {
            return (self, DropOutcome::Noop, Some(pane_id.to_string()));
        }
        if zone.is_none() || target.agent_id.is_none() {
            let from = self.locate(agent);
            let displaced = target.agent_id.clone();
            let mut next = self;
            let outcome;
            match (&from, &displaced) {
                (Some(from), Some(d)) => {
                    outcome = DropOutcome::Swapped;
                    let d = d.clone();
                    next = next.map_space(&from.space_id, |mut x| {
                        x.layout = x.layout.set_pane_agent(&from.pane_id, Some(&d));
                        x.with_member(&d)
                    });
                    next = next.detach(agent, space_id);
                    let same = from.space_id == space_id;
                    next = next.map_space(space_id, |mut x| {
                        x.layout = x.layout.set_pane_agent(pane_id, Some(agent));
                        if same {
                            x
                        } else {
                            x.without_member(&d)
                        }
                    });
                }
                _ => {
                    outcome = if displaced.is_some() {
                        DropOutcome::Replaced
                    } else {
                        DropOutcome::Placed
                    };
                    next = next.detach(agent, space_id);
                    next = next.map_space(space_id, |mut x| {
                        if let Some(h) = x.layout.find_by_agent(agent) {
                            x.layout = remove_pane(&x.layout, &h.id).unwrap_or(x.layout.clone());
                        }
                        x.layout = x.layout.set_pane_agent(pane_id, Some(agent));
                        match &displaced {
                            Some(d) => x.with_member(d),
                            None => x,
                        }
                    });
                }
            }
            next = next.map_space(space_id, |x| {
                let mut x = x.with_member(agent);
                x.focused_pane_id = Some(pane_id.to_string());
                x.tidy()
            });
            return (next, outcome, Some(pane_id.to_string()));
        }
        let side = zone.expect("a side");
        let next = self.clone().detach(agent, space_id);
        let here = next.space(space_id).expect("the space").layout.clone();
        let before = match here.find_by_agent(agent) {
            Some(h) => remove_pane(&here, &h.id).unwrap_or_else(empty_layout),
            None => here,
        };
        if before.find_pane(pane_id).is_none() {
            return self.drop_on_space(space_id, agent, fit);
        }
        let (t, fresh) = split_pane(&before, pane_id, side, Some(agent));
        if !room_for(&before, &t, &fresh, fit) {
            return self.into_chip(space_id, agent);
        }
        let state = next.map_space(space_id, |x| {
            let mut x = x.with_member(agent);
            x.layout = t;
            x.focused_pane_id = Some(fresh.clone());
            x.maximized_pane_id = None;
            x.tidy()
        });
        (state, DropOutcome::Placed, Some(fresh))
    }

    /// Drop on a space (its tab or area): an empty pane, else split the
    /// roomiest pane, else a chip. Already shown here: just focus it.
    pub fn drop_on_space(
        self,
        space_id: &str,
        agent: &str,
        fit: Option<Fit>,
    ) -> (UiState, DropOutcome, Option<String>) {
        let Some(sp) = self.space(space_id) else {
            return (self, DropOutcome::Noop, None);
        };
        if let Some(here) = sp.layout.find_by_agent(agent) {
            let id = here.id.clone();
            return (self.focus_pane(space_id, &id), DropOutcome::Noop, Some(id));
        }
        let next = self.clone().detach(agent, space_id);
        let layout = next.space(space_id).expect("the space").layout.clone();
        if let Some(empty) = layout.leaves().into_iter().find(|p| p.agent_id.is_none()) {
            let state = next.map_space(space_id, |x| {
                let mut x = x.with_member(agent);
                x.layout = x.layout.set_pane_agent(&empty.id, Some(agent));
                x.focused_pane_id = Some(empty.id.clone());
                x.tidy()
            });
            return (state, DropOutcome::Placed, Some(empty.id));
        }
        let (spot, side) = auto_spot(&layout, fit);
        let (t, fresh) = split_pane(&layout, &spot, side, Some(agent));
        if !room_for(&layout, &t, &fresh, fit) {
            return self.into_chip(space_id, agent);
        }
        let state = next.map_space(space_id, |x| {
            let mut x = x.with_member(agent);
            x.layout = t;
            x.focused_pane_id = Some(fresh.clone());
            x.maximized_pane_id = None;
            x.tidy()
        });
        (state, DropOutcome::Placed, Some(fresh))
    }

    fn into_chip(self, space_id: &str, agent: &str) -> (UiState, DropOutcome, Option<String>) {
        let next = self.detach(agent, space_id).map_space(space_id, |mut sp| {
            if let Some(h) = sp.layout.find_by_agent(agent) {
                sp.layout = if sp.layout.leaves().len() > 1 {
                    remove_pane(&sp.layout, &h.id).unwrap_or_else(empty_layout)
                } else {
                    sp.layout.set_pane_agent(&h.id, None)
                };
            }
            sp.with_member(agent).tidy()
        });
        (next, DropOutcome::Chip, None)
    }
}

fn folded(layout: &Node, fit: Option<Fit>, keep: Option<&str>) -> Vec<String> {
    match fit {
        None => vec![],
        Some(f) => fit_layout(layout, f.area, f.min, keep)
            .1
            .into_iter()
            .map(|p| p.id)
            .collect(),
    }
}

fn room_for(before: &Node, after: &Node, fresh: &str, fit: Option<Fit>) -> bool {
    if fit.is_none() {
        return true;
    }
    let hidden = folded(after, fit, Some(fresh));
    !hidden.iter().any(|h| h == fresh) && hidden.len() <= folded(before, fit, None).len()
}

/// The pane (and side) whose split leaves the roomiest new tile.
fn auto_spot(layout: &Node, fit: Option<Fit>) -> (String, Side) {
    let (area, min) = match fit {
        Some(f) => (f.area, f.min),
        None => (
            (1600.0, 900.0),
            MinSize {
                min_w: 1.0,
                min_h: 1.0,
            },
        ),
    };
    let mut best = (layout.leaves()[0].id.clone(), Side::Right, -1.0);
    for (id, r) in pane_rects(
        layout,
        Rect {
            x: 0.0,
            y: 0.0,
            w: area.0,
            h: area.1,
        },
    ) {
        let right = (r.w / 2.0 / min.min_w).min(r.h / min.min_h);
        let bottom = (r.w / min.min_w).min(r.h / 2.0 / min.min_h);
        let (side, score) = if right >= bottom {
            (Side::Right, right)
        } else {
            (Side::Bottom, bottom)
        };
        if score > best.2 + 1e-9 {
            best = (id, side, score);
        }
    }
    (best.0, best.1)
}

// ── the file ─────────────────────────────────────────────────────────────

pub fn ui_file(root: &Path) -> PathBuf {
    root.join("ui.json")
}

/// `ui.json` under the data folder (defaults when missing or unreadable).
pub fn load(root: &Path) -> UiState {
    std::fs::read_to_string(ui_file(root))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(UiState::sanitize)
        .unwrap_or_default()
}

/// Write atomically (temp file + rename), as the Tauri app does.
pub fn save(root: &Path, state: &UiState) -> std::io::Result<()> {
    let path = ui_file(root);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(root)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::tests::agent;
    use serde_json::json;

    fn agents() -> Vec<AgentView> {
        vec![
            agent("a", "/work/alpha", "idle", 1, true),
            agent("b", "/work/alpha", "working", 2, true),
            agent("c", "/work/beta", "blocked", 3, true),
        ]
    }

    #[test]
    fn hand_over_tiles_new_agents_first() {
        let s = UiState::default().place_agent(ALL_SPACE, "old", None, None);
        let ids: Vec<String> = ["n1", "n2", "n3"].iter().map(|s| s.to_string()).collect();
        let s = s.tile_agents(ALL_SPACE, &ids, 2);
        let sp = s.space(ALL_SPACE).unwrap();
        assert_eq!(sp.layout.agents(), vec!["n1".to_string(), "n2".to_string()], "capped");
        assert_eq!(sp.focused_agent().as_deref(), Some("n1"));
        let s = UiState::default()
            .place_agent(ALL_SPACE, "old", None, None)
            .tile_agents(ALL_SPACE, &ids[..1], 6);
        assert_eq!(
            s.space(ALL_SPACE).unwrap().layout.agents(),
            vec!["n1".to_string(), "old".to_string()],
            "what was shown follows"
        );
    }

    #[test]
    fn placing_keeps_one_pane_per_agent() {
        let s = UiState::default().place_agent(ALL_SPACE, "a", None, None);
        assert_eq!(s.locate("a").unwrap().space_id, ALL_SPACE);
        let (s, custom) = s.create_custom_space(MAIN);
        let s = s.place_agent(&custom, "a", None, None);
        assert_eq!(s.locate("a").unwrap().space_id, custom, "moved, not copied");
        assert!(s.space(ALL_SPACE).unwrap().layout.agents().is_empty());
        assert_eq!(s.space(&custom).unwrap().members, vec!["a".to_string()]);
        let s = s.place_agent(&custom, "b", None, Some(Side::Right));
        assert_eq!(
            s.space(&custom).unwrap().layout.agents(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            s.space(&custom).unwrap().focused_agent().as_deref(),
            Some("b")
        );
    }

    #[test]
    fn presets_fill_with_free_members_focused_first() {
        let s = UiState::default().place_agent(ALL_SPACE, "c", None, None);
        let s = s.apply_preset(ALL_SPACE, Preset::G2x2, &agents(), |n| vec![n]);
        let sp = s.space(ALL_SPACE).unwrap();
        assert_eq!(sp.layout.leaves().len(), 4);
        assert_eq!(sp.layout.agents()[0], "c");
        assert_eq!(sp.layout.agents().len(), 3);
        let s = s.apply_preset(ALL_SPACE, Preset::Auto, &agents(), |n| vec![n]);
        assert_eq!(s.space(ALL_SPACE).unwrap().layout.leaves().len(), 3);
    }

    #[test]
    fn maximise_needs_two_panes_and_closing_tidies() {
        let s = UiState::default().place_agent(ALL_SPACE, "a", None, None);
        let s = s.clone().toggle_maximize(ALL_SPACE, None);
        assert_eq!(s.space(ALL_SPACE).unwrap().maximized_pane_id, None);
        let s = s.place_agent(ALL_SPACE, "b", None, Some(Side::Right));
        let s = s.toggle_maximize(ALL_SPACE, None);
        let max = s
            .space(ALL_SPACE)
            .unwrap()
            .maximized_pane_id
            .clone()
            .unwrap();
        let s = s.close_pane(ALL_SPACE, &max);
        let sp = s.space(ALL_SPACE).unwrap();
        assert_eq!(sp.maximized_pane_id, None);
        assert_eq!(sp.layout.agents(), vec!["a".to_string()]);
    }

    #[test]
    fn drops_swap_split_and_fall_back_to_chips() {
        let s = UiState::default()
            .place_agent(ALL_SPACE, "a", None, None)
            .place_agent(ALL_SPACE, "b", None, Some(Side::Right));
        let pa = s.locate("a").unwrap().pane_id;
        let pb = s.locate("b").unwrap().pane_id;
        let (s2, out, _) = s.clone().drop_on_pane(ALL_SPACE, "a", &pb, None, None);
        assert_eq!(out, DropOutcome::Swapped);
        assert_eq!(s2.locate("a").unwrap().pane_id, pb);
        assert_eq!(s2.locate("b").unwrap().pane_id, pa);
        let (s3, out, _) = s.clone().drop_on_pane(ALL_SPACE, "a", &pa, None, None);
        assert_eq!(out, DropOutcome::Noop);
        assert_eq!(s3, s);
        let tiny = Fit {
            area: (500.0, 300.0),
            min: MinSize {
                min_w: 300.0,
                min_h: 200.0,
            },
        };
        let (s4, out, _) =
            s.clone()
                .drop_on_pane(ALL_SPACE, "c", &pb, Some(Side::Bottom), Some(tiny));
        assert_eq!(out, DropOutcome::Chip);
        assert!(s4.locate("c").is_none());
        let (s5, out, p) = s.drop_on_space(ALL_SPACE, "c", None);
        assert_eq!(out, DropOutcome::Placed);
        assert_eq!(s5.locate("c").unwrap().pane_id, p.unwrap());
    }

    #[test]
    fn spaces_open_rename_close_and_prune() {
        let (s, id) =
            UiState::default().open_project_space("/work/alpha", "alpha", &agents(), MAIN);
        assert_eq!(s.space(&id).unwrap().layout.agents().len(), 2);
        let (s, again) = s.open_project_space("/work/alpha", "alpha", &agents(), MAIN);
        assert_eq!(id, again, "reused");
        let s = s.rename_space(&id, "  ").rename_space(&id, "Alpha work");
        assert_eq!(s.space(&id).unwrap().name, "Alpha work");
        let s = s.prune_agents(&["b".to_string()]);
        assert_eq!(s.space(&id).unwrap().layout.agents(), vec!["b".to_string()]);
        let s = s.close_space(&id).close_space(ALL_SPACE);
        assert_eq!(s.spaces.len(), 1);
        assert!(s.window_of.is_empty());
    }

    #[test]
    fn the_file_keeps_unknown_fields_and_the_react_shape() {
        let raw = json!({
            "v": 1, "theme": "dark", "look": "glass", "wallCollapsed": ["x"],
            "spaces": [
                {"id": "s1", "name": "One", "kind": "custom", "members": ["a"],
                 "layout": {"type": "pane", "id": "p1", "agentId": "a"},
                 "focusedPaneId": "p1", "maximizedPaneId": null, "density": "dense"},
                {"broken": true}
            ],
            "windowOf": {"s1": "main"}, "density": "comfortable", "fontSize": 14
        });
        let s = UiState::sanitize(raw);
        assert_eq!(s.spaces[0].id, ALL_SPACE, "All is always there");
        assert_eq!(s.spaces.len(), 2);
        assert_eq!(s.density, Density::Comfortable);
        assert_eq!(s.space("s1").unwrap().density, Some(Density::Dense));
        let out = serde_json::to_value(&s).unwrap();
        assert_eq!(out["theme"], "dark");
        assert_eq!(out["wallCollapsed"], json!(["x"]));
        assert_eq!(out["spaces"][1]["layout"]["agentId"], "a");
        assert_eq!(out["spaces"][1]["focusedPaneId"], "p1");
        let fresh = UiState::sanitize(json!({"v": 2}));
        assert_eq!(fresh.spaces.len(), 1);
        assert_eq!(fresh.spaces[0].id, ALL_SPACE);
    }

    #[test]
    fn saves_and_loads_atomically() {
        let dir = std::env::temp_dir().join(format!("pw-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (s, id) = UiState::default().create_custom_space(MAIN);
        save(&dir, &s).unwrap();
        assert_eq!(load(&dir).space(&id).unwrap().name, "Space 1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn density_minimums() {
        let m = fold_min(Density::Compact, 13.0);
        assert_eq!((m.min_w, m.min_h), (388.0, 203.0));
    }
}
