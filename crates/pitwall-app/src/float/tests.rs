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
}

/// The control over a store of `agents`, the user away from Pitwall.
/// `fail`: what typing an answer fails with.
fn setup(agents: Vec<AgentView>, fail: Option<&'static str>, cx: &mut TestAppContext) -> (Entity<AgentStore>, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let store = cx.new(|_| AgentStore::new(agents));
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        cx.set_global(InFront(false));
        let (a, o, k) = (calls.clone(), calls.clone(), calls.clone());
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
    assert_eq!((w, h), model::size(&list, true, &|_| false, &|_| None));
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

    // Keys go nowhere until the terminal is clicked; then to its agent.
    v.simulate_keystrokes("down");
    assert!(calls.typed.lock().unwrap().is_empty());
    click(&mut v, "float-term-api");
    v.simulate_keystrokes("down enter 2 escape");
    let typed = calls.typed.lock().unwrap().clone();
    assert_eq!(typed.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["api"; 4]);
    assert_eq!(typed.iter().map(|(_, k)| k.as_slice()).collect::<Vec<_>>(), [b"\x1b[B".as_slice(), b"\r", b"2", b"\x1b"]);

    // Closed again, or gone: nothing is watched any more.
    click(&mut v, "float-collapse");
    let _ = window(cx);
    assert!(fake.watching.lock().unwrap().is_empty());
    click(&mut window(cx), "float-puck");
    assert_eq!(*fake.watching.lock().unwrap(), ["api"]);
    set_agents(&store, vec![agent("api", "working", None)], cx);
    assert!(!up(cx) && fake.watching.lock().unwrap().is_empty());
}
