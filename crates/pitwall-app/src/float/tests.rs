use std::sync::Mutex;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use pitwall_proto::AgentView;

use super::model::tests::agent;
use super::*;

/// What the control was asked to type and to open.
#[derive(Default)]
struct Calls {
    answers: Mutex<Vec<(String, bool, String)>>,
    typed: Mutex<Vec<(String, Vec<u8>)>>,
    opened: Mutex<Vec<String>>,
    inboxes: Mutex<Vec<(String, String)>>,
}

/// The edge across from the end the window is anchored at (its top on
/// macOS), and the corner across from where it is anchored.
const END: &str = if GROWS_UP { "float-resize-top" } else { "float-resize-bottom" };
const CORNER: &str = if GROWS_UP { "float-resize-top-left" } else { "float-resize-bottom-left" };

/// The control over a store of `agents`, the user away from Pitwall.
/// `fail`: what typing an answer fails with.
fn setup(agents: Vec<AgentView>, fail: Option<&'static str>, cx: &mut TestAppContext) -> (Entity<AgentStore>, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let store = cx.new(|_| AgentStore::new(agents));
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        cx.set_global(InFront(false));
        let (a, o, k, i) = (calls.clone(), calls.clone(), calls.clone(), calls.clone());
        init(
            &store,
            Arc::new(move |id, accept, seen| {
                a.answers.lock().unwrap().push((id.into(), accept, seen.into()));
                fail.map_or(Ok(()), |e| Err(e.into()))
            }),
            Arc::new(move |id, keys| {
                k.typed.lock().unwrap().push((id.into(), keys.to_vec()));
                Ok(())
            }),
            Arc::new(move |id, _| o.opened.lock().unwrap().push(id.into())),
            Arc::new(move |ext, thread, _| i.inboxes.lock().unwrap().push((ext.into(), thread.into()))),
            cx,
        );
    });
    cx.run_until_parked();
    (store, calls)
}

fn set_agents(store: &Entity<AgentStore>, agents: Vec<AgentView>, cx: &mut TestAppContext) {
    store.update(cx, |s, cx| {
        s.agents = agents;
        cx.notify();
    });
    cx.run_until_parked();
}

fn up(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| cx.global::<Float>().window.is_some())
}

fn dims(cx: &mut TestAppContext) -> (f32, f32) {
    cx.update(|cx| cx.global::<Float>().size)
}

/// The control's window, for clicking in it, at the size the control
/// asked for (a test window records a resize without applying it).
fn window(cx: &mut TestAppContext) -> VisualTestContext {
    let (handle, (w, h)) = cx.update(|cx| {
        let f = cx.global::<Float>();
        (f.window.expect("the control is up"), f.size)
    });
    let v = VisualTestContext::from_window(handle.into(), cx);
    v.simulate_resize(size(px(w), px(h)));
    v.run_until_parked();
    v
}

fn click(v: &mut VisualTestContext, what: &'static str) {
    let b = v.debug_bounds(what).unwrap_or_else(|| panic!("{what} is drawn"));
    v.simulate_click(b.center(), Modifiers::default());
    v.run_until_parked();
    // The click may have changed the control's size.
    let (w, h) = v.update(|_, cx| cx.global::<Float>().size);
    v.simulate_resize(size(px(w), px(h)));
    v.run_until_parked();
}

const BASH: &str = "Bash command\n  touch a.txt\nDo you want to proceed?\n❯ 1. Yes\n  2. No\nEsc to cancel";

#[gpui::test]
fn it_is_up_only_while_a_screen_asks_the_user_who_is_elsewhere(cx: &mut TestAppContext) {
    let (store, _) = setup(vec![agent("api", "working", None)], None, cx);
    assert!(!up(cx), "nobody needs the user");
    cx.update(|cx| assert!(cx.global::<Float>().watch.is_none(), "nothing is watched while no agent asks"));

    // Blocked by its hooks with nothing on its screen (Claude Code's auto
    // mode checking a command): nothing to answer, so nothing shows.
    set_agents(&store, vec![agent("api", "blocked", None)], cx);
    assert!(!up(cx));

    set_agents(&store, vec![agent("api", "blocked", Some((BASH, true, true)))], cx);
    assert!(up(cx));
    assert_eq!(dims(cx), (model::PUCK + 2. * model::PAD, model::PUCK + 2. * model::PAD), "the mark alone at first");

    // The user comes back to Pitwall: gone within a watch interval.
    cx.update(|cx| cx.set_global(InFront(true)));
    cx.executor().advance_clock(WATCH * 2);
    cx.run_until_parked();
    assert!(!up(cx));
    // And leaves again while it still waits.
    cx.update(|cx| cx.set_global(InFront(false)));
    cx.executor().advance_clock(WATCH * 2);
    cx.run_until_parked();
    assert!(up(cx));

    // Answered in the terminal: nothing left to show.
    set_agents(&store, vec![agent("api", "working", None)], cx);
    assert!(!up(cx));
    cx.update(|cx| assert!(cx.global::<Float>().watch.is_none()));
}

#[gpui::test]
fn the_setting_turns_it_off_and_on(cx: &mut TestAppContext) {
    let (_store, _) = setup(vec![agent("api", "blocked", Some((BASH, true, true)))], None, cx);
    assert!(up(cx));
    let def = pitwall_proto::settings::find(KEY).expect("the setting");
    assert_eq!(def.default_value(), serde_json::json!(true), "on unless turned off");
    let set = |on: bool, cx: &mut TestAppContext| {
        cx.update(|cx| crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::json!(on))));
        cx.run_until_parked();
    };
    set(false, cx);
    assert!(!up(cx));
    cx.update(|cx| assert!(!enabled(cx) && cx.global::<Float>().watch.is_none()));
    set(true, cx);
    assert!(up(cx));
}

#[gpui::test]
fn every_asking_agent_has_its_own_card_and_buttons(cx: &mut TestAppContext) {
    let (store, calls) = setup(
        vec![agent("api", "blocked", Some((BASH, true, true))), agent("web", "blocked", Some(("rm -rf build?", true, true)))],
        None,
        cx,
    );
    let mut v = window(cx);
    // The mark alone at first; a click opens both cards at once.
    assert!(v.debug_bounds("float-accept-api").is_none());
    click(&mut v, "float-puck");
    for what in ["float-prompt-api", "float-accept-api", "float-reject-api", "float-prompt-web", "float-accept-web", "float-reject-web"] {
        assert!(v.debug_bounds(what).is_some(), "{what}");
    }
    click(&mut v, "float-accept-api");
    assert_eq!(*calls.answers.lock().unwrap(), [("api".to_string(), true, BASH.to_string())]);
    // A second click while the agent hasn't moved on types nothing more.
    click(&mut v, "float-accept-api");
    assert_eq!(calls.answers.lock().unwrap().len(), 1);

    // The other agent, rejected, without selecting anything first.
    click(&mut v, "float-reject-web");
    assert_eq!(calls.answers.lock().unwrap()[1], ("web".to_string(), false, "rm -rf build?".to_string()));

    // "api" asks something new: it can be answered again.
    set_agents(
        &store,
        vec![agent("api", "blocked", Some(("touch b.txt?", true, true))), agent("web", "blocked", Some(("rm -rf build?", true, true)))],
        cx,
    );
    let mut v = window(cx);
    click(&mut v, "float-accept-api");
    assert_eq!(calls.answers.lock().unwrap()[2], ("api".to_string(), true, "touch b.txt?".to_string()));

    // The header's button closes the cards again.
    click(&mut v, "float-collapse");
    assert_eq!(dims(cx), (model::PUCK + 2. * model::PAD, model::PUCK + 2. * model::PAD));
    let mut v = window(cx);
    assert!(v.debug_bounds("float-puck").is_some() && v.debug_bounds("float-accept-api").is_none());
}

#[gpui::test]
fn accept_all_answers_every_yes_or_no_once(cx: &mut TestAppContext) {
    let (_store, calls) = setup(
        vec![
            agent("api", "blocked", Some((BASH, true, true))),
            agent("docs", "blocked", Some(("Which color?\n❯ 1. Red\n  2. Blue", false, false))),
            agent("web", "blocked", Some(("rm -rf build?", true, true))),
        ],
        None,
        cx,
    );
    let mut v = window(cx);
    click(&mut v, "float-puck");
    click(&mut v, "float-accept-all");
    let ids = |calls: &Calls| calls.answers.lock().unwrap().iter().map(|(id, yes, _)| (id.clone(), *yes)).collect::<Vec<_>>();
    assert_eq!(ids(&calls), [("api".to_string(), true), ("web".to_string(), true)], "the question is left alone");
    click(&mut v, "float-accept-all");
    assert_eq!(ids(&calls).len(), 2, "nothing is typed twice");
}

#[gpui::test]
fn one_yes_or_no_has_no_accept_all(cx: &mut TestAppContext) {
    let (_store, _) = setup(vec![agent("api", "blocked", Some((BASH, true, true)))], None, cx);
    let mut v = window(cx);
    click(&mut v, "float-puck");
    assert!(v.debug_bounds("float-accept-api").is_some() && v.debug_bounds("float-accept-all").is_none());
}

#[gpui::test]
fn a_prompt_that_is_not_a_yes_or_no_only_opens(cx: &mut TestAppContext) {
    let (_store, calls) = setup(vec![agent("docs", "blocked", Some(("Which color?\n❯ 1. Red\n  2. Blue", false, false)))], None, cx);
    let mut v = window(cx);
    click(&mut v, "float-puck");
    assert!(v.debug_bounds("float-prompt-docs").is_some(), "its prompt is shown");
    assert!(v.debug_bounds("float-accept-docs").is_none() && v.debug_bounds("float-reject-docs").is_none());
    assert!(v.debug_bounds("float-note-docs").is_some());
    // Nothing is typed even if asked to.
    cx.update(|cx| answer("docs", true, cx));
    assert!(calls.answers.lock().unwrap().is_empty());
    let mut v = window(cx);
    click(&mut v, "float-open-docs");
    assert_eq!(*calls.opened.lock().unwrap(), ["docs"]);
}

#[gpui::test]
fn an_answer_that_was_not_typed_says_why(cx: &mut TestAppContext) {
    let (store, _) = setup(vec![agent("api", "blocked", Some((BASH, true, true)))], Some("its prompt changed"), cx);
    let mut v = window(cx);
    click(&mut v, "float-puck");
    let before = dims(cx).1;
    let mut v = window(cx);
    click(&mut v, "float-accept-api");
    cx.update(|cx| {
        let f = cx.global::<Float>();
        assert_eq!(f.errors.get("api").map(|e| e.to_string()), Some("Not sent: its prompt changed.".to_string()));
        assert!(f.answered.is_empty(), "it can be tried again");
        assert_eq!(f.size.1, before + model::NOTE_H, "the window makes room for the line");
    });
    let mut v = window(cx);
    assert!(v.debug_bounds("float-note-api").is_some());
    // The agent moved on: the error goes with its prompt.
    set_agents(&store, vec![agent("api", "working", None)], cx);
    cx.update(|cx| assert!(cx.global::<Float>().errors.is_empty()));
}

#[gpui::test]
fn it_opens_and_resizes_from_where_it_was_dragged(cx: &mut TestAppContext) {
    let (_store, _) = setup(vec![agent("api", "blocked", Some((BASH, true, true)))], None, cx);
    cx.update(|cx| {
        cx.global_mut::<Float>().anchor = Some(point(px(900.), px(500.)));
        let b = bounds((80., 80.), cx).unwrap();
        // The corner that stays put is the right one, at the dragged spot.
        assert_eq!(b.origin.x + b.size.width, px(900.));
        let open = bounds((model::WIDTH, 300.), cx).unwrap();
        assert_eq!(open.origin.x + open.size.width, px(900.));
        if GROWS_UP {
            assert_eq!((b.origin.y + b.size.height, open.origin.y + open.size.height), (px(500.), px(500.)));
        } else {
            assert_eq!((b.origin.y, open.origin.y), (px(500.), px(500.)));
        }
    });
}

#[gpui::test]
fn everything_drawn_fits_the_window(cx: &mut TestAppContext) {
    let long = (0..40).map(|i| format!("line {i} {}", "x".repeat(90))).collect::<Vec<_>>().join("\n");
    let mut agents = vec![agent("api", "blocked", Some((&long, true, true)))];
    agents.extend((0..6).map(|i| agent(&format!("a{i}"), "blocked", Some(("Which?", false, false)))));
    let (_store, _) = setup(agents, None, cx);
    let mut v = window(cx);
    let puck = v.debug_bounds("float-puck").unwrap();
    assert_eq!((puck.size.width, puck.size.height), (px(model::PUCK), px(model::PUCK)));
    assert_eq!((puck.origin.x, puck.origin.y), (px(model::PAD), px(model::PAD)));
    click(&mut v, "float-puck");
    let (w, h) = dims(cx);
    let list = cx.update(|cx| items(cx));
    assert_eq!((w, h), model::size(&list, &[], true, model::Shape::default(), &|_| false, &|_| None));
    assert_eq!(w, model::WIDTH + 2. * model::PAD);
    let mut v = window(cx);
    for what in [
        "float-header", "float-collapse", "float-card-api", "float-prompt-api", "float-accept-api", "float-reject-api",
        "float-open-api", "float-card-a0", "float-note-a0", "float-open-a0", "float-card-a1", "float-open-a1", "float-more",
    ] {
        let b = v.debug_bounds(what).unwrap_or_else(|| panic!("{what} is drawn"));
        assert!(b.origin.y >= px(0.) && b.origin.y + b.size.height <= px(h), "{what} inside 0..{h}: {b:?}");
        assert!(b.origin.x >= px(0.) && b.origin.x + b.size.width <= px(w), "{what} inside the width: {b:?}");
    }
    assert!(v.debug_bounds("float-card-a2").is_none(), "only the first {MAX_CARDS} have a card");
    let prompt = v.debug_bounds("float-prompt-api").unwrap();
    assert_eq!(prompt.size.height, px(model::LINES_SHARED as f32 * LINE_H + BOX_PAD));
    // Each card is exactly as tall as the window was sized for.
    let card = v.debug_bounds("float-card-api").unwrap();
    assert_eq!(card.size.height, px(model::card_height(model::LINES_SHARED, false)));
}

/// A screen source that sends each watcher `frame` at once.
struct Fake {
    frame: pitwall_proto::ScreenFrame,
    watching: Mutex<Vec<String>>,
}

impl ScreenSource for Fake {
    fn watch(&self, agent_id: &str, mut sink: pitwall_core::term::FrameSink) -> Result<u64, String> {
        if agent_id == "far" {
            return Err("not here".into());
        }
        sink(&self.frame);
        self.watching.lock().unwrap().push(agent_id.into());
        Ok(1)
    }
    fn unwatch(&self, agent_id: &str, _: u64) {
        self.watching.lock().unwrap().retain(|a| a != agent_id);
    }
}

#[gpui::test]
fn a_card_shows_the_rows_of_the_terminal_that_ask(cx: &mut TestAppContext) {
    use pitwall_proto::{ScreenFrame, ScreenLine, ScreenRun};
    // A screen with output above the prompt and blank rows under it.
    let rows = ["⏺ I'll create it.", "", "────────", " Bash command", "", "   touch a.txt", "", " Do you want to proceed?", " ❯ 1. Yes", "   2. No", "", " Esc to cancel", "", ""];
    let frame = ScreenFrame {
        cols: 120,
        rows: rows.len() as u16,
        cursor: None,
        full: true,
        lines: rows.iter().enumerate().map(|(i, r)| ScreenLine(i as u16, vec![ScreenRun(r.to_string(), 0, 0, 0)])).collect(),
    };
    let fake = Arc::new(Fake { frame, watching: Mutex::new(Vec::new()) });
    cx.update(|cx| cx.set_global(Screens(fake.clone())));
    let (store, calls) = setup(vec![agent("api", "blocked", Some((BASH, true, true)))], None, cx);
    let mut v = window(cx);
    assert!(fake.watching.lock().unwrap().is_empty(), "no screen is watched for the mark alone");
    click(&mut v, "float-puck");
    let mut v = window(cx);
    assert_eq!(*fake.watching.lock().unwrap(), ["api"]);
    // From "Bash command" to "Esc to cancel": 9 rows, and the window is sized for them.
    let term = v.debug_bounds("float-term-api").expect("the terminal is drawn");
    assert_eq!(term.size.height, px(9. * LINE_H + BOX_PAD));
    assert!(v.debug_bounds("float-prompt-api").is_none(), "not the text as well");
    assert_eq!(dims(cx).1, model::HEADER_H + RULE + model::card_height(9, false) + model::BORDER + 2. * model::PAD);
    // A row of the terminal is as tall as the window was sized for.
    let row = v.update(|window, _| crate::wall::paint::row_height(model::TERM_FONT, window));
    assert!(row <= LINE_H && row > LINE_H - 1., "a terminal row is {row} px, the card allows {LINE_H}");
    let accept = v.debug_bounds("float-accept-api").unwrap();
    assert!(accept.origin.y + accept.size.height <= px(dims(cx).1));

    // Opened: keys go to the first card's agent, with no click before.
    v.simulate_keystrokes("down enter 2 escape");
    let typed = calls.typed.lock().unwrap().clone();
    assert_eq!(typed.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["api"; 4]);
    assert_eq!(typed.iter().map(|(_, k)| k.as_slice()).collect::<Vec<_>>(), [b"\x1b[B".as_slice(), b"\r", b"2", b"\x1b"]);

    // Closed again, or gone: nothing is watched any more.
    click(&mut v, "float-collapse");
    let _ = window(cx);
    assert!(fake.watching.lock().unwrap().is_empty());
    // Closed, keys go nowhere; opened again, to the agent once more.
    window(cx).simulate_keystrokes("down");
    assert_eq!(calls.typed.lock().unwrap().len(), 4);
    click(&mut window(cx), "float-puck");
    assert_eq!(*fake.watching.lock().unwrap(), ["api"]);
    window(cx).simulate_keystrokes("enter");
    assert_eq!(calls.typed.lock().unwrap().len(), 5);
    set_agents(&store, vec![agent("api", "working", None)], cx);
    assert!(!up(cx) && fake.watching.lock().unwrap().is_empty());
}

/// Drag what `what` draws (a resize edge) by `dx`, `dy` as the pointer
/// would: press, move in a few steps with the button held, release.
fn drag(v: &mut VisualTestContext, what: &'static str, dx: f32, dy: f32) {
    let b = v.debug_bounds(what).unwrap_or_else(|| panic!("{what} is drawn"));
    let at = b.center();
    v.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    for k in [0.25, 0.5, 1.] {
        v.simulate_mouse_move(at + point(px(dx * k), px(dy * k)), MouseButton::Left, Modifiers::default());
        v.run_until_parked();
    }
    v.simulate_mouse_up(at + point(px(dx), px(dy)), MouseButton::Left, Modifiers::default());
    v.run_until_parked();
    let (w, h) = v.update(|_, cx| cx.global::<Float>().size);
    v.simulate_resize(size(px(w), px(h)));
    v.run_until_parked();
}

fn double_click(v: &mut VisualTestContext, what: &'static str) {
    let at = v.debug_bounds(what).unwrap_or_else(|| panic!("{what} is drawn")).center();
    for count in [1, 2] {
        v.simulate_event(gpui::MouseDownEvent { position: at, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: count, first_mouse: false });
        v.simulate_event(gpui::MouseUpEvent { position: at, modifiers: Modifiers::default(), button: MouseButton::Left, click_count: count });
        v.run_until_parked();
    }
    let (w, h) = v.update(|_, cx| cx.global::<Float>().size);
    v.simulate_resize(size(px(w), px(h)));
    v.run_until_parked();
}

fn user_size(cx: &mut TestAppContext) -> (Option<f32>, Option<usize>) {
    cx.update(|cx| {
        let f = cx.global::<Float>();
        (f.width, f.rows)
    })
}

/// A prompt longer than any card shows.
fn long_prompt() -> String {
    (0..60).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")
}

#[gpui::test]
fn its_edges_resize_the_open_panel(cx: &mut TestAppContext) {
    let long = long_prompt();
    let (_store, _) = setup(vec![agent("api", "blocked", Some((&long, true, true)))], None, cx);
    let mut v = window(cx);
    assert!(v.debug_bounds("float-resize-left").is_none(), "the mark alone doesn't resize");
    click(&mut v, "float-puck");
    let (w0, h0) = dims(cx);
    assert_eq!(w0, model::WIDTH + 2. * model::PAD);
    let mut v = window(cx);
    // The edges straddle the panel's: in the clear space and over it.
    let side = v.debug_bounds("float-resize-left").unwrap();
    assert!(side.origin.x < px(model::PAD) && side.origin.x + side.size.width > px(model::PAD));

    // Its left edge, 150 to the left: that much wider, as tall.
    drag(&mut v, "float-resize-left", -150., 0.);
    assert_eq!(user_size(cx), (Some(model::WIDTH + 150.), None));
    assert_eq!(dims(cx), (w0 + 150., h0));
    assert!(cx.update(|cx| cx.global::<Float>().resize.is_none()), "over at the release");
    assert!(PRESS.get().is_none() && !DRAGGED.get(), "a press on an edge doesn't move the window");
    let mut v = window(cx);
    let header = v.debug_bounds("float-header").unwrap();
    assert_eq!(header.size.width, px(model::WIDTH + 150. - model::BORDER), "the panel fills the new width");

    // The edge across from the anchored end, 4 rows further out: 4 rows
    // more (up on macOS, where it is anchored at the bottom).
    let out = if GROWS_UP { -1. } else { 1. };
    drag(&mut v, END, 0., out * 4. * LINE_H);
    assert_eq!(user_size(cx), (Some(model::WIDTH + 150.), Some(model::LINES_ALONE + 4)));
    assert_eq!(dims(cx), (w0 + 150., h0 + 4. * LINE_H));
    let mut v = window(cx);
    let prompt = v.debug_bounds("float-prompt-api").unwrap();
    assert_eq!(prompt.size.height, px((model::LINES_ALONE + 4) as f32 * LINE_H + BOX_PAD));

    // The corner: both at once.
    drag(&mut v, CORNER, -50., out * 2. * LINE_H);
    assert_eq!(user_size(cx), (Some(model::WIDTH + 200.), Some(model::LINES_ALONE + 6)));
    let resized = dims(cx);
    assert_eq!(resized, (w0 + 200., h0 + 6. * LINE_H));

    // Closed to the mark and opened again: the same size.
    let mut v = window(cx);
    click(&mut v, "float-collapse");
    assert_eq!(dims(cx), (model::PUCK + 2. * model::PAD, model::PUCK + 2. * model::PAD), "the mark doesn't change");
    click(&mut window(cx), "float-puck");
    assert_eq!(dims(cx), resized);

    // A double click on an edge: back to the default size.
    let mut v = window(cx);
    double_click(&mut v, "float-resize-left");
    assert_eq!(user_size(cx), (None, None));
    assert_eq!(dims(cx), (w0, h0));
}

#[gpui::test]
fn a_resize_stays_within_its_bounds(cx: &mut TestAppContext) {
    let long = long_prompt();
    let (_store, _) = setup(vec![agent("api", "blocked", Some((&long, true, true)))], None, cx);
    click(&mut window(cx), "float-puck");
    let out = if GROWS_UP { -1. } else { 1. };
    let mut v = window(cx);
    drag(&mut v, "float-resize-left", 2000., 0.);
    assert_eq!(user_size(cx).0, Some(model::MIN_WIDTH));
    let mut v = window(cx);
    drag(&mut v, "float-resize-left", -5000., 0.);
    let widest = user_size(cx).0.unwrap();
    assert!(widest <= model::MAX_WIDTH && widest > model::WIDTH, "{widest}");
    let mut v = window(cx);
    drag(&mut v, END, 0., -out * 100. * LINE_H);
    assert_eq!(user_size(cx).1, Some(model::MIN_ROWS));
    let mut v = window(cx);
    drag(&mut v, END, 0., out * 100. * LINE_H);
    let rows = user_size(cx).1.unwrap();
    assert!(rows <= model::MAX_ROWS && rows > model::LINES_ALONE, "{rows}");
    // And the window still fits the screen.
    let area = cx.update(|cx| cx.primary_display().unwrap().visible_bounds());
    assert!(dims(cx).1 <= f32::from(area.size.height));
}

#[gpui::test]
fn several_cards_share_a_height_drag(cx: &mut TestAppContext) {
    let long = long_prompt();
    let agents = (0..3).map(|i| agent(&format!("a{i}"), "blocked", Some((long.as_str(), true, true)))).collect();
    let (_store, _) = setup(agents, None, cx);
    click(&mut window(cx), "float-puck");
    let h0 = dims(cx).1;
    let out = if GROWS_UP { -1. } else { 1. };
    // Three cards, 6 rows of travel: 2 rows more each.
    let mut v = window(cx);
    drag(&mut v, END, 0., out * 6. * LINE_H);
    assert_eq!(dims(cx).1, h0 + 6. * LINE_H);
    assert_eq!(cx.update(|cx| shape(cx.global::<Float>()).max_lines(3)), model::LINES_SHARED + 2);
}

#[gpui::test]
fn a_taller_card_shows_the_terminal_above_a_short_prompt(cx: &mut TestAppContext) {
    use pitwall_proto::{ScreenFrame, ScreenLine, ScreenRun};
    // 3 rows of output over a 9-row prompt.
    let rows = ["⏺ I'll create it.", "", "────────", " Bash command", "", "   touch a.txt", "", " Do you want to proceed?", " ❯ 1. Yes", "   2. No", "", " Esc to cancel", "", ""];
    let frame = ScreenFrame {
        cols: 120,
        rows: rows.len() as u16,
        cursor: None,
        full: true,
        lines: rows.iter().enumerate().map(|(i, r)| ScreenLine(i as u16, vec![ScreenRun(r.to_string(), 0, 0, 0)])).collect(),
    };
    cx.update(|cx| cx.set_global(Screens(Arc::new(Fake { frame, watching: Mutex::new(Vec::new()) }))));
    let (_store, _) = setup(vec![agent("api", "blocked", Some((BASH, true, true)))], None, cx);
    click(&mut window(cx), "float-puck");
    let mut v = window(cx);
    let h0 = dims(cx).1;
    assert_eq!(v.debug_bounds("float-term-api").unwrap().size.height, px(9. * LINE_H + BOX_PAD), "the default: the prompt alone");
    // One row taller than the default's 16: the 3 rows above the prompt
    // come in, and no more (the screen has none).
    let out = if GROWS_UP { -1. } else { 1. };
    drag(&mut v, END, 0., out * LINE_H);
    let mut v = window(cx);
    assert_eq!(v.debug_bounds("float-term-api").unwrap().size.height, px(12. * LINE_H + BOX_PAD));
    assert_eq!(dims(cx).1, h0 + 3. * LINE_H, "the window is sized for them");
}

#[gpui::test]
fn the_size_is_kept_with_the_place(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("pitwall-float-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = place_file(&dir);
    // A file from before the size was kept: its place alone.
    std::fs::write(&file, r#"{"x":900.0,"y":500.0}"#).unwrap();
    assert_eq!(load_place(&file), Some(Place { anchor: Some(point(px(900.), px(500.))), width: None, rows: None }));

    let long = long_prompt();
    let (_store, _) = setup(vec![agent("api", "blocked", Some((&long, true, true)))], None, cx);
    cx.update(|cx| cx.global_mut::<Float>().file = Some(file.clone()));
    click(&mut window(cx), "float-puck");
    let mut v = window(cx);
    drag(&mut v, CORNER, -100., if GROWS_UP { -3. } else { 3. } * LINE_H);
    let kept = load_place(&file).unwrap();
    assert_eq!((kept.width, kept.rows), (Some(model::WIDTH + 100.), Some(model::LINES_ALONE + 3)));
    // Reset: the size goes, the place stays.
    double_click(&mut window(cx), CORNER);
    let kept = load_place(&file).unwrap();
    assert_eq!((kept.width, kept.rows), (None, None));
    assert_eq!(kept.anchor.is_some(), cx.update(|cx| cx.global::<Float>().anchor.is_some()));
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(&dir);
}

#[gpui::test]
fn every_edge_and_corner_resizes_it(cx: &mut TestAppContext) {
    let long = long_prompt();
    let (_store, _) = setup(vec![agent("api", "blocked", Some((&long, true, true)))], None, cx);
    // Anchored where the window fits: its bottom right on macOS, its top
    // right elsewhere (the window hangs down from there).
    let y0 = if GROWS_UP { 900. } else { 100. };
    cx.update(|cx| cx.global_mut::<Float>().anchor = Some(point(px(1500.), px(y0))));
    click(&mut window(cx), "float-puck");
    let mut v = window(cx);
    for e in Edge::ALL {
        assert!(v.debug_bounds(e.name()).is_some(), "{} is drawn", e.name());
    }
    let (w0, h0) = dims(cx);
    let anchor = |cx: &mut TestAppContext| cx.update(|cx| cx.global::<Float>().anchor.unwrap());

    // Its right edge, 100 to the right: wider, and the right side (where
    // it is anchored) goes with the pointer; its left stays.
    let moved = || crate::platform::FLOAT_SHIFT.with(|s| s.replace((0., 0.)));
    moved();
    drag(&mut v, "float-resize-right", 100., 0.);
    assert_eq!(user_size(cx), (Some(model::WIDTH + 100.), None));
    assert_eq!(dims(cx), (w0 + 100., h0));
    assert_eq!(anchor(cx), point(px(1600.), px(y0)));
    // On macOS the window is told to move its corner as the drag goes;
    // elsewhere it opens again at the new place on release.
    assert_eq!(moved(), if GROWS_UP { (100., 0.) } else { (0., 0.) });

    // The end it is anchored at (its bottom on macOS), 3 rows out: taller,
    // and that end goes with it.
    let anchored = if GROWS_UP { "float-resize-bottom" } else { "float-resize-top" };
    let out = if GROWS_UP { 1. } else { -1. };
    let mut v = window(cx);
    drag(&mut v, anchored, 0., out * 3. * LINE_H);
    assert_eq!(user_size(cx).1, Some(model::LINES_ALONE + 3));
    assert_eq!(dims(cx), (w0 + 100., h0 + 3. * LINE_H));
    assert_eq!(anchor(cx), point(px(1600.), px(y0 + out * 3. * LINE_H)));

    // The corner where both meet: both at once.
    let corner = if GROWS_UP { "float-resize-bottom-right" } else { "float-resize-top-right" };
    let mut v = window(cx);
    drag(&mut v, corner, 50., out * LINE_H);
    assert_eq!(user_size(cx), (Some(model::WIDTH + 150.), Some(model::LINES_ALONE + 4)));
    assert_eq!(anchor(cx), point(px(1650.), px(y0 + out * 4. * LINE_H)));

    // The other two corners: the width with the edge they are on, the rows
    // with the end they are at.
    let free = if GROWS_UP { "float-resize-top-right" } else { "float-resize-bottom-right" };
    let mut v = window(cx);
    drag(&mut v, free, -150., -out * LINE_H);
    assert_eq!(user_size(cx), (Some(model::WIDTH), Some(model::LINES_ALONE + 5)));
    let other = if GROWS_UP { "float-resize-bottom-left" } else { "float-resize-top-left" };
    let mut v = window(cx);
    drag(&mut v, other, 20., -out * LINE_H);
    assert_eq!(user_size(cx), (Some(model::WIDTH - 20.), Some(model::LINES_ALONE + 4)));

    // A double click on any of them: the default size.
    double_click(&mut window(cx), "float-resize-right");
    assert_eq!(user_size(cx), (None, None));
    assert_eq!(dims(cx).0, w0);
}

#[gpui::test]
fn without_room_a_drag_outward_keeps_the_size(cx: &mut TestAppContext) {
    let long = long_prompt();
    let (_store, _) = setup(vec![agent("api", "blocked", Some((&long, true, true)))], None, cx);
    // Placed so that it already reaches past the screen's left edge and
    // past the end it grows to (the top on macOS, the bottom elsewhere).
    let y = if GROWS_UP { 100. } else { 1000. };
    cx.update(|cx| cx.global_mut::<Float>().anchor = Some(point(px(300.), px(y))));
    click(&mut window(cx), "float-puck");
    let start = dims(cx);
    // Outward: no room, nothing smaller.
    let out = if GROWS_UP { -1. } else { 1. };
    let mut v = window(cx);
    drag(&mut v, END, 0., out * 2. * LINE_H);
    assert_eq!(user_size(cx).1, Some(model::LINES_ALONE));
    let mut v = window(cx);
    drag(&mut v, "float-resize-left", -100., 0.);
    assert_eq!(user_size(cx).0, None, "the width it had");
    assert_eq!(dims(cx), start);
    // Inward: smaller, as ever.
    let mut v = window(cx);
    drag(&mut v, END, 0., -out * 2. * LINE_H);
    assert_eq!(user_size(cx).1, Some(model::LINES_ALONE - 2));
    let mut v = window(cx);
    drag(&mut v, "float-resize-left", 50., 0.);
    assert_eq!(user_size(cx).0, Some(model::WIDTH - 50.));
}

#[gpui::test]
fn the_header_offers_three_sizes_kept_with_the_place(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("pitwall-float-presets-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = place_file(&dir);
    let _ = std::fs::remove_file(&file);
    let long = long_prompt();
    let (_store, _) = setup(vec![agent("api", "blocked", Some((&long, true, true)))], None, cx);
    cx.update(|cx| cx.global_mut::<Float>().file = Some(file.clone()));
    click(&mut window(cx), "float-puck");
    let medium = dims(cx);
    let preset = |cx: &mut TestAppContext| cx.update(|cx| model::Preset::of(cx.global::<Float>().width, cx.global::<Float>().rows));
    assert_eq!(preset(cx), Some(model::Preset::Medium), "the default is Medium");

    click(&mut window(cx), "float-size-small");
    assert_eq!(user_size(cx), model::Preset::Small.size());
    assert_eq!(preset(cx), Some(model::Preset::Small));
    let small = dims(cx);
    assert!(small.0 < medium.0 && small.1 < medium.1, "{small:?} < {medium:?}");
    let kept = load_place(&file).unwrap();
    assert_eq!((kept.width, kept.rows), model::Preset::Small.size(), "kept for the next start");

    click(&mut window(cx), "float-size-large");
    let large = dims(cx);
    assert_eq!(user_size(cx), model::Preset::Large.size());
    assert!(large.0 > medium.0 && large.1 > medium.1, "{large:?} > {medium:?}");

    // Minimised to the mark and opened again: the same size.
    click(&mut window(cx), "float-collapse");
    assert_eq!(dims(cx), (model::PUCK + 2. * model::PAD, model::PUCK + 2. * model::PAD));
    click(&mut window(cx), "float-puck");
    assert_eq!(dims(cx), large);

    // A drag: none of them any more, and the size dragged to is kept.
    let mut v = window(cx);
    drag(&mut v, "float-resize-left", -10., 0.);
    assert_eq!(preset(cx), None);
    click(&mut window(cx), "float-size-medium");
    assert_eq!((user_size(cx), dims(cx)), ((None, None), medium));
    let kept = load_place(&file).unwrap();
    assert_eq!((kept.width, kept.rows), (None, None));
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(&dir);
}

// ── questions from extensions' inboxes ─────────────────────────────────────

use crate::extensions::hub::{self, Hub};
use pitwall_core::extensions::{Adapter, Found, Manifest, MatchAgents};
use serde_json::json;

fn inbox_manifest() -> Manifest {
    Manifest {
        adapter: Some(Adapter { command: vec!["node".into(), "a.mjs".into()] }),
        match_agents: Some(MatchAgents { agent: "{project}".into(), session: "{agent_id}".into() }),
        icon: Some("inbox".into()),
        ..Manifest::named("demo", "Demo inbox")
    }
}

/// The control as [`setup`], with an extension that has an inbox, turned
/// on (`on`), its adapter's other end driven by the test.
fn setup_inbox(agents: Vec<AgentView>, on: bool, cx: &mut TestAppContext) -> (Entity<AgentStore>, Arc<Calls>, Entity<Hub>) {
    let h = cx.update(|cx| {
        crate::kit::init(cx);
        crate::extensions::set_found(vec![Found { folder: "demo".into(), manifest: Ok(inbox_manifest()) }], cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", on));
        hub::hub(cx)
    });
    cx.run_until_parked();
    let (store, calls) = setup(agents, None, cx);
    (store, calls, h)
}

fn say(h: &Entity<Hub>, line: serde_json::Value, cx: &mut TestAppContext) {
    h.read_with(cx, |h, _| h.ends["demo"].say(line));
    cx.run_until_parked();
}

fn said(h: &Entity<Hub>, cx: &mut TestAppContext) -> Vec<serde_json::Value> {
    h.read_with(cx, |h, _| h.ends.get("demo").map(|e| e.sent()).unwrap_or_default())
}

/// The adapter connects with two threads: one from agent "api" with a
/// blocking question, one with nothing open.
fn connect(h: &Entity<Hub>, cx: &mut TestAppContext) {
    let _ = said(h, cx);
    say(h, json!({"type": "hello", "protocol": 1}), cx);
    say(h, json!({"type": "status", "state": "ready"}), cx);
    say(
        h,
        json!({"type": "threads", "threads": [
            {"id": "t1", "title": "Which database?", "from": {"agent": "p", "session": "api"}, "unread": 1, "openAsks": 1, "blocking": true, "lastActivity": "2026-01-02T10:00:00Z"},
            {"id": "t2", "title": "Done", "from": {"agent": "p", "session": "web"}, "lastActivity": "2026-01-03T10:00:00Z"}
        ]}),
        cx,
    );
}

fn thread_t1(open: bool) -> serde_json::Value {
    json!({"type": "thread", "thread": {"id": "t1", "title": "Which database?", "from": {"agent": "p", "session": "api"}, "canReply": true, "messages": [
        {"id": "m1", "author": {"kind": "agent", "name": "api"}, "time": "2026-01-02T10:00:00Z", "text": "I need to pick one.",
         "asks": [{"number": 1, "question": "Postgres or SQLite?", "open": open, "blocking": true, "suggestions": [{"label": "Postgres", "description": "Shared"}, {"label": "SQLite", "description": "One file"}]}]}
    ]}})
}

fn answers(lines: &[serde_json::Value]) -> Vec<serde_json::Value> {
    lines.iter().filter(|l| l["type"] == "answer").cloned().collect()
}

#[gpui::test]
fn an_inbox_question_is_a_card_answered_with_a_click(cx: &mut TestAppContext) {
    let (_store, calls, h) = setup_inbox(vec![agent("api", "working", None)], true, cx);
    assert!(!up(cx), "nothing asks yet");
    connect(&h, cx);
    // The thread with an open question is asked for, to show it.
    assert_eq!(said(&h, cx), [json!({"type": "open", "thread": "t1"})]);
    assert!(!up(cx), "not before its question has arrived");
    say(&h, thread_t1(true), cx);
    assert!(up(cx), "a question needs the user: the mark is up");
    let mut v = window(cx);
    click(&mut v, "float-puck");
    let mut v = window(cx);
    for what in ["float-ask-t1-1", "float-pick-0-t1-1", "float-pick-1-t1-1", "float-own-t1-1", "float-send-t1-1", "float-decline-t1-1", "float-open-t1-1"] {
        assert!(v.debug_bounds(what).is_some(), "{what}");
    }
    let q = cx.update(|cx| asks(cx));
    assert_eq!((q.len(), q[0].who(), q[0].title.as_str(), q[0].ext_name.as_str()), (1, "api", "Which database?", "Demo inbox"));
    assert_eq!(cx.update(|cx| model::title(&items(cx), &asks(cx))), "api asks you");
    // The window is as tall as the card was drawn.
    let card = v.debug_bounds("float-ask-t1-1").unwrap();
    let (_, h_win) = dims(cx);
    assert_eq!(h_win, model::HEADER_H + RULE + f32::from(card.size.height) + model::BORDER + 2. * model::PAD);

    // A suggestion, clicked: sent as the inbox sends it.
    let mut v = window(cx);
    click(&mut v, "float-pick-1-t1-1");
    let sent = answers(&said(&h, cx));
    assert_eq!(sent.len(), 1);
    assert_eq!((&sent[0]["thread"], &sent[0]["ask"], &sent[0]["picks"]), (&json!("t1"), &json!(1), &json!([1])));
    assert!(window(cx).debug_bounds("float-ask-note-t1-1").is_some(), "it says it is on its way");
    // Clicked again while on its way: nothing more.
    click(&mut window(cx), "float-pick-0-t1-1");
    assert!(answers(&said(&h, cx)).is_empty());

    // Done: the question is answered in its thread, and nothing asks.
    say(&h, json!({"type": "result", "req": sent[0]["req"], "ok": true}), cx);
    say(&h, thread_t1(false), cx);
    assert!(cx.update(|cx| asks(cx)).is_empty());
    assert!(!up(cx));
    assert!(calls.answers.lock().unwrap().is_empty(), "nothing typed into an agent");
}

#[gpui::test]
fn an_own_answer_is_typed_in_its_field_not_into_an_agent(cx: &mut TestAppContext) {
    let (_store, calls, h) = setup_inbox(vec![agent("web", "blocked", Some((BASH, true, true)))], true, cx);
    connect(&h, cx);
    say(&h, thread_t1(true), cx);
    click(&mut window(cx), "float-puck");
    // Agents first, then questions; the title says both.
    let mut v = window(cx);
    assert!(v.debug_bounds("float-card-web").is_some() && v.debug_bounds("float-ask-t1-1").is_some());
    let card = v.debug_bounds("float-card-web").unwrap();
    assert!(card.origin.y < v.debug_bounds("float-ask-t1-1").unwrap().origin.y);
    assert_eq!(cx.update(|cx| model::title(&items(cx), &asks(cx))), "web and 1 question");

    // The field, clicked: what is typed goes there, not to the agent the
    // keyboard went to when the cards opened.
    click(&mut v, "float-own-t1-1");
    let mut v = window(cx);
    v.simulate_input("Postgres, with a pool");
    v.simulate_keystrokes("enter");
    v.run_until_parked();
    assert!(calls.typed.lock().unwrap().is_empty(), "nothing typed into the agent: {:?}", calls.typed.lock().unwrap());
    let sent = answers(&said(&h, cx));
    assert_eq!(sent.len(), 1);
    assert_eq!((&sent[0]["text"], sent[0].get("picks")), (&json!("Postgres, with a pool"), Some(&json!([]))));

    // It failed: why, on its card, and it can be answered again.
    say(&h, json!({"type": "result", "req": sent[0]["req"], "ok": false, "message": "The server refused it."}), cx);
    let mut v = window(cx);
    assert!(v.debug_bounds("float-ask-note-t1-1").is_some());
    let note = cx.update(|cx| {
        let f = cx.global::<Float>();
        asks(cx).first().and_then(|a| ask_note(f, a)).map(|n| n.0.to_string())
    });
    assert_eq!(note.as_deref(), Some("Not sent: The server refused it."));
    assert!(cx.update(|cx| cx.global::<Float>().sent.is_empty()));
}

#[gpui::test]
fn a_question_that_takes_several_is_sent_with_its_picks(cx: &mut TestAppContext) {
    let (_store, _, h) = setup_inbox(vec![], true, cx);
    connect(&h, cx);
    say(
        &h,
        json!({"type": "thread", "thread": {"id": "t1", "title": "Platforms", "canReply": true, "messages": [
            {"id": "m1", "author": {"kind": "agent", "name": "docs"}, "time": "2026-01-02T10:00:00Z", "text": "",
             "asks": [{"number": 2, "question": "Which platforms?", "multiple": true, "open": true, "suggestions": [{"label": "macOS"}, {"label": "Windows"}, {"label": "Linux"}]}]}
        ]}}),
        cx,
    );
    click(&mut window(cx), "float-puck");
    click(&mut window(cx), "float-pick-0-t1-2");
    click(&mut window(cx), "float-pick-2-t1-2");
    click(&mut window(cx), "float-pick-0-t1-2");
    click(&mut window(cx), "float-pick-1-t1-2");
    assert!(answers(&said(&h, cx)).is_empty(), "toggled, not sent");
    click(&mut window(cx), "float-send-t1-2");
    let sent = answers(&said(&h, cx));
    assert_eq!((sent.len(), &sent[0]["picks"]), (1, &json!([1, 2])));
}

#[gpui::test]
fn open_in_inbox_and_decline(cx: &mut TestAppContext) {
    let (_store, calls, h) = setup_inbox(vec![], true, cx);
    connect(&h, cx);
    say(&h, thread_t1(true), cx);
    click(&mut window(cx), "float-puck");
    click(&mut window(cx), "float-open-t1-1");
    assert_eq!(*calls.inboxes.lock().unwrap(), [("demo".to_string(), "t1".to_string())]);
    click(&mut window(cx), "float-decline-t1-1");
    let sent = answers(&said(&h, cx));
    assert_eq!((sent.len(), &sent[0]["declined"]), (1, &json!(true)));
}

#[gpui::test]
fn answered_in_its_inbox_the_card_goes(cx: &mut TestAppContext) {
    let (_store, _, h) = setup_inbox(vec![agent("web", "blocked", Some((BASH, true, true)))], true, cx);
    connect(&h, cx);
    say(&h, thread_t1(true), cx);
    click(&mut window(cx), "float-puck");
    assert!(window(cx).debug_bounds("float-ask-t1-1").is_some());
    let with = dims(cx).1;
    // The inbox answers it (through what its tile answers with).
    let ask = cx.update(|cx| asks(cx)[0].ask.clone());
    let req = cx.update(|cx| crate::main_screen::inbox_tile::answer_ask("demo", "t1", &ask, vec![0], None, false, cx)).expect("sent");
    cx.run_until_parked();
    assert!(window(cx).debug_bounds("float-ask-note-t1-1").is_some(), "on its way here too");
    say(&h, json!({"type": "result", "req": req, "ok": true}), cx);
    say(&h, thread_t1(false), cx);
    let mut v = window(cx);
    assert!(v.debug_bounds("float-ask-t1-1").is_none(), "gone here");
    assert!(v.debug_bounds("float-card-web").is_some(), "the agent's card stays");
    assert!(dims(cx).1 < with);
}

#[gpui::test]
fn without_an_inbox_that_is_on_it_is_as_before(cx: &mut TestAppContext) {
    let (_store, _, h) = setup_inbox(vec![agent("api", "working", None)], false, cx);
    assert!(h.read_with(cx, |h, _| !h.ends.contains_key("demo")), "its adapter never started");
    assert!(!up(cx) && cx.update(|cx| asks(cx)).is_empty());
    // Turned on, with a question: shown; turned off again: gone, and its
    // thread let go.
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", true)));
    cx.run_until_parked();
    connect(&h, cx);
    say(&h, thread_t1(true), cx);
    assert!(up(cx));
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("demo", false)));
    cx.run_until_parked();
    assert!(!up(cx));
    assert!(cx.update(|cx| cx.global::<Float>().opened.is_empty()));
}

#[gpui::test]
fn its_threads_are_let_go_while_pitwall_is_in_front(cx: &mut TestAppContext) {
    let (_store, _, h) = setup_inbox(vec![], true, cx);
    connect(&h, cx);
    assert_eq!(said(&h, cx), [json!({"type": "open", "thread": "t1"})]);
    say(&h, thread_t1(true), cx);
    assert!(up(cx));
    cx.update(|cx| cx.set_global(InFront(true)));
    cx.executor().advance_clock(WATCH * 2);
    cx.run_until_parked();
    assert!(!up(cx));
    assert_eq!(said(&h, cx), [json!({"type": "close", "thread": "t1"})]);
    cx.update(|cx| cx.set_global(InFront(false)));
    cx.executor().advance_clock(WATCH * 2);
    cx.run_until_parked();
    assert_eq!(said(&h, cx), [json!({"type": "open", "thread": "t1"})]);
}
