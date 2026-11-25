//! What the floating control shows, as plain data: the agents whose
//! screens ask the user something, each one's prompt as lines, the open
//! questions in extensions' inboxes, and how large the window is for them
//! (it has no scrolling: it is sized to its content).

use std::collections::{HashMap, HashSet};

use pitwall_core::extensions::protocol::{Ask, Thread, ThreadDetail};
use pitwall_proto::{AgentView, AskView, Status};

/// Clear space around what is drawn, inside the window: room for its
/// shadow.
pub const PAD: f32 = 14.;
/// Collapsed: a pill that says who waits, what it wants and how many wait,
/// with Accept and Reject for a plain yes/no.
pub const PILL_W: f32 = 420.;
pub const PILL_H: f32 = 60.;
/// The panel's width with a card open, until the user drags its edge
/// (then [`Shape::width`], between [`MIN_WIDTH`] and [`MAX_WIDTH`]).
pub const WIDTH: f32 = 520.;
pub const MIN_WIDTH: f32 = 400.;
pub const MAX_WIDTH: f32 = 1100.;
/// The header: who asks, "1 of 3" with the arrows, Accept all, and the
/// button back to the pill.
pub const HEADER_H: f32 = 48.;
/// The summary: its label ("Run", "Edit", "Question"), and each line of
/// what it names (the command, the file, the question), at most
/// [`SUMMARY_LINES`] of them.
pub const SUMMARY_LABEL_H: f32 = 18.;
pub const SUMMARY_LINE: f32 = 20.;
pub const SUMMARY_LINES: usize = 3;
/// The row over the terminal's rows when a summary is above them: its
/// label and "Show all".
pub const RAW_HEAD_H: f32 = 24.;
/// Rows of the terminal shown under a summary until "Show all".
pub const RAW_FEW: usize = 5;
/// The prompt's font size (JetBrains Mono), and the height of one of its
/// lines: a terminal row's at that size (`wall::paint::cell_height`,
/// checked by a test).
pub const TERM_FONT: f32 = 10.5;
pub const LINE_H: f32 = 16.;
/// The prompt box's own padding, top and bottom together.
pub const BOX_PAD: f32 = 16.;
/// The buttons under the prompt.
pub const BUTTONS_H: f32 = 30.;
/// A line under the buttons: sent, why it wasn't, or how to answer.
pub const NOTE_H: f32 = 22.;
/// The keyboard hints under the card.
pub const HINT_H: f32 = 30.;
/// Space above a card's first row, around its parts, and below its buttons.
pub const GAP: f32 = 10.;
/// The panel's border, top and bottom together.
pub const BORDER: f32 = 2.;
/// The hairline under the header and over the hints.
pub const RULE: f32 = 1.;

/// Threads with open questions whose messages the control asks their
/// extension for (the first, in the inbox's order), so it can show them.
pub const ASK_THREADS: usize = 6;
/// A question card's parts, for its height before it is measured (the
/// drawn card says its own: [`super::Float`] keeps it).
pub const CARD_HEAD: f32 = 26.;
pub const QUESTION_LINE: f32 = 19.;
pub const OPTION_H: f32 = 30.;
pub const CHIP_H: f32 = 28.;
pub const ANSWER_H: f32 = 30.;
/// Characters of the prompt per line: what fits [`WIDTH`] in its font
/// (a wider or narrower panel scales it, [`wrap_cols`]).
pub const WRAP_COLS: usize = 74;
/// Rows of the prompt shown with nothing above it to summarise it (or
/// after "Show all"); a longer prompt keeps its top. The user can drag
/// that between [`MIN_ROWS`] and [`MAX_ROWS`].
pub const LINES_ALONE: usize = 16;
pub const MIN_ROWS: usize = 3;
pub const MAX_ROWS: usize = 40;

/// The size the user gave the open panel by dragging its edges: its width,
/// and the rows a card shows when one agent asks (`None`: the default,
/// and a short prompt shows only itself; set, a card shows that many rows
/// of its terminal, the ones above a short prompt too, so that making it
/// taller always shows more).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub width: f32,
    pub rows: Option<usize>,
}

impl Default for Shape {
    fn default() -> Self {
        Shape { width: WIDTH, rows: None }
    }
}

impl Shape {
    /// Rows of prompt a card shows in full.
    pub fn max_lines(&self) -> usize {
        self.rows.unwrap_or(LINES_ALONE)
    }

    /// Characters of prompt text per line.
    pub fn cols(&self) -> usize {
        wrap_cols(self.width)
    }

    /// A card fills its rows with the terminal above a short prompt.
    pub fn fill(&self) -> bool {
        self.rows.is_some()
    }
}

/// The sizes one click in the header gives the open panel. Medium is the
/// default (what a double click on an edge goes back to).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Small,
    Medium,
    Large,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Small, Preset::Medium, Preset::Large];

    /// Its width and rows when one agent asks, as the user's size keeps
    /// them (`None`: the default's).
    pub fn size(self) -> (Option<f32>, Option<usize>) {
        match self {
            Preset::Small => (Some(460.), Some(8)),
            Preset::Medium => (None, None),
            Preset::Large => (Some(860.), Some(28)),
        }
    }

    /// The panel has this size (a size dragged to is none of them).
    pub fn of(width: Option<f32>, rows: Option<usize>) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.size() == (width, rows))
    }

    /// Its letter on the button, and its name.
    pub fn letter(self) -> &'static str {
        match self {
            Preset::Small => "S",
            Preset::Medium => "M",
            Preset::Large => "L",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Small => "small",
            Preset::Medium => "medium",
            Preset::Large => "large",
        }
    }
}

/// Characters of the prompt per line in a panel `width` wide.
pub fn wrap_cols(width: f32) -> usize {
    ((WRAP_COLS as f32 * width / WIDTH).floor() as usize).max(1)
}

/// An agent that needs the user.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub name: String,
    /// Its project, as the sidebar names it.
    pub project: String,
    pub kind: String,
    pub kind_name: String,
    /// The question it asks or the permission it wants, when known.
    pub detail: Option<String>,
    /// Its prompt as drawn, and whether it is a yes/no Pitwall can answer.
    pub ask: AskView,
    /// This Pitwall runs it (another machine's agents are read-only here).
    pub local: bool,
}

impl Item {
    /// Accept can be offered: a yes/no on this machine.
    pub fn can_accept(&self) -> bool {
        self.local && self.ask.accept
    }

    pub fn can_reject(&self) -> bool {
        self.local && self.ask.accept && self.ask.reject
    }

    /// The prompt's lines as shown, at most `max`, of at most `cols`
    /// characters. A yes/no loses its list of choices: the buttons under
    /// it are those.
    pub fn lines(&self, max: usize, cols: usize) -> Vec<String> {
        let text = if self.ask.accept { without_choices(&self.ask.text) } else { self.ask.text.clone() };
        wrap(&text, cols, max)
    }
}

/// The agents whose screens ask the user something, in the store's order.
/// A blocked agent with nothing on its screen (its hooks say it waits,
/// e.g. while Claude Code's auto mode checks a command itself) has nothing
/// to answer here and is left out.
pub fn items(agents: &[AgentView]) -> Vec<Item> {
    agents
        .iter()
        .filter(|a| a.status == Status::Blocked)
        .filter_map(|a| {
            Some(Item {
                id: a.id.clone(),
                name: a.name.clone(),
                project: a.project_display.clone(),
                kind: a.kind.clone(),
                kind_name: a.kind_name.clone(),
                detail: a.status_detail.clone(),
                ask: a.ask.clone()?,
                local: !crate::agents::mirrored(a),
            })
        })
        .collect()
}

/// `items` oldest first: in the order they began to ask (`since`: a
/// number per agent, lower is earlier; one not in it is newest), the
/// store's order among equals.
pub fn oldest_first(mut items: Vec<Item>, since: &HashMap<String, u64>) -> Vec<Item> {
    items.sort_by_key(|i| since.get(&i.id).copied().unwrap_or(u64::MAX));
    items
}

/// What waits, one card at a time: an agent, or a question in an
/// extension's inbox.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Entry<'a> {
    Agent(&'a Item),
    Ask(&'a AskItem),
}

impl Entry<'_> {
    /// What it is known by: the agent's id, the question's key.
    pub fn key(&self) -> String {
        match self {
            Entry::Agent(i) => i.id.clone(),
            Entry::Ask(a) => a.key(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Entry::Agent(i) => &i.name,
            Entry::Ask(a) => a.who(),
        }
    }
}

/// Everything that waits, in the order the cards go through it: the
/// agents (oldest first, as given), then the questions.
pub fn entries<'a>(items: &'a [Item], asks: &'a [AskItem]) -> Vec<Entry<'a>> {
    items.iter().map(Entry::Agent).chain(asks.iter().map(Entry::Ask)).collect()
}

/// Where the card shown is in `entries`: the one keyed `current`, else
/// the first.
pub fn position(entries: &[Entry], current: Option<&str>) -> usize {
    current.and_then(|c| entries.iter().position(|e| e.key() == c)).unwrap_or(0)
}

/// The entry `by` steps from `at` (around at either end).
pub fn step(len: usize, at: usize, by: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (at as isize + by).rem_euclid(len as isize) as usize
}

/// The first entry from `at` on (around) not answered yet, else `at`.
pub fn next_waiting(entries: &[Entry], at: usize, answered: &dyn Fn(&str) -> bool) -> usize {
    (0..entries.len()).map(|k| step(entries.len(), at, k as isize)).find(|&i| !answered(&entries[i].key())).unwrap_or(at)
}

/// "Accept all" is offered: two or more wait, every one of them a yes/no
/// this machine can answer, and one not answered yet.
pub fn accept_all(entries: &[Entry], answered: &dyn Fn(&str) -> bool) -> bool {
    entries.len() >= 2 && entries.iter().all(|e| matches!(e, Entry::Agent(i) if i.can_accept())) && entries.iter().any(|e| !answered(&e.key()))
}

/// Names shared by two of `entries`: those show their project too.
pub fn clashes(entries: &[Entry], name: &str) -> bool {
    entries.iter().filter(|e| e.name() == name).count() > 1
}

/// What a prompt asks for, read off its text: a label ("Run", "Edit",
/// "Create", "Question") and what it names (the command, the file, the
/// question), word for word. `partial`: the prompt names more than that
/// (a command on several lines): the rest is only in its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub label: &'static str,
    pub target: String,
    /// A command or a path (drawn in the terminal's font), not prose.
    pub code: bool,
    pub partial: bool,
}

/// The summary of `item`'s prompt, when its text has a shape this knows
/// (Claude Code's "Bash command", "Edit file" and "Create file" dialogs,
/// Codex's "$ command", or a question); `None` otherwise: the card then
/// shows the terminal's rows alone. Nothing is reworded: the target is a
/// line of the prompt.
pub fn summary(item: &Item) -> Option<Summary> {
    let text = if item.ask.accept { without_choices(&item.ask.text) } else { item.ask.text.clone() };
    let all: Vec<&str> = text.lines().collect();
    let first = all.iter().position(|l| !l.trim().is_empty())?;
    let head = all[first].trim();
    // The block of lines from `from` (its first that isn't blank) up to
    // the next blank line.
    let block = |from: usize| -> Vec<&str> {
        let start = all.iter().skip(from).position(|l| !l.trim().is_empty()).map(|i| i + from);
        start.map_or_else(Vec::new, |s| all[s..].iter().take_while(|l| !l.trim().is_empty()).map(|l| l.trim()).collect())
    };
    let file = |label: &'static str| {
        let lines = block(first + 1);
        Some(Summary { label, target: lines.first()?.to_string(), code: true, partial: false })
    };
    match head {
        "Bash command" => {
            let lines = block(first + 1);
            // A second line is the command's description (prose), or
            // more of the command.
            let partial = match lines.len() {
                0 | 1 => false,
                2 => !is_prose(lines[1]),
                _ => true,
            };
            return Some(Summary { label: "Run", target: lines.first()?.to_string(), code: true, partial });
        }
        "Edit file" => return file("Edit"),
        "Create file" => return file("Create"),
        _ => {}
    }
    // Codex: "Would you like to run the following command?", "$ command".
    if head.to_lowercase().starts_with("would you like to run") {
        let at = all.iter().position(|l| l.trim_start().starts_with("$ "))?;
        let lines = block(at);
        let cmd = lines.first()?.trim_start_matches("$ ").trim();
        return Some(Summary { label: "Run", target: cmd.to_string(), code: true, partial: lines.len() > 1 });
    }
    // A question: its line (what the rules read as the question, when it
    // is in the prompt; else the prompt's first line when that asks).
    if !item.ask.accept {
        let asked = item.detail.as_deref().map(str::trim).filter(|d| !d.is_empty() && all.iter().any(|l| l.trim() == *d));
        let line = asked.or_else(|| head.ends_with('?').then_some(head))?;
        return Some(Summary { label: "Question", target: line.to_string(), code: false, partial: false });
    }
    None
}

/// A line of prose (a command's description), not more of a command.
fn is_prose(l: &str) -> bool {
    l.chars().next().is_some_and(char::is_uppercase) && l.contains(' ') && !l.contains(['|', '&', ';', '$', '<', '>', '`', '=', '\\', '/'])
}

/// The one line the pill shows of what is asked: the summary ("Run: pnpm
/// test", "Edit src/a.ts", the question), else what the rules read as
/// the question, else the prompt's first line.
pub fn gist(item: &Item) -> String {
    match summary(item) {
        Some(s) if s.label == "Question" => s.target,
        Some(s) => format!("{}{} {}{}", s.label, if s.label == "Run" { ":" } else { "" }, s.target, if s.partial { " …" } else { "" }),
        None => item
            .detail
            .clone()
            .filter(|d| !d.trim().is_empty())
            .or_else(|| item.ask.text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string))
            .unwrap_or_default(),
    }
}

/// The pill can answer it in one click: a yes/no on this machine whose
/// summary says all it does (else the user looks first: the pill offers
/// to show it).
pub fn pill_answers(item: &Item) -> bool {
    item.can_accept() && summary(item).is_some_and(|s| !s.partial)
}

/// The summary's target as drawn in a panel `width` wide: lines of at
/// most what fits, at most [`SUMMARY_LINES`].
pub fn summary_lines(s: &Summary, width: f32) -> Vec<String> {
    // The terminal's font is a little wider than the UI's.
    let per = if s.code { 8.0 } else { 7.6 };
    let cols = (((width - 2. * 16.) / per).floor() as usize).max(10);
    let mut lines = wrap(&s.target, cols, SUMMARY_LINES);
    if s.partial {
        if let Some(l) = lines.last_mut().filter(|l| l.as_str() != "…") {
            l.push_str(" …");
        }
    }
    lines
}

/// An open question in an extension's inbox (docs/spec/extensions.md),
/// answered from the control as from its inbox tile.
#[derive(Debug, Clone, PartialEq)]
pub struct AskItem {
    /// The extension, its name and its icon.
    pub ext: String,
    pub ext_name: String,
    pub icon: String,
    /// The connection the thread came through (an extension with several).
    pub conn: Option<String>,
    /// The thread, as the inbox knows it, and its title.
    pub thread: String,
    pub title: String,
    /// Who asks: the Pitwall agent the thread is matched to, else the name
    /// the thread gives.
    pub from: Option<String>,
    pub ask: Ask,
    /// A request on its thread is on its way.
    pub busy: bool,
    /// Why the last one on its thread didn't go.
    pub note: Option<String>,
}

impl AskItem {
    /// What it is known by in the control (its card, its height, its
    /// answer field).
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.ext, self.thread, self.ask.number)
    }

    /// Who asks, else the extension.
    pub fn who(&self) -> &str {
        self.from.as_deref().unwrap_or(&self.ext_name)
    }

    /// Its suggestions have descriptions (each a row, as in the inbox).
    pub fn described(&self) -> bool {
        self.ask.suggestions.iter().any(|s| !s.description.is_empty())
    }
}

/// One extension's inbox as the control reads it.
pub struct Inbox<'a> {
    pub ext: &'a str,
    pub name: &'a str,
    pub icon: &'a str,
    pub threads: &'a [Thread],
    pub details: &'a HashMap<String, ThreadDetail>,
    /// Threads with a request on its way, and why the last one failed.
    pub busy: &'a HashSet<String>,
    pub notes: &'a HashMap<String, String>,
    /// A thread's connection's name (an extension with several).
    pub conn: &'a dyn Fn(&str) -> Option<String>,
    /// Who a thread is from (the Pitwall agent's name, when matched).
    pub from: &'a dyn Fn(&Thread) -> Option<String>,
}

/// The threads to show questions of: those with open asks, in the inbox's
/// order (`inbox_tile::ordered`), at most [`ASK_THREADS`] of each inbox.
pub fn ask_threads(threads: &[Thread]) -> Vec<Thread> {
    let mut list = crate::main_screen::inbox_tile::ordered(threads, crate::main_screen::inbox_tile::Filter::NeedsYou, &HashSet::new());
    list.truncate(ASK_THREADS);
    list
}

/// Every open question of `inboxes` whose thread has arrived: blocking
/// ones first, then in the inboxes' order (each its threads' order, a
/// thread's asks in its messages' order).
pub fn asks(inboxes: &[Inbox]) -> Vec<AskItem> {
    let mut out = Vec::new();
    for b in inboxes {
        for th in ask_threads(b.threads) {
            let Some(d) = b.details.get(&th.id) else { continue };
            for a in d.messages.iter().flat_map(|m| &m.asks).filter(|a| a.open && a.withdrawn.is_none() && a.answer.is_none()) {
                out.push(AskItem {
                    ext: b.ext.to_string(),
                    ext_name: b.name.to_string(),
                    icon: b.icon.to_string(),
                    conn: (b.conn)(&th.id),
                    thread: th.id.clone(),
                    title: if d.title.is_empty() { th.title.clone() } else { d.title.clone() },
                    from: (b.from)(&th),
                    ask: a.clone(),
                    busy: b.busy.contains(&th.id),
                    note: b.notes.get(&th.id).cloned(),
                });
            }
        }
    }
    // Stable: the inboxes' order within each.
    out.sort_by_key(|a| !a.ask.blocking);
    out
}

/// A question card's height in a panel `width` wide, until it is drawn
/// (then it is measured): its head, its question's lines, its suggestions,
/// the answer row, and a note.
pub fn ask_height(a: &AskItem, width: f32, note: bool) -> f32 {
    // Characters of the question per line, at its size.
    let cols = ((width - 24.) / 7.5).max(10.) as usize;
    let lines = wrap(&a.ask.question, cols, 99).len().max(1) as f32;
    let options = match a.ask.suggestions.len() {
        0 => 0.,
        n if a.described() => n as f32 * (OPTION_H + 4.) + GAP,
        n => {
            let chars: usize = a.ask.suggestions.iter().map(|s| s.label.chars().count() + 6).sum();
            ((chars as f32 * 7.5 / (width - 24.)).ceil().max(1.)).min(n as f32) * (CHIP_H + 6.) + GAP
        }
    };
    GAP + CARD_HEAD + lines * QUESTION_LINE + GAP + options + ANSWER_H + if note { NOTE_H } else { 0. } + GAP
}

/// `text` up to its numbered choices ("❯ 1. Yes", "2. No", the key hints
/// under them), without the blank lines that leaves at its end. A text
/// that is nothing but choices is kept whole.
fn without_choices(text: &str) -> String {
    let first = |l: &str| {
        let l = l.trim_start().trim_start_matches(['❯', '›', '>', '●', '○', '*']).trim_start();
        l.strip_prefix("1.").or_else(|| l.strip_prefix("1)")).is_some_and(|rest| rest.starts_with(' '))
    };
    let lines: Vec<&str> = text.lines().collect();
    let Some(at) = lines.iter().position(|l| first(l)) else { return text.to_string() };
    let mut kept = &lines[..at];
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept = &kept[..kept.len() - 1];
    }
    if kept.is_empty() {
        return text.to_string();
    }
    kept.join("\n")
}

/// `text` cut into lines of at most `cols` characters, at most `max`
/// lines: a longer text keeps its first `max - 1` and ends in "…". A long
/// line continues on the next, broken after a space when one is near.
pub fn wrap(text: &str, cols: usize, max: usize) -> Vec<String> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest: Vec<char> = line.chars().collect();
        if rest.is_empty() {
            out.push(String::new());
            continue;
        }
        while rest.len() > cols {
            // The last space in the final third of the line, if any.
            let cut = (cols - cols / 3..cols).rev().find(|&i| rest[i] == ' ').map_or(cols, |i| i + 1);
            out.push(rest[..cut].iter().collect::<String>().trim_end().to_string());
            rest.drain(..cut);
            // A continuation starts at its first word.
            let lead = rest.iter().take_while(|c| **c == ' ').count();
            rest.drain(..lead);
        }
        if rest.is_empty() {
            continue;
        }
        out.push(rest.into_iter().collect());
    }
    if out.len() > max {
        out.truncate(max.saturating_sub(1));
        out.push("…".into());
    }
    out
}

/// Rows of prompt a card shows: its terminal's (`rows`) when it has the
/// screen, else its text's; [`RAW_FEW`] under a summary (`few`), else at
/// most the shape's.
pub fn lines_shown(item: &Item, shape: Shape, rows: Option<TermRows>, few: bool) -> usize {
    let max = if few { RAW_FEW } else { shape.max_lines() };
    rows.map_or_else(|| item.lines(max, shape.cols()).len(), |r| r.shown(max, shape.fill() && !few))
}

/// How an agent's card looks, for its height: the summary's lines (none:
/// no summary), the prompt's rows, and whether a line is under the buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub summary: Option<usize>,
    pub rows: usize,
    pub note: bool,
}

/// How `item`'s card looks in `shape`: `term` where its prompt is on its
/// screen (when the card has it), `all` after "Show all", `note` a line
/// under the buttons.
pub fn look(item: &Item, shape: Shape, term: Option<TermRows>, all: bool, note: bool) -> Look {
    let summary = summary(item).map(|s| summary_lines(&s, shape.width).len());
    Look { summary, rows: lines_shown(item, shape, term, summary.is_some() && !all), note }
}

/// A line under the buttons: why an answer wasn't sent, that it was, or
/// how to answer what the buttons can't.
pub fn has_note(item: &Item, error: bool, sent: bool) -> bool {
    error || sent || !item.can_accept()
}

/// The rows of an agent's screen that hold its prompt, as `(start, end)`
/// into `rows` (each row's text), at most `max` of them: from the row the
/// prompt (`ask`, as the rules cut it out of the screen) starts on to the
/// last row that draws anything. A prompt longer than `max` keeps its top;
/// one that can't be found gives the last `max` rows. With `fill`, a
/// shorter prompt has the rows above it shown too, up to `max`.
pub fn term_span(rows: &[String], ask: &str, max: usize, fill: bool) -> Option<(usize, usize)> {
    Some(TermRows::find(rows, ask)?.span(max, fill))
}

/// Where an agent's prompt is on its screen: the rows above it, and its
/// own (from the row it starts on to the last row that draws anything).
/// Kept in [`super::Float`] so the window's height follows a new maximum
/// at once, before the card is drawn again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermRows {
    pub above: usize,
    pub prompt: usize,
    /// The prompt was found (else all that is drawn counts as its rows).
    pub found: bool,
}

impl TermRows {
    /// Where the prompt is in `rows` (each row's text); `None` when
    /// nothing is drawn.
    pub fn find(rows: &[String], ask: &str) -> Option<TermRows> {
        let end = rows.iter().rposition(|r| !r.trim().is_empty())? + 1;
        let first = ask.lines().map(str::trim).find(|l| !l.is_empty());
        Some(match first.and_then(|l| rows[..end].iter().rposition(|r| r.contains(l))) {
            Some(start) => TermRows { above: start, prompt: end - start, found: true },
            None => TermRows { above: 0, prompt: end, found: false },
        })
    }

    /// How many rows a card shows of them, at most `max`; with `fill`,
    /// rows above a shorter prompt too.
    pub fn shown(self, max: usize, fill: bool) -> usize {
        let have = if fill { self.above + self.prompt } else { self.prompt };
        have.clamp(1, max.max(1))
    }

    /// Those rows, as `(start, end)` into the screen: a prompt longer than
    /// them keeps its top, else they end at the last row drawn.
    pub fn span(self, max: usize, fill: bool) -> (usize, usize) {
        let (n, end) = (self.shown(max, fill), self.above + self.prompt);
        if self.found && self.prompt > n {
            (self.above, self.above + n)
        } else {
            (end.saturating_sub(n), end)
        }
    }
}

/// An agent's card's height.
pub fn card_height(l: Look) -> f32 {
    let summary = l.summary.map_or(0., |n| SUMMARY_LABEL_H + n as f32 * SUMMARY_LINE + GAP + RAW_HEAD_H);
    let rows = if l.rows == 0 { 0. } else { l.rows as f32 * LINE_H + BOX_PAD + GAP };
    GAP + summary + rows + BUTTONS_H + if l.note { NOTE_H } else { 0. } + GAP
}

/// The window's size: the pill, or the panel (`card`: the height of the
/// card it shows) in the user's `shape`, and [`PAD`] around either.
pub fn size(card: Option<f32>, shape: Shape) -> (f32, f32) {
    let (w, h) = content_size(card, shape);
    (w + 2. * PAD, h + 2. * PAD)
}

/// What is drawn, without the clear space around it.
pub fn content_size(card: Option<f32>, shape: Shape) -> (f32, f32) {
    match card {
        None => (PILL_W, PILL_H),
        Some(card) => (shape.width, HEADER_H + RULE + card + RULE + HINT_H + BORDER),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A blocked or working Claude Code agent, with what its screen asks
    /// (the text, accept, reject).
    pub fn agent(id: &str, status: &str, ask: Option<(&str, bool, bool)>) -> AgentView {
        let mut a = crate::agents::tests::agent(id, "/work/p", status, 1, true);
        a.kind = "claude".into();
        a.kind_name = "Claude Code".into();
        a.terminal = false;
        a.status_detail = Some("Do you want to proceed?".into());
        a.ask = ask.map(|(text, accept, reject)| AskView { text: text.into(), accept, reject });
        a
    }

    fn item(text: &str, accept: bool) -> Item {
        items(&[agent("a", "blocked", Some((text, accept, accept)))]).remove(0)
    }

    const DEF: Shape = Shape { width: WIDTH, rows: None };

    /// A prompt `prompt` rows tall with `above` rows of the screen over it.
    fn at(above: usize, prompt: usize) -> TermRows {
        TermRows { above, prompt, found: true }
    }

    const BASH: &str = "Bash command\n\n  pnpm test --run src/cart\n  Run the cart unit tests\n\nDo you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again for pnpm test commands\n  3. No, and tell Claude what to do differently (esc)";
    const CODEX: &str = "Would you like to run the following command?\n\n  Reason: remove the stale build output\n\n  $ rm -rf build\n\n› 1. Yes, proceed (y)\n  2. No, and tell Codex what to do differently (esc)";

    #[test]
    fn only_agents_whose_screens_ask_are_listed() {
        let agents = vec![
            agent("api", "blocked", Some(("touch a.txt", true, true))),
            agent("web", "working", None),
            agent("docs", "blocked", Some(("Which color?", false, false))),
            // Blocked by its hooks, nothing on its screen (auto mode
            // checking a command): nothing to answer.
            agent("auto", "blocked", None),
            agent("old", "done", None),
        ];
        let list = items(&agents);
        assert_eq!(list.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["api", "docs"]);
        assert!(list[0].can_accept() && list[0].can_reject());
        assert!(!list[1].can_accept() && !list[1].can_reject());
        assert_eq!(list[0].project, "p");
    }

    #[test]
    fn another_machines_agent_is_never_answered_here() {
        let mut far = agent("far", "blocked", Some(("Bash command\n  rm -rf build", true, true)));
        far.machine.link = Some(pitwall_proto::peer::LinkView { peer: "studio".into(), state: pitwall_proto::peer::LinkState::Connected, since: 0, last_seen: Some(1) });
        let list = items(&[far]);
        assert!(!list[0].local && !list[0].can_accept() && !list[0].can_reject());
        assert!(!pill_answers(&list[0]), "not from the pill either");
        // Its prompt is still shown.
        assert_eq!(list[0].lines(9, WRAP_COLS), ["Bash command", "  rm -rf build"]);
    }

    #[test]
    fn a_prompt_with_no_key_for_no_offers_accept_only() {
        let list = items(&[agent("amp", "blocked", Some(("Approve?", true, false)))]);
        assert!(list[0].can_accept() && !list[0].can_reject());
        // "No" without "yes" is never offered.
        let list = items(&[agent("odd", "blocked", Some(("?", false, true)))]);
        assert!(!list[0].can_accept() && !list[0].can_reject());
    }

    #[test]
    fn a_yes_no_is_shown_without_its_choices() {
        let bash = "Bash command\n  touch a.txt\n\nDo you want to proceed?\n❯ 1. Yes\n  2. No\n\nEsc to cancel · Tab to amend";
        assert_eq!(item(bash, true).lines(9, WRAP_COLS), ["Bash command", "  touch a.txt", "", "Do you want to proceed?"]);
        // A question keeps its choices: they are what is asked.
        assert_eq!(item("Which color?\n❯ 1. Red\n  2. Blue", false).lines(9, WRAP_COLS), ["Which color?", "❯ 1. Red", "  2. Blue"]);
        // Nothing but choices: kept.
        assert_eq!(item("1. Yes\n2. No", true).lines(9, WRAP_COLS), ["1. Yes", "2. No"]);
        // "1.5 GB" is not a choice.
        assert_eq!(without_choices("Download\n1.5 GB?"), "Download\n1.5 GB?");
    }

    #[test]
    fn long_lines_continue_and_long_prompts_keep_their_top() {
        assert_eq!(wrap("abcdefg\n\nxy", 3, 10), ["abc", "def", "g", "", "xy"]);
        assert_eq!(wrap("a\nb\nc\nd", 80, 3), ["a", "b", "…"]);
        assert_eq!(wrap("", 80, 3), Vec::<String>::new());
        // Characters, not bytes.
        assert_eq!(wrap("❯❯❯❯", 2, 9), ["❯❯", "❯❯"]);
        // Broken after a space near the end rather than inside a word.
        assert_eq!(wrap("requires confirmation for this command", 12, 9), ["requires", "confirmation", "for this", "command"]);
        for l in wrap(&"word ".repeat(40), 12, 99) {
            assert!(l.chars().count() <= 12, "{l:?}");
        }
    }

    fn summ(label: &'static str, target: &str, code: bool, partial: bool) -> Option<Summary> {
        Some(Summary { label, target: target.into(), code, partial })
    }

    #[test]
    fn a_summary_is_lines_of_the_prompt_itself() {
        // Claude Code's command, with its description under it.
        assert_eq!(summary(&item(BASH, true)), summ("Run", "pnpm test --run src/cart", true, false));
        assert_eq!(gist(&item(BASH, true)), "Run: pnpm test --run src/cart");
        assert!(pill_answers(&item(BASH, true)));
        // A command alone.
        assert_eq!(summary(&item("Bash command\n  ls -la\nDo you want to proceed?\n❯ 1. Yes\n  2. No", true)), summ("Run", "ls -la", true, false));
        // A command on several lines: only its first is a summary, and the
        // pill doesn't answer it (the rest is only in the rows).
        let multi = item("Bash command\n\n  cat <<EOF > a.txt\n  hello\n  EOF\n\nDo you want to proceed?\n❯ 1. Yes\n  2. No", true);
        assert_eq!(summary(&multi), summ("Run", "cat <<EOF > a.txt", true, true));
        assert_eq!(gist(&multi), "Run: cat <<EOF > a.txt …");
        assert!(!pill_answers(&multi));
        let two = item("Bash command\n  cd web &&\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No", true);
        assert!(summary(&two).unwrap().partial, "a second line that isn't prose is more of the command");
        // Codex.
        assert_eq!(summary(&item(CODEX, true)), summ("Run", "rm -rf build", true, false));
        // A file.
        let edit = item("Edit file\n src/cart/total.ts\n 13 -   let sum = 0;\nDo you want to make this edit to total.ts?\n❯ 1. Yes\n  2. No", true);
        assert_eq!(summary(&edit), summ("Edit", "src/cart/total.ts", true, false));
        assert_eq!(gist(&edit), "Edit src/cart/total.ts");
        assert_eq!(summary(&item("Create file\n notes.md\n\nDo you want to create notes.md?\n❯ 1. Yes\n  2. No", true)), summ("Create", "notes.md", true, false));
        // A question: the question the rules read, when it is in the prompt.
        let mut q = item("☐ Format\n\nWhich format should the changelog use?\n\n❯ 1. Keep a Changelog\n  2. Plain list", false);
        q.detail = Some("Which format should the changelog use?".into());
        assert_eq!(summary(&q), summ("Question", "Which format should the changelog use?", false, false));
        assert!(!pill_answers(&q));
        q.detail = Some("Something else".into());
        assert_eq!(summary(&q), None, "not in the prompt, and its first line doesn't ask");
        // Anything else: no summary; the pill says what the rules read.
        let odd = item("Trust this folder?\n❯ 1. Yes\n  2. No", true);
        assert_eq!(summary(&odd), None);
        assert_eq!(gist(&odd), "Do you want to proceed?");
        assert!(!pill_answers(&odd), "nothing to go on: the pill shows it first");
    }

    #[test]
    fn a_long_target_wraps_and_says_so() {
        let s = Summary { label: "Run", target: format!("pnpm {}", "--flag ".repeat(40)), code: true, partial: false };
        let lines = summary_lines(&s, WIDTH);
        assert_eq!(lines.len(), SUMMARY_LINES);
        assert_eq!(lines.last().map(String::as_str), Some("…"));
        let s = Summary { label: "Run", target: "cat <<EOF > a.txt".into(), code: true, partial: true };
        assert_eq!(summary_lines(&s, WIDTH), ["cat <<EOF > a.txt …"]);
    }

    #[test]
    fn the_oldest_waits_first_and_the_cards_go_round() {
        let list = items(&[agent("a", "blocked", Some((BASH, true, true))), agent("b", "blocked", Some((BASH, true, true))), agent("c", "blocked", Some(("Which?", false, false)))]);
        let since = HashMap::from([("a".to_string(), 3), ("b".to_string(), 1)]);
        let list = oldest_first(list, &since);
        assert_eq!(list.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["b", "a", "c"], "one not seen yet is newest");
        let e = entries(&list, &[]);
        assert_eq!(position(&e, Some("a")), 1);
        assert_eq!(position(&e, Some("gone")), 0);
        assert_eq!((step(3, 0, -1), step(3, 2, 1), step(0, 0, 1)), (2, 0, 0));
        let answered = |k: &str| k == "b" || k == "a";
        assert_eq!(next_waiting(&e, 0, &answered), 2);
        assert_eq!(next_waiting(&e, 0, &|_| true), 0);
        // Accept all only while every one is a yes/no.
        assert!(!accept_all(&e, &|_| false), "c is a question");
        assert!(accept_all(&e[..2], &|_| false));
        assert!(!accept_all(&e[..2], &answered), "all answered");
        assert!(!accept_all(&e[..1], &|_| false), "one alone");
        assert!(!clashes(&e, "a"));
    }

    #[test]
    fn the_window_is_as_large_as_what_it_shows() {
        assert_eq!(content_size(None, DEF), (PILL_W, PILL_H));
        assert_eq!(size(None, DEF), (PILL_W + 2. * PAD, PILL_H + 2. * PAD));
        let one = item(BASH, true);
        // A summary over a few rows of the terminal.
        let l = look(&one, DEF, Some(at(3, 12)), false, false);
        assert_eq!(l, Look { summary: Some(1), rows: RAW_FEW, note: false });
        let card = GAP + (SUMMARY_LABEL_H + SUMMARY_LINE + GAP + RAW_HEAD_H) + (RAW_FEW as f32 * LINE_H + BOX_PAD + GAP) + BUTTONS_H + GAP;
        assert_eq!(card_height(l), card);
        assert_eq!(content_size(Some(card), DEF), (WIDTH, HEADER_H + RULE + card + RULE + HINT_H + BORDER));
        // "Show all": every row of the prompt, up to the shape's.
        assert_eq!(look(&one, DEF, Some(at(3, 12)), true, false).rows, 12);
        assert_eq!(look(&one, DEF, Some(at(3, 40)), true, false).rows, LINES_ALONE);
        // No summary: the rows alone, all of them.
        let odd = item("Trust this folder?\n❯ 1. Yes\n  2. No", true);
        assert_eq!(look(&odd, DEF, Some(at(0, 9)), false, false), Look { summary: None, rows: 9, note: false });
        assert_eq!(card_height(look(&odd, DEF, Some(at(0, 9)), false, true)) - card_height(look(&odd, DEF, Some(at(0, 9)), false, false)), NOTE_H);
        // A note: an error, a sent answer, or what the buttons can't answer.
        assert!(!has_note(&one, false, false) && has_note(&one, true, false) && has_note(&one, false, true));
        assert!(has_note(&item("Which?", false), false, false));
    }

    #[test]
    fn the_terminal_rows_shown_are_the_prompts() {
        let rows = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let screen = rows(&[
            "⏺ I'll create the file.",
            "",
            "────────────────────",
            " Bash command",
            "",
            "   touch a.txt",
            "",
            " Do you want to proceed?",
            " ❯ 1. Yes",
            "   2. No",
            "",
            " Esc to cancel",
            "",
            "",
        ]);
        let ask = "Bash command\n  touch a.txt\nDo you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel";
        // From the prompt's first line to the last row drawn.
        assert_eq!(term_span(&screen, ask, 18, false), Some((3, 12)));
        // Too long: its top is kept.
        assert_eq!(term_span(&screen, ask, 4, false), Some((3, 7)));
        // Not found (the screen moved on): the last rows.
        assert_eq!(term_span(&screen, "Something else", 5, false), Some((7, 12)));
        // The same line twice: the lower one is the prompt.
        let twice = rows(&["Bash command", "old output", "Bash command", "  ls"]);
        assert_eq!(term_span(&twice, "Bash command\n  ls", 9, false), Some((2, 4)));
        assert_eq!(term_span(&rows(&["", " "]), ask, 9, false), None);
        assert_eq!(term_span(&[], ask, 9, false), None);

        // Made taller by the user: the rows above a short prompt fill the
        // card, up to the screen's top; a long prompt still keeps its top.
        assert_eq!(term_span(&screen, ask, 11, true), Some((1, 12)));
        assert_eq!(term_span(&screen, ask, 18, true), Some((0, 12)));
        assert_eq!(term_span(&screen, ask, 4, true), Some((3, 7)));
        let found = TermRows::find(&screen, ask).unwrap();
        assert_eq!(found, at(3, 9));
        assert_eq!((found.shown(18, false), found.shown(18, true), found.shown(4, true)), (9, 12, 4));
    }

    #[test]
    fn the_user_can_make_it_wider_and_taller() {
        let text = (0..30).map(|i| format!("line {i} {}", "word ".repeat(20))).collect::<Vec<_>>().join("\n");
        let one = item(&text, true);
        let wide = Shape { width: 900., rows: None };
        assert_eq!(content_size(Some(100.), wide).0, 900.);
        assert_eq!(wrap_cols(WIDTH), WRAP_COLS);
        assert!(one.lines(99, wide.cols()).len() < one.lines(99, DEF.cols()).len(), "fewer lines when wider");
        // The pill doesn't change.
        assert_eq!(content_size(None, wide), (PILL_W, PILL_H));
        // Taller: more rows of prompt.
        let tall = Shape { width: WIDTH, rows: Some(24) };
        assert_eq!(lines_shown(&one, tall, None, false), 24);
        assert_eq!(lines_shown(&one, tall, None, true), RAW_FEW, "under a summary, a few until Show all");
        // A short prompt on its screen: the default shows the prompt alone,
        // a size the user set shows the rows above it too.
        assert_eq!(lines_shown(&one, DEF, Some(at(30, 5)), false), 5);
        assert_eq!(lines_shown(&one, tall, Some(at(30, 5)), false), 24);
        assert_eq!(lines_shown(&one, tall, Some(at(10, 5)), false), 15, "no more than the screen has");
    }

    #[test]
    fn the_sizes_are_within_the_bounds_and_medium_is_the_default() {
        assert_eq!(Preset::Medium.size(), (None, None));
        assert_eq!(Preset::of(None, None), Some(Preset::Medium));
        assert_eq!(Preset::of(Some(461.), Some(8)), None, "a dragged size is none of them");
        for p in Preset::ALL {
            assert_eq!(Preset::of(p.size().0, p.size().1), Some(p));
            let (w, r) = p.size();
            assert!(w.is_none_or(|w| (MIN_WIDTH..=MAX_WIDTH).contains(&w)), "{p:?}");
            assert!(r.is_none_or(|r| (MIN_ROWS..=MAX_ROWS).contains(&r)), "{p:?}");
        }
        let (small, large) = (Preset::Small.size(), Preset::Large.size());
        assert!(small.0.unwrap() < WIDTH && large.0.unwrap() > WIDTH);
        assert!(small.1.unwrap() < LINES_ALONE && large.1.unwrap() > LINES_ALONE);
    }

    fn thread(id: &str, asks: u32, blocking: bool, last: &str) -> Thread {
        Thread { id: id.into(), title: format!("Thread {id}"), from: None, unread: 0, open_asks: asks, blocking, last_activity: last.into() }
    }

    fn ask(n: u32, open: bool, blocking: bool) -> Ask {
        Ask { number: n, question: format!("Question {n}?"), multiple: false, open, suggestions: vec![], answer: None, withdrawn: None, blocking }
    }

    fn detail(id: &str, asks: Vec<Ask>) -> ThreadDetail {
        use pitwall_core::extensions::protocol::{Author, AuthorKind, Message};
        let msg = Message { id: "m".into(), author: Author { kind: AuthorKind::Agent, name: "a".into() }, time: String::new(), text: String::new(), detail: String::new(), asks, answers: vec![], withdrawals: vec![] };
        ThreadDetail { id: id.into(), title: String::new(), from: None, unread: 0, messages: vec![msg], earlier: false, can_reply: true }
    }

    #[test]
    fn open_questions_are_listed_blocking_first() {
        let threads = vec![thread("old", 1, false, "2026-01-01"), thread("new", 2, false, "2026-01-03"), thread("block", 1, true, "2026-01-02"), thread("done", 0, false, "2026-01-04")];
        let mut details = HashMap::new();
        details.insert("old".to_string(), detail("old", vec![ask(1, true, false)]));
        // An answered one and a withdrawn one are left out.
        let mut withdrawn = ask(3, true, false);
        withdrawn.withdrawn = Some("not needed".into());
        details.insert("new".to_string(), detail("new", vec![ask(1, false, false), ask(2, true, false), withdrawn]));
        details.insert("block".to_string(), detail("block", vec![ask(1, true, true)]));
        let (busy, notes) = (HashSet::from(["new".to_string()]), HashMap::from([("old".to_string(), "Refused.".to_string())]));
        let conn = |id: &str| (id == "old").then(|| "Work".to_string());
        let from = |t: &Thread| Some(format!("agent of {}", t.id));
        let list = asks(&[Inbox { ext: "x", name: "X", icon: "inbox", threads: &threads, details: &details, busy: &busy, notes: &notes, conn: &conn, from: &from }]);
        let got: Vec<(&str, u32)> = list.iter().map(|a| (a.thread.as_str(), a.ask.number)).collect();
        assert_eq!(got, [("block", 1), ("new", 2), ("old", 1)]);
        assert!(list[1].busy && !list[0].busy);
        assert_eq!((list[2].conn.as_deref(), list[2].note.as_deref(), list[2].who()), (Some("Work"), Some("Refused."), "agent of old"));
        assert_eq!(list[0].title, "Thread block", "the list's title when the thread has none");
        // After the agents, as cards of their own.
        let one = items(&[agent("api", "blocked", Some((BASH, true, true)))]);
        let e = entries(&one, &list);
        assert_eq!(e.iter().map(|e| e.name()).collect::<Vec<_>>(), ["api", "agent of block", "agent of new", "agent of old"]);
        assert_eq!(e[1].key(), "x/block/1");
        // A thread that hasn't arrived has no card yet.
        details.remove("block");
        let list = asks(&[Inbox { ext: "x", name: "X", icon: "inbox", threads: &threads, details: &details, busy: &busy, notes: &notes, conn: &conn, from: &from }]);
        assert_eq!(list.len(), 2);
        // At most so many threads are asked for, the blocking ones first.
        let many: Vec<Thread> = (0..10).map(|i| thread(&format!("t{i}"), 1, i == 7, &format!("2026-01-0{}", i % 9))).collect();
        let picked = ask_threads(&many);
        assert_eq!(picked.len(), ASK_THREADS);
        assert_eq!(picked[0].id, "t7");
    }
}
