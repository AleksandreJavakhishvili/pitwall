//! What the floating control shows, as plain data: the agents whose
//! screens ask the user something, each one's prompt as lines, the open
//! questions in extensions' inboxes, and how large the window is for them
//! (it has no scrolling: it is sized to its content).

use std::collections::{HashMap, HashSet};

use pitwall_core::extensions::protocol::{Ask, Thread, ThreadDetail};
use pitwall_proto::{AgentView, AskView, Status};

/// Clear space around what is drawn, inside the window: room for its
/// shadow and for the count that sits on the mark's corner.
pub const PAD: f32 = 14.;
/// Collapsed: a tile with the Pitwall mark, the count on its corner.
pub const PUCK: f32 = 46.;
/// The panel's width with the cards open, until the user drags its edge
/// (then [`Shape::width`], between [`MIN_WIDTH`] and [`MAX_WIDTH`]).
pub const WIDTH: f32 = 600.;
pub const MIN_WIDTH: f32 = 420.;
pub const MAX_WIDTH: f32 = 1100.;
/// The header: the mark, the count, "Accept all", and the toggle.
pub const HEADER_H: f32 = 44.;
/// A card's first row: the agent and what it wants.
pub const CARD_HEAD: f32 = 26.;
/// The prompt's font size (JetBrains Mono), and the height of one of its
/// lines: a terminal row's at that size (`wall::paint::cell_height`,
/// checked by a test).
pub const TERM_FONT: f32 = 10.5;
pub const LINE_H: f32 = 16.;
/// The prompt box's own padding, top and bottom together.
pub const BOX_PAD: f32 = 16.;
/// The buttons under the prompt.
pub const BUTTONS_H: f32 = 30.;
/// A line of help or of an error under the buttons.
pub const NOTE_H: f32 = 18.;
/// Space above a card's first row, around its prompt box, and below its buttons.
pub const GAP: f32 = 8.;
/// The "+N more" row.
pub const MORE_H: f32 = 28.;
/// The panel's border, top and bottom together.
pub const BORDER: f32 = 2.;
/// The hairline above each card and above the "+N more" row.
pub const RULE: f32 = 1.;

/// Cards shown (agents first, then questions); the rest are counted
/// ("+2 more").
pub const MAX_CARDS: usize = 3;
/// Threads with open questions whose messages the control asks their
/// extension for (the first, in the inbox's order), so it can show them.
pub const ASK_THREADS: usize = 6;
/// A question card's parts, for its height before it is measured (the
/// drawn card says its own: [`super::Float`] keeps it).
pub const QUESTION_LINE: f32 = 19.;
pub const OPTION_H: f32 = 30.;
pub const CHIP_H: f32 = 28.;
pub const ANSWER_H: f32 = 30.;
/// Characters of the prompt per line: what fits [`WIDTH`] in its font
/// (a wider or narrower panel scales it, [`wrap_cols`]).
pub const WRAP_COLS: usize = 86;
/// Rows of the prompt shown when one agent asks, and when several do; a
/// longer prompt keeps its top. The user can drag that between
/// [`MIN_ROWS`] and [`MAX_ROWS`] (rows when one asks; see [`max_lines`]).
pub const LINES_ALONE: usize = 16;
pub const LINES_SHARED: usize = 7;
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
    /// Rows of prompt a card shows when `count` agents ask.
    pub fn max_lines(&self, count: usize) -> usize {
        max_lines(count, self.rows.unwrap_or(LINES_ALONE))
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
                kind: a.kind.clone(),
                kind_name: a.kind_name.clone(),
                detail: a.status_detail.clone(),
                ask: a.ask.clone()?,
                local: !crate::agents::mirrored(a),
            })
        })
        .collect()
}

/// The agents' cards shown: the first [`MAX_CARDS`].
pub fn shown(items: &[Item]) -> &[Item] {
    &items[..items.len().min(MAX_CARDS)]
}

/// The questions' cards shown: what the agents' leave of [`MAX_CARDS`].
pub fn shown_asks(items: &[Item], asks: &[AskItem]) -> usize {
    asks.len().min(MAX_CARDS - shown(items).len())
}

/// The shown agents that Accept can be typed into.
pub fn acceptable(items: &[Item]) -> Vec<&Item> {
    shown(items).iter().filter(|i| i.can_accept()).collect()
}

/// The header's text.
pub fn title(items: &[Item], asks: &[AskItem]) -> String {
    let questions = |n: usize| if n == 1 { "1 question".to_string() } else { format!("{n} questions") };
    match (items, asks) {
        ([], []) => String::new(),
        ([one], []) => format!("{} needs you", one.name),
        (many, []) => format!("{} agents need you", many.len()),
        ([], [one]) => format!("{} asks you", one.who()),
        ([], many) => format!("{} for you", questions(many.len())),
        ([one], asks) => format!("{} and {}", one.name, questions(asks.len())),
        (many, asks) => format!("{} agents and {}", many.len(), questions(asks.len())),
    }
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

/// Lines of prompt a card shows when `count` agents ask, `alone` when one
/// does: several share in the default's proportion ([`LINES_SHARED`] of
/// [`LINES_ALONE`]), at least [`MIN_ROWS`].
pub fn max_lines(count: usize, alone: usize) -> usize {
    if count <= 1 {
        alone
    } else {
        ((alone * LINES_SHARED) as f32 / LINES_ALONE as f32).round().max(MIN_ROWS as f32) as usize
    }
}

/// The rows when one agent asks that give each card `per_card` rows when
/// `count` do (the other way round of [`max_lines`]).
pub fn alone_for(count: usize, per_card: usize) -> usize {
    if count <= 1 {
        per_card
    } else {
        ((per_card * LINES_ALONE) as f32 / LINES_SHARED as f32).round() as usize
    }
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

/// Rows of prompt a card shows when `count` agents ask: its terminal's
/// (`rows`) when it has the screen, else its text's, at most the shape's.
pub fn lines_shown(item: &Item, count: usize, shape: Shape, rows: Option<TermRows>) -> usize {
    let max = shape.max_lines(count);
    rows.map_or_else(|| item.lines(max, shape.cols()).len(), |r| r.shown(max, shape.fill()))
}

/// Whether a card has a line under its buttons: an error, or why there
/// is nothing to press but Open.
pub fn has_note(item: &Item, error: bool) -> bool {
    error || !item.can_accept()
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

/// A card's height with `lines` rows of prompt.
pub fn card_height(lines: usize, note: bool) -> f32 {
    let text = if lines == 0 { 0. } else { lines as f32 * LINE_H + BOX_PAD + GAP };
    GAP + CARD_HEAD + text + BUTTONS_H + if note { NOTE_H } else { 0. } + GAP
}

/// The window's size: the mark alone, or the cards under the header (in
/// the user's `shape`), and [`PAD`] around either.
/// `asks`: each question's card's height, in order. `error`: whether an
/// agent's last answer failed. `rows`: where the prompt is on its
/// terminal, when a card has the agent's screen (else its prompt's text
/// is shown, wrapped).
pub fn size(items: &[Item], asks: &[f32], expanded: bool, shape: Shape, error: &dyn Fn(&str) -> bool, rows: &dyn Fn(&str) -> Option<TermRows>) -> (f32, f32) {
    let (w, h) = content_size(items, asks, expanded, shape, error, rows);
    (w + 2. * PAD, h + 2. * PAD)
}

/// What is drawn, without the clear space around it.
pub fn content_size(items: &[Item], asks: &[f32], expanded: bool, shape: Shape, error: &dyn Fn(&str) -> bool, rows: &dyn Fn(&str) -> Option<TermRows>) -> (f32, f32) {
    let count = items.len() + asks.len();
    if !expanded || count == 0 {
        return (PUCK, PUCK);
    }
    let cards: f32 = shown(items).iter().map(|i| RULE + card_height(lines_shown(i, count, shape, rows(&i.id)), has_note(i, error(&i.id)))).sum();
    let questions: f32 = asks[..asks.len().min(MAX_CARDS - shown(items).len())].iter().map(|h| RULE + h).sum();
    let more = if count > MAX_CARDS { RULE + MORE_H } else { 0. };
    (shape.width, HEADER_H + cards + questions + more + BORDER)
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

    fn no_error(_: &str) -> bool {
        false
    }

    fn no_rows(_: &str) -> Option<TermRows> {
        None
    }

    const DEF: Shape = Shape { width: WIDTH, rows: None };

    /// A prompt `prompt` rows tall with `above` rows of the screen over it.
    fn at(above: usize, prompt: usize) -> TermRows {
        TermRows { above, prompt, found: true }
    }

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
        assert_eq!(title(&list, &[]), "2 agents need you");
        assert_eq!(title(&list[..1], &[]), "api needs you");
        assert_eq!(title(&[], &[]), "");
        assert_eq!(acceptable(&list).iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["api"]);
    }

    #[test]
    fn another_machines_agent_is_never_answered_here() {
        let mut far = agent("far", "blocked", Some(("rm -rf build", true, true)));
        far.machine.link = Some(pitwall_proto::peer::LinkView { peer: "studio".into(), state: pitwall_proto::peer::LinkState::Connected, since: 0, last_seen: Some(1) });
        let list = items(&[far]);
        assert!(!list[0].local && !list[0].can_accept() && !list[0].can_reject());
        // Its prompt is still shown.
        assert_eq!(list[0].lines(9, WRAP_COLS), ["rm -rf build"]);
        assert!(acceptable(&list).is_empty());
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
        let list = items(&[agent("api", "blocked", Some((bash, true, true)))]);
        assert_eq!(list[0].lines(9, WRAP_COLS), ["Bash command", "  touch a.txt", "", "Do you want to proceed?"]);
        // A question keeps its choices: they are what is asked.
        let q = "Which color?\n❯ 1. Red\n  2. Blue";
        let list = items(&[agent("docs", "blocked", Some((q, false, false)))]);
        assert_eq!(list[0].lines(9, WRAP_COLS), ["Which color?", "❯ 1. Red", "  2. Blue"]);
        // Nothing but choices: kept.
        let list = items(&[agent("x", "blocked", Some(("1. Yes\n2. No", true, true)))]);
        assert_eq!(list[0].lines(9, WRAP_COLS), ["1. Yes", "2. No"]);
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

    #[test]
    fn the_window_is_as_large_as_what_it_shows() {
        let one = items(&[agent("api", "blocked", Some(("touch a.txt\nDo you want to proceed?", true, true)))]);
        assert_eq!(content_size(&one, &[], false, DEF, &no_error, &no_rows), (PUCK, PUCK));
        assert_eq!(content_size(&[], &[], true, DEF, &no_error, &no_rows), (PUCK, PUCK));
        assert_eq!(size(&one, &[], false, DEF, &no_error, &no_rows), (PUCK + 2. * PAD, PUCK + 2. * PAD));
        let card = GAP + CARD_HEAD + (2. * LINE_H + BOX_PAD + GAP) + BUTTONS_H + GAP;
        assert_eq!(content_size(&one, &[], true, DEF, &no_error, &no_rows), (WIDTH, HEADER_H + RULE + card + BORDER));
        assert_eq!(content_size(&one, &[], true, DEF, &|_| true, &no_rows).1, HEADER_H + RULE + card + NOTE_H + BORDER, "an error takes a line");

        // A question: a line says to open the agent.
        let q = items(&[agent("docs", "blocked", Some(("Which?", false, false)))]);
        assert_eq!(card_height(lines_shown(&q[0], 1, DEF, None), has_note(&q[0], false)), GAP + CARD_HEAD + (LINE_H + BOX_PAD + GAP) + BUTTONS_H + NOTE_H + GAP);

        // Several: every card at once, each with a shorter prompt.
        let long = (0..30).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let alone = items(&[agent("a", "blocked", Some((&long, true, true)))]);
        assert_eq!(alone[0].lines(max_lines(1, LINES_ALONE), WRAP_COLS).len(), LINES_ALONE);
        let many: Vec<AgentView> = (0..5).map(|i| agent(&format!("a{i}"), "blocked", Some((&long, true, true)))).collect();
        let many = items(&many);
        assert_eq!(shown(&many).len(), MAX_CARDS);
        let card = card_height(lines_shown(&many[0], many.len(), DEF, None), false);
        assert_eq!(card, GAP + CARD_HEAD + (LINES_SHARED as f32 * LINE_H + BOX_PAD + GAP) + BUTTONS_H + GAP);
        assert_eq!(content_size(&many, &[], true, DEF, &no_error, &no_rows), (WIDTH, HEADER_H + 3. * (RULE + card) + RULE + MORE_H + BORDER));

        // A card that has its agent's screen is as tall as the rows it shows.
        let with = content_size(&one, &[], true, DEF, &no_error, &|_| Some(at(10, 6)));
        assert_eq!(with.1, HEADER_H + RULE + card_height(6, false) + BORDER);
        assert_eq!(content_size(&one, &[], true, DEF, &no_error, &|_| Some(at(0, 99))).1, HEADER_H + RULE + card_height(LINES_ALONE, false) + BORDER);
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
        assert_eq!(term_span(&screen, "Something else", 5, true), Some((7, 12)));
        let found = TermRows::find(&screen, ask).unwrap();
        assert_eq!(found, at(3, 9));
        assert_eq!((found.shown(18, false), found.shown(18, true), found.shown(4, true)), (9, 12, 4));
    }

    #[test]
    fn the_user_can_make_it_wider_and_taller() {
        let text = (0..30).map(|i| format!("line {i} {}", "word ".repeat(20))).collect::<Vec<_>>().join("\n");
        let one = items(&[agent("api", "blocked", Some((&text, true, true)))]);
        // Wider: its width, and the prompt's text wraps at what fits it.
        let wide = Shape { width: 900., rows: None };
        assert_eq!(content_size(&one, &[], true, wide, &no_error, &no_rows).0, 900.);
        assert_eq!(size(&one, &[], true, wide, &no_error, &no_rows).0, 900. + 2. * PAD);
        assert_eq!(wrap_cols(WIDTH), WRAP_COLS);
        assert_eq!(wrap_cols(900.), 129);
        assert_eq!(wrap_cols(MIN_WIDTH), 60);
        assert!(one[0].lines(99, wide.cols()).iter().all(|l| l.chars().count() <= 129));
        assert!(one[0].lines(99, wide.cols()).len() < one[0].lines(99, DEF.cols()).len(), "fewer lines when wider");
        // The mark alone doesn't change.
        assert_eq!(content_size(&one, &[], false, wide, &no_error, &no_rows), (PUCK, PUCK));

        // Taller: more rows of prompt, of text or of terminal.
        let tall = Shape { width: WIDTH, rows: Some(24) };
        assert_eq!(lines_shown(&one[0], 1, tall, None), 24);
        let h = |s: Shape, rows: Option<TermRows>| content_size(&one, &[], true, s, &no_error, &|_| rows).1;
        assert_eq!(h(tall, None) - h(DEF, None), 8. * LINE_H, "8 rows more");
        // A short prompt on its screen: the default shows the prompt alone,
        // a size the user set shows the rows above it too.
        assert_eq!(h(DEF, Some(at(30, 5))), HEADER_H + RULE + card_height(5, false) + BORDER);
        assert_eq!(h(tall, Some(at(30, 5))), HEADER_H + RULE + card_height(24, false) + BORDER);
        assert_eq!(h(tall, Some(at(10, 5))), HEADER_H + RULE + card_height(15, false) + BORDER, "no more than the screen has");

        // Several share in the same proportion; the default is unchanged.
        assert_eq!((max_lines(1, LINES_ALONE), max_lines(3, LINES_ALONE)), (LINES_ALONE, LINES_SHARED));
        assert_eq!(max_lines(3, 32), 14);
        assert_eq!(max_lines(3, MIN_ROWS), MIN_ROWS, "never fewer than the least");
        // Each card's rows back to the rows when one asks, and again.
        for per in MIN_ROWS..=17 {
            assert_eq!(max_lines(3, alone_for(3, per)), per, "{per}");
            assert_eq!(alone_for(1, per), per);
        }
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

    #[test]
    fn the_title_counts_agents_and_questions() {
        let one = items(&[agent("api", "blocked", Some(("?", true, true)))]);
        let two = items(&[agent("api", "blocked", Some(("?", true, true))), agent("web", "blocked", Some(("?", true, true)))]);
        let q = |from: Option<&str>| AskItem {
            ext: "x".into(),
            ext_name: "Inbox".into(),
            icon: "inbox".into(),
            conn: None,
            thread: "t".into(),
            title: "T".into(),
            from: from.map(str::to_string),
            ask: ask(1, true, false),
            busy: false,
            note: None,
        };
        assert_eq!(title(&[], &[q(Some("docs"))]), "docs asks you");
        assert_eq!(title(&[], &[q(None)]), "Inbox asks you");
        assert_eq!(title(&[], &[q(None), q(None)]), "2 questions for you");
        assert_eq!(title(&one, &[q(None)]), "api and 1 question");
        assert_eq!(title(&two, &[q(None), q(None)]), "2 agents and 2 questions");
        // The cards: agents first, questions in what is left of three.
        assert_eq!(shown_asks(&one, &[q(None), q(None), q(None)]), 2);
        assert_eq!(shown_asks(&two, &[q(None)]), 1);
        let h = |n: usize| content_size(&one, &vec![100.; n], true, DEF, &no_error, &no_rows).1;
        assert_eq!(h(1) - h(0), RULE + 100.);
        assert_eq!(h(3) - h(2), RULE + MORE_H, "the third question is counted, not shown");
    }
}
