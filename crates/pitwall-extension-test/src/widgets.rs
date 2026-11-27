//! Pitwall's components in the harness (ui.md §3.10): what the user does
//! to a checkbox, a radio group, a slider, a number field, a split, a
//! chip, a popover or a confirm sheet, sent as Pitwall sends it, and how
//! the text snapshot shows them. What a key or a click does to a control
//! comes from the tile host's own rules (`pitwall_tile_host::widgets`),
//! so the harness and Pitwall agree.

use pitwall_tile_host::ui::{self, ControlValue, Element, Event, Listen, Node};
use pitwall_tile_host::widgets::{self as w, Outcome};

use crate::{Found, Session};

fn tone(t: &Option<ui::Tone>) -> String {
    match t {
        None => String::new(),
        Some(ui::Tone::Info) => " info".into(),
        Some(ui::Tone::Good) => " good".into(),
        Some(ui::Tone::Warn) => " warn".into(),
        Some(ui::Tone::Bad) => " bad".into(),
    }
}

fn flag(on: bool, name: &str) -> String {
    if on {
        format!(" {name}")
    } else {
        String::new()
    }
}

fn opt(name: &str, v: &Option<String>) -> String {
    v.as_ref().map(|v| format!(" {name}={v:?}")).unwrap_or_default()
}

/// A component in the text snapshot (`now_ms`: the harness's clock, for
/// relative times); `None` for elements that aren't components.
pub(crate) fn describe(e: &Element, now_ms: i64) -> Option<String> {
    Some(match e {
        Element::Checkbox(c) | Element::Switch(c) => {
            let kind = if matches!(e, Element::Checkbox(_)) { "checkbox" } else { "switch" };
            let state = if c.mixed { "mixed" } else if c.checked { "on" } else { "off" };
            format!("{kind} {:?} {state}{}", c.label, flag(c.disabled, "disabled"))
        }
        Element::RadioGroup(g) | Element::Segmented(g) => {
            let kind = if matches!(e, Element::RadioGroup(_)) { "radio-group" } else { "segmented" };
            let items: Vec<String> = g.items.iter().map(|i| format!("{}{}{}", if g.selected.as_ref() == Some(&i.key) { "(•) " } else { "( ) " }, i.label, if i.disabled { " disabled" } else { "" })).collect();
            format!("{kind} [{}]{}", items.join(", "), flag(g.disabled, "disabled"))
        }
        Element::Slider(s) => format!("slider {} in {}..{}{}{}", w::number(s.value, None), w::number(s.min, None), w::number(s.max, None), if s.step > 0. { format!(" step {}", w::number(s.step, None)) } else { String::new() }, flag(s.disabled, "disabled")),
        Element::NumberField(f) => {
            let v = f.value.map_or("empty".to_string(), |v| w::format_number(v, f.decimals));
            format!("number-field {v}{}{}", flag(f.readonly, "readonly"), flag(f.disabled, "disabled"))
        }
        Element::SearchField(_) => unreachable!("described as a text field"),
        Element::FormRow(r) => format!("form-row {:?}{}{}{}", r.label, flag(r.required, "required"), opt("help", &r.help), opt("error", &r.error)),
        Element::Progress(p) => match p.value {
            Some(v) => format!("progress {:.0}%{}", v.clamp(0., 1.) * 100., tone(&p.tone)),
            None => format!("progress indeterminate{}", tone(&p.tone)),
        },
        Element::Spinner => "spinner".into(),
        Element::Skeleton(s) => match s {
            ui::Skeleton::Lines(n) => format!("skeleton {n} lines"),
            ui::Skeleton::Block => "skeleton block".into(),
            ui::Skeleton::Circle => "skeleton circle".into(),
        },
        Element::EmptyState(s) => format!("empty-state {:?}{}{}", s.title, opt("text", &s.text), opt("action", &s.action)),
        Element::Banner(b) => format!("banner{} {:?}{}{}{}", tone(&Some(b.tone)), b.text, opt("title", &b.title), opt("action", &b.action), flag(b.dismissible, "dismissible")),
        Element::Badge(b) => format!("badge {:?}{}", b.text, tone(&b.tone)),
        Element::Chip(c) => format!("chip {:?}{}{}{}{}", c.label, tone(&c.tone), flag(c.selected, "selected"), flag(c.removable, "removable"), flag(c.disabled, "disabled")),
        Element::Kbd(k) => format!("kbd {k:?}"),
        Element::Split(s) => format!("split {} {:.2}", if s.orientation == ui::Orientation::Horizontal { "horizontal" } else { "vertical" }, s.initial),
        Element::Disclosure(d) => format!("disclosure {:?} {}{}", d.title, if d.open { "open" } else { "closed" }, opt("detail", &d.detail)),
        Element::Card(c) => format!("card{}{}", opt("title", &c.title), opt("detail", &c.detail)),
        Element::Divider(o) => format!("divider{}", if *o == ui::Orientation::Vertical { " vertical" } else { "" }),
        Element::Breadcrumbs(c) => format!("breadcrumbs {}", c.iter().map(|c| c.label.as_str()).collect::<Vec<_>>().join(" / ")),
        Element::Toolbar => "toolbar".into(),
        Element::Popover(_) => "popover".into(),
        Element::ConfirmSheet(s) => format!("confirm-sheet {:?} confirm={:?}{}{}", s.title, s.confirm, opt("text", &s.text), flag(s.danger, "danger")),
        Element::KeyValues(kv) => format!("key-values {}", kv.iter().map(|k| format!("{}={:?}", k.key, k.value)).collect::<Vec<_>>().join(" ")),
        Element::Avatar(a) => format!("avatar {:?} {}", a.name, w::initials(&a.name)),
        Element::RelativeTime(t) => format!("relative-time {:?}", w::relative_time(t.at, now_ms)),
        Element::Copyable(c) => format!("copyable {:?}", c.text),
        Element::Link(l) => format!("link {:?}{}", l.label, opt("url", &l.url)),
        Element::Sparkline(s) => format!("sparkline {} values", s.values.len()),
        Element::BarChart(_) | Element::LineChart(_) | Element::StatusTimeline(_) => {
            let kind = match e {
                Element::BarChart(_) => "bar-chart",
                Element::LineChart(_) => "line-chart",
                _ => "status-timeline",
            };
            format!("{kind} {:?}", w::chart_summary(e).unwrap_or_default())
        }
        _ => return None,
    })
}

impl Session {
    /// The component at `key_path` (or with that key) in the main view.
    fn component(&self, key_path: &str) -> Found {
        match self.find_key(key_path) {
            Some(f) => f,
            None => panic!("there is no node “{key_path}”; it shows:\n{}", self.text_snapshot()),
        }
    }

    fn listener(found: &Found, on: Listen) -> Option<u32> {
        found.node.on.iter().find(|l| l.on == on).map(|l| l.handler)
    }

    /// Send what `outcome` says to the component `found`, as Pitwall
    /// would. `what`: the action, for the panic when it can't.
    fn apply(&mut self, found: &Found, outcome: Outcome, what: &str) -> &mut Self {
        let event = match outcome {
            Outcome::Value(value) => {
                let Some(handler) = Session::listener(found, Listen::Value) else { panic!("“{}” can't be changed: it has no on_change listener ({what})", found.key_path) };
                Event::ValueChanged(ui::ValueChange { handler, value })
            }
            Outcome::Dismiss => {
                let Some(handler) = Session::listener(found, Listen::Dismiss) else { panic!("“{}” has no dismiss listener ({what})", found.key_path) };
                Event::Dismissed(handler)
            }
            Outcome::Click => {
                let Some(handler) = Session::listener(found, Listen::Click) else { panic!("“{}” has no click listener ({what})", found.key_path) };
                Event::Click(ui::Click { handler, button: ui::MouseButton::Left, modifiers: ui::Modifiers::empty(), count: 1, at: ui::Point { x: 1., y: 1. } })
            }
        };
        self.take_focus();
        if w::focusable(&found.node.element, Session::listener(found, Listen::Click).is_some()) {
            let (view, path) = (found.view.clone(), found.key_path.clone());
            self.move_focus(&view, Some(path));
        }
        self.send(event);
        self.settle()
    }

    /// Click the checkbox or switch at `key_path` (or a disclosure's
    /// header), as the user would: it sends the other state. Panics when
    /// it is disabled or isn't one.
    pub fn toggle(&mut self, key_path: &str) -> &mut Self {
        let f = self.component(key_path);
        let Some(o) = w::on_click(&f.node.element) else { panic!("“{key_path}” isn't a checkbox, switch or disclosure that can be toggled now") };
        self.apply(&f, o, "toggle")
    }

    /// Choose `key` in the radio group or segmented control at
    /// `key_path`, click the breadcrumb `key`, or the status timeline's
    /// segment `key`. Panics when there is no such choice or it is
    /// disabled.
    pub fn choose(&mut self, key_path: &str, key: &str) -> &mut Self {
        let f = self.component(key_path);
        let ok = match &f.node.element {
            Element::RadioGroup(g) | Element::Segmented(g) => !g.disabled && g.items.iter().any(|i| i.key == key && !i.disabled),
            Element::Breadcrumbs(c) => c.iter().take(c.len().saturating_sub(1)).any(|c| c.key == key),
            Element::StatusTimeline(t) => t.segments.iter().any(|s| s.key == key),
            _ => panic!("“{key_path}” isn't a radio group, segmented control, breadcrumbs or status timeline"),
        };
        assert!(ok, "“{key_path}” has no choice “{key}” that can be picked");
        self.apply(&f, Outcome::Value(ControlValue::Choice(key.to_string())), "choose")
    }

    /// Drag the slider at `key_path` to `value` (kept in its range and
    /// snapped to its step, as Pitwall does) and let go.
    pub fn set_slider(&mut self, key_path: &str, value: f64) -> &mut Self {
        let f = self.component(key_path);
        let Element::Slider(s) = &f.node.element else { panic!("“{key_path}” isn't a slider") };
        assert!(!s.disabled, "the slider “{key_path}” is disabled");
        let v = w::snap(value, s.min, s.max, s.step);
        self.apply(&f, Outcome::Value(ControlValue::Number(v)), "set_slider")
    }

    /// Type `text` into the number field at `key_path` and press Enter:
    /// it sends the number (kept within its bounds), or nothing when the
    /// text isn't a number (the field puts its value back). Returns
    /// whether anything was sent.
    pub fn type_number(&mut self, key_path: &str, text: &str) -> bool {
        let f = self.component(key_path);
        let Element::NumberField(n) = &f.node.element else { panic!("“{key_path}” isn't a number field") };
        assert!(!n.disabled && !n.readonly, "the number field “{key_path}” can't be changed");
        match w::parse_number(text, n) {
            Some(v) if v != n.value => {
                let value = v.map_or(ControlValue::Empty, ControlValue::Number);
                self.apply(&f, Outcome::Value(value), "type_number");
                true
            }
            _ => false,
        }
    }

    /// Click the + (`up`) or − of the number field at `key_path`.
    pub fn step_number(&mut self, key_path: &str, up: bool) -> &mut Self {
        let f = self.component(key_path);
        let Element::NumberField(n) = &f.node.element else { panic!("“{key_path}” isn't a number field") };
        let Some(v) = w::step_number(n, up) else { panic!("“{key_path}” can't step {}", if up { "up" } else { "down" }) };
        self.apply(&f, Outcome::Value(ControlValue::Number(v)), "step_number")
    }

    /// Drag the bar of the split at `key_path` so its first pane has
    /// `share` of it (0.0–1.0) and let go.
    pub fn resize_split(&mut self, key_path: &str, share: f32) -> &mut Self {
        let f = self.component(key_path);
        let Element::Split(s) = &f.node.element else { panic!("“{key_path}” isn't a split") };
        let share = w::split_share(share, 0., s.min_first, s.min_second);
        self.apply(&f, Outcome::Value(ControlValue::Number(f64::from(share))), "resize_split")
    }

    /// Close or remove the banner, chip, popover or confirm sheet at
    /// `key_path` (its ×, Esc, a click outside it, Cancel).
    pub fn dismiss(&mut self, key_path: &str) -> &mut Self {
        let f = self.component(key_path);
        self.apply(&f, Outcome::Dismiss, "dismiss")
    }

    /// Press `key` (as GPUI names it: `space`, `enter`, `left`, `right`,
    /// `up`, `down`, `home`, `end`, `pageup`, `pagedown`, `backspace`,
    /// `delete`, `escape`) while the component at `key_path` has focus, as
    /// Pitwall handles it. Returns whether it did anything.
    pub fn key_on(&mut self, key_path: &str, key: &str) -> bool {
        let f = self.component(key_path);
        match w::on_key(&f.node.element, key) {
            Some(o) => {
                self.apply(&f, o, "key_on");
                true
            }
            None => false,
        }
    }

    /// Click the copy button of the copyable at `key_path`: its text goes
    /// to [`Session::clipboard`], and its click listener hears it.
    pub fn copy(&mut self, key_path: &str) -> &mut Self {
        let f = self.component(key_path);
        let text = match &f.node.element {
            Element::Copyable(c) => c.text.clone(),
            _ => panic!("“{key_path}” isn't a copyable"),
        };
        self.clipboard = Some(text);
        if Session::listener(&f, Listen::Click).is_some() {
            self.apply(&f, Outcome::Click, "copy");
        }
        self
    }

    /// Click the link at `key_path`: with a web address and the `show`
    /// permission Pitwall opens the page (it is in
    /// [`Session::opened_pages`]); otherwise its click listener hears it.
    pub fn open_link(&mut self, key_path: &str) -> &mut Self {
        let f = self.component(key_path);
        let Element::Link(l) = &f.node.element else { panic!("“{key_path}” isn't a link") };
        let url = l.url.clone().filter(|u| pitwall_core::extensions::view::web_view_url(u).is_ok());
        match url {
            Some(u) if self.grants.iter().any(|g| g == "show") => {
                self.opened_pages.push(u);
                self
            }
            _ => self.apply(&f, Outcome::Click, "open_link"),
        }
    }

    /// Empty the search field at `key_path` with its clear button.
    pub fn clear_search(&mut self, key_path: &str) -> &mut Self {
        self.type_into(key_path, "")
    }

    /// What the last [`Session::copy`] put on the clipboard.
    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard.as_deref()
    }

    /// The web pages links opened, oldest first.
    pub fn opened_pages(&self) -> &[String] {
        &self.opened_pages
    }
}

/// A search field as a text field: its value and revision, for the
/// harness's field bookkeeping.
pub(crate) fn field_value(n: &Node) -> Option<(Option<&String>, u64)> {
    match &n.element {
        Element::Input(i) => Some((i.value.as_ref(), i.revision)),
        Element::SearchField(s) => Some((s.value.as_ref(), s.revision)),
        _ => None,
    }
}
