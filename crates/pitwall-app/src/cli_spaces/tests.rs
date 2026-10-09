use std::time::{Duration, Instant};

use gpui::TestAppContext;

use super::*;

#[gpui::test]
fn spaces_change_through_the_ui_store(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::ui_state::store(cx);
        let all = views(&crate::ui_state::get(cx));
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].id.as_str(), all[0].kind.as_str(), all[0].window.as_str()), ("all", "all", "main"));

        let made = create("API work", cx).unwrap();
        assert_eq!((made.name.as_str(), made.kind.as_str(), made.window.as_str()), ("API work", "custom", "main"));
        assert_eq!(crate::ui_state::get(cx).spaces.len(), 2, "in the one store every window reads");

        // By id or by name.
        let renamed = rename("api work", "API", cx).unwrap();
        assert_eq!((renamed.id.as_str(), renamed.name.as_str()), (made.id.as_str(), "API"));
        assert_eq!(rename("All", "x", cx).unwrap_err().code, code::BAD_PARAMS);
        assert_eq!(rename("nope", "x", cx).unwrap_err().code, code::NOT_FOUND);

        // An agent dropped on it: shown there and a member, gone elsewhere.
        crate::ui_state::update(cx, |ui| *ui = ui.clone().place_agent(ALL_SPACE, "a1", None, None));
        let there = move_agent("a1", "API", cx).unwrap();
        assert_eq!((there.shown.clone(), there.members.clone()), (vec!["a1".to_string()], vec!["a1".to_string()]));
        let ui = crate::ui_state::get(cx);
        assert_eq!(ui.locate("a1").unwrap().space_id, made.id);
        assert!(view(&ui, ALL_SPACE).unwrap().shown.is_empty());

        // Windows: only open ones (or a new one); All stays in main.
        assert_eq!(move_to_window("All", "new", cx).unwrap_err().code, code::BAD_PARAMS);
        let e = move_to_window("API", "pitwall-7", cx).unwrap_err();
        assert!(e.code == code::NOT_FOUND && e.message.contains("no open window"), "{e}");
        assert_eq!(move_to_window("API", "main", cx).unwrap().window, "main");
        // No window can open in a test app: refused, nothing changed.
        assert_eq!(move_to_window("API", "new", cx).unwrap_err().code, code::OTHER);
        assert_eq!(crate::ui_state::get(cx).window_of_space(&made.id), "main");
    });
}

/// The server's thread asks; the main thread answers.
#[gpui::test]
fn requests_from_the_server_thread_run_on_the_main_thread(cx: &mut TestAppContext) {
    let b = AppWorkspace::new();
    cx.update(|cx| {
        crate::ui_state::store(cx);
        attach(b.clone(), cx);
    });
    let asking = {
        let b = b.clone();
        std::thread::spawn(move || (b.create("Review").unwrap(), b.spaces().unwrap()))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !asking.is_finished() && Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    let (made, all) = asking.join().unwrap();
    assert_eq!(made.name, "Review");
    assert_eq!(all.len(), 2);
    cx.update(|cx| assert!(crate::ui_state::get(cx).space(&made.id).is_some()));
}
