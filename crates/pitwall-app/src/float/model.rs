//! What the floating control shows, as plain data: the agents whose
//! screens ask the user something, each one's prompt as lines, and how
//! large the window is for them (it has no scrolling: it is sized to its
//! content).

use pitwall_proto::{AgentView, AskView, Status};

/// Clear space around what is drawn, inside the window: room for its
/// shadow and for the count that sits on the mark's corner.
pub const PAD: f32 = 14.;
/// Collapsed: a tile with the Pitwall mark, the count on its corner.
pub const PUCK: f32 = 46.;
/// The window's width with the cards open.
pub const WIDTH: f32 = 600.;
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

/// Agents shown as cards; the rest are counted ("+2 more").
pub const MAX_CARDS: usize = 3;
/// Characters of the prompt per line: what fits [`WIDTH`] in its font.
pub const WRAP_COLS: usize = 86;
/// Rows of the prompt shown when one agent asks, and when several do; a
/// longer prompt keeps its top.
pub const LINES_ALONE: usize = 16;
pub const LINES_SHARED: usize = 7;

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

    /// The prompt's lines as shown, at most `max`. A yes/no loses its
    /// list of choices: the buttons under it are those.
    pub fn lines(&self, max: usize) -> Vec<String> {
        let text = if self.ask.accept { without_choices(&self.ask.text) } else { self.ask.text.clone() };
        wrap(&text, WRAP_COLS, max)
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

/// The cards shown: the first [`MAX_CARDS`].
pub fn shown(items: &[Item]) -> &[Item] {
    &items[..items.len().min(MAX_CARDS)]
}

/// The shown agents that Accept can be typed into.
pub fn acceptable(items: &[Item]) -> Vec<&Item> {
    shown(items).iter().filter(|i| i.can_accept()).collect()
}

/// The header's text.
pub fn title(items: &[Item]) -> String {
    match items {
        [] => String::new(),
        [one] => format!("{} needs you", one.name),
        many => format!("{} agents need you", many.len()),
    }
}

/// Lines of prompt a card shows when `count` agents ask.
pub fn max_lines(count: usize) -> usize {
    if count <= 1 {
        LINES_ALONE
    } else {
        LINES_SHARED
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

/// Rows of prompt a card shows: its terminal's (`rows`) when it has the
/// screen, else its text's, at most `max`.
pub fn lines_shown(item: &Item, max: usize, rows: Option<usize>) -> usize {
    rows.map_or_else(|| item.lines(max).len(), |r| r.clamp(1, max))
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
/// one that can't be found gives the last `max` rows.
pub fn term_span(rows: &[String], ask: &str, max: usize) -> Option<(usize, usize)> {
    let end = rows.iter().rposition(|r| !r.trim().is_empty())? + 1;
    let first = ask.lines().map(str::trim).find(|l| !l.is_empty());
    let start = first
        .and_then(|l| rows[..end].iter().rposition(|r| r.contains(l)))
        .unwrap_or_else(|| end.saturating_sub(max));
    Some((start, end.min(start + max.max(1))))
}

/// A card's height with `lines` rows of prompt.
pub fn card_height(lines: usize, note: bool) -> f32 {
    let text = if lines == 0 { 0. } else { lines as f32 * LINE_H + BOX_PAD + GAP };
    GAP + CARD_HEAD + text + BUTTONS_H + if note { NOTE_H } else { 0. } + GAP
}

/// The window's size: the mark alone, or the cards under the header,
/// and [`PAD`] around either.
/// `error`: whether an agent's last answer failed. `rows`: the rows of its
/// terminal a card shows, when it has the agent's screen (else its
/// prompt's text is shown, wrapped).
pub fn size(items: &[Item], expanded: bool, error: &dyn Fn(&str) -> bool, rows: &dyn Fn(&str) -> Option<usize>) -> (f32, f32) {
    let (w, h) = content_size(items, expanded, error, rows);
    (w + 2. * PAD, h + 2. * PAD)
}

/// What is drawn, without the clear space around it.
pub fn content_size(items: &[Item], expanded: bool, error: &dyn Fn(&str) -> bool, rows: &dyn Fn(&str) -> Option<usize>) -> (f32, f32) {
    if !expanded || items.is_empty() {
        return (PUCK, PUCK);
    }
    let max = max_lines(items.len());
    let cards: f32 = shown(items).iter().map(|i| RULE + card_height(lines_shown(i, max, rows(&i.id)), has_note(i, error(&i.id)))).sum();
    let more = if items.len() > MAX_CARDS { RULE + MORE_H } else { 0. };
    (WIDTH, HEADER_H + cards + more + BORDER)
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

    fn no_rows(_: &str) -> Option<usize> {
        None
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
        assert_eq!(title(&list), "2 agents need you");
        assert_eq!(title(&list[..1]), "api needs you");
        assert_eq!(title(&[]), "");
        assert_eq!(acceptable(&list).iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["api"]);
    }

    #[test]
    fn another_machines_agent_is_never_answered_here() {
        let mut far = agent("far", "blocked", Some(("rm -rf build", true, true)));
        far.machine.link = Some(pitwall_proto::peer::LinkView { peer: "studio".into(), state: pitwall_proto::peer::LinkState::Connected, since: 0, last_seen: Some(1) });
        let list = items(&[far]);
        assert!(!list[0].local && !list[0].can_accept() && !list[0].can_reject());
        // Its prompt is still shown.
        assert_eq!(list[0].lines(9), ["rm -rf build"]);
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
        assert_eq!(list[0].lines(9), ["Bash command", "  touch a.txt", "", "Do you want to proceed?"]);
        // A question keeps its choices: they are what is asked.
        let q = "Which color?\n❯ 1. Red\n  2. Blue";
        let list = items(&[agent("docs", "blocked", Some((q, false, false)))]);
        assert_eq!(list[0].lines(9), ["Which color?", "❯ 1. Red", "  2. Blue"]);
        // Nothing but choices: kept.
        let list = items(&[agent("x", "blocked", Some(("1. Yes\n2. No", true, true)))]);
        assert_eq!(list[0].lines(9), ["1. Yes", "2. No"]);
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
        assert_eq!(content_size(&one, false, &no_error, &no_rows), (PUCK, PUCK));
        assert_eq!(content_size(&[], true, &no_error, &no_rows), (PUCK, PUCK));
        assert_eq!(size(&one, false, &no_error, &no_rows), (PUCK + 2. * PAD, PUCK + 2. * PAD));
        let card = GAP + CARD_HEAD + (2. * LINE_H + BOX_PAD + GAP) + BUTTONS_H + GAP;
        assert_eq!(content_size(&one, true, &no_error, &no_rows), (WIDTH, HEADER_H + RULE + card + BORDER));
        assert_eq!(content_size(&one, true, &|_| true, &no_rows).1, HEADER_H + RULE + card + NOTE_H + BORDER, "an error takes a line");

        // A question: a line says to open the agent.
        let q = items(&[agent("docs", "blocked", Some(("Which?", false, false)))]);
        assert_eq!(card_height(lines_shown(&q[0], 9, None), has_note(&q[0], false)), GAP + CARD_HEAD + (LINE_H + BOX_PAD + GAP) + BUTTONS_H + NOTE_H + GAP);

        // Several: every card at once, each with a shorter prompt.
        let long = (0..30).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let alone = items(&[agent("a", "blocked", Some((&long, true, true)))]);
        assert_eq!(alone[0].lines(max_lines(1)).len(), LINES_ALONE);
        let many: Vec<AgentView> = (0..5).map(|i| agent(&format!("a{i}"), "blocked", Some((&long, true, true)))).collect();
        let many = items(&many);
        assert_eq!(shown(&many).len(), MAX_CARDS);
        let card = card_height(lines_shown(&many[0], LINES_SHARED, None), false);
        assert_eq!(card, GAP + CARD_HEAD + (LINES_SHARED as f32 * LINE_H + BOX_PAD + GAP) + BUTTONS_H + GAP);
        assert_eq!(content_size(&many, true, &no_error, &no_rows), (WIDTH, HEADER_H + 3. * (RULE + card) + RULE + MORE_H + BORDER));

        // A card that has its agent's screen is as tall as the rows it shows.
        let with = content_size(&one, true, &no_error, &|_| Some(6));
        assert_eq!(with.1, HEADER_H + RULE + card_height(6, false) + BORDER);
        assert_eq!(content_size(&one, true, &no_error, &|_| Some(99)).1, HEADER_H + RULE + card_height(LINES_ALONE, false) + BORDER);
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
        assert_eq!(term_span(&screen, ask, 18), Some((3, 12)));
        // Too long: its top is kept.
        assert_eq!(term_span(&screen, ask, 4), Some((3, 7)));
        // Not found (the screen moved on): the last rows.
        assert_eq!(term_span(&screen, "Something else", 5), Some((7, 12)));
        // The same line twice: the lower one is the prompt.
        let twice = rows(&["Bash command", "old output", "Bash command", "  ls"]);
        assert_eq!(term_span(&twice, "Bash command\n  ls", 9), Some((2, 4)));
        assert_eq!(term_span(&rows(&["", " "]), ask, 9), None);
        assert_eq!(term_span(&[], ask, 9), None);
    }
}
