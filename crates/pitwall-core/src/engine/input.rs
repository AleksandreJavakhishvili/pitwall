//! Talking to a running agent: its output stream, keystrokes, size, prompts.

use super::{tasks, Engine, Shared};
use crate::model::AgentView;
use crate::term::{FrameSink, OutputSink, ScreenSource};

type Res<T> = Result<T, String>;

/// Replay the agent's buffered output to `sink`, then stream live output.
/// Returns the subscription id for [`detach_output`].
pub fn attach_output(engine: &Engine, agent_id: &str, sink: OutputSink) -> Res<u64> {
    Ok(engine.host(agent_id)?.attach(sink))
}

/// Agent `agent_id`'s terminal while it is attached, for a host that
/// streams it elsewhere view-only (`TermHost::watch_output`; another
/// machine's viewer). A restart makes a new one.
pub fn terminal(engine: &Engine, agent_id: &str) -> Option<std::sync::Arc<crate::term::TermHost>> {
    engine.with(agent_id, |a| a.host.clone()).ok().flatten()
}

pub fn detach_output(engine: &Engine, agent_id: &str, subscription_id: u64) {
    // Detaching from an agent that is gone or restarted is not an error.
    if let Ok(Some(host)) = engine.with(agent_id, |a| a.host.clone()) {
        host.detach(subscription_id);
    }
}

/// Make `source` (a UI's terminal emulator for the agent) the agent's
/// screen: it gets the buffered output, then all output as it comes, and
/// status rules and the Wall read it (`TermHost::share_screen`). Instead of
/// [`attach_output`] for a UI that parses the output anyway.
pub fn share_screen(engine: &Engine, agent_id: &str, source: Box<dyn ScreenSource>) -> Res<()> {
    engine.host(agent_id)?.share_screen(source);
    Ok(())
}

/// Whether a UI's terminal is the agent's screen ([`share_screen`]).
pub fn screen_shared(engine: &Engine, agent_id: &str) -> bool {
    engine.host(agent_id).is_ok_and(|h| h.screen_shared())
}

/// Styled frames of the agent's screen (the Wall), at most one per `gap`.
/// Returns the watch id for [`unwatch_screen`].
pub fn watch_screen(engine: &Engine, agent_id: &str, sink: FrameSink, gap: std::time::Duration) -> Res<u64> {
    Ok(engine.host(agent_id)?.watch_screen(sink, gap))
}

pub fn unwatch_screen(engine: &Engine, agent_id: &str, watch_id: u64) {
    // Like detach_output: a gone or restarted agent is not an error.
    if let Ok(Some(host)) = engine.with(agent_id, |a| a.host.clone()) {
        host.unwatch_screen(watch_id);
    }
}

/// Keystrokes typed in the agent's terminal.
pub fn write_input(engine: &Shared, agent_id: &str, data: &str) -> Res<()> {
    let host = engine.host(agent_id)?;
    host.touch_input();
    host.write(data.as_bytes())?;
    if data.contains('\r') {
        // Enter in the terminal: there's probably a conversation to resume now.
        let changed = engine.with(agent_id, |a| {
            // A terminal's shell in use after the agent last started in it
            // exited (terminals.rs: when it is forgotten).
            if a.is_terminal() && a.inner.is_none() && a.rec.inner_agent.as_ref().is_some_and(|m| m.left_at.is_some()) {
                a.shell_used = true;
            }
            !std::mem::replace(&mut a.rec.has_conversation, true)
        })?;
        if changed {
            engine.changed(true);
        }
        tasks::begin_for(engine, agent_id, None, false);
    }
    Ok(())
}

/// Raw bytes typed in the agent's terminal (a native terminal view): text
/// goes through [`write_input`]; bytes that aren't UTF-8 (legacy mouse
/// reports, 8-bit meta) are written as they are.
pub fn write_input_bytes(engine: &Shared, agent_id: &str, data: &[u8]) -> Res<()> {
    match std::str::from_utf8(data) {
        Ok(text) => write_input(engine, agent_id, text),
        Err(_) => {
            let host = engine.host(agent_id)?;
            host.touch_input();
            host.write(data)?;
            Ok(())
        }
    }
}

/// Answer the yes/no prompt on the agent's screen (a permission prompt) by
/// typing its keys: `accept` for yes, else no. For answering outside the
/// terminal, where what the user saw may be out of date: the screen is
/// read again, and nothing is typed unless it still shows `seen`
/// ([`pitwall_proto::AskView::text`]) and that prompt has the answer.
pub fn answer(engine: &Shared, agent_id: &str, accept: bool, seen: &str) -> Res<()> {
    let host = engine.host(agent_id)?;
    let kind = engine.with(agent_id, |a| a.kind().to_string())?;
    let (text, title) = host.screen_text();
    let d = pitwall_detect::detect(&kind, &text, title.as_deref());
    if d.state != Some(pitwall_detect::Detected::Blocked) {
        return Err("it no longer asks anything".into());
    }
    if d.excerpt.as_deref() != Some(seen) {
        return Err("its prompt changed".into());
    }
    let keys = match (d.answer, accept) {
        (Some(a), true) => a.accept,
        (Some(a), false) => a.reject.ok_or("this prompt has no key for no: open the agent")?,
        (None, _) => return Err("this prompt can't be answered from here: open the agent".into()),
    };
    host.touch_input();
    host.write(keys.as_bytes())?;
    Ok(())
}

/// The size the UI shows the agent at. Recorded even with no process
/// (stopped, or mid-restart) so the next start uses it.
pub fn resize(engine: &Engine, agent_id: &str, cols: u16, rows: u16) -> Res<()> {
    let (host, recorded) = engine.with(agent_id, |a| (a.host.clone(), a.rec.set_term_size((cols, rows))))?;
    let resized = host.is_some_and(|h| h.resize(cols, rows));
    if recorded || resized {
        engine.changed(recorded);
    }
    Ok(())
}

/// Send a prompt now (pasted verbatim, then Enter).
pub fn send_prompt(engine: &Shared, agent_id: &str, text: String) -> Res<()> {
    let now = engine.now();
    let session = engine.with(agent_id, |a| {
        let s = a.host.clone().filter(|_| a.running());
        if s.is_some() {
            a.note_sent(&text, now);
        }
        s
    })?;
    let session = session.ok_or("agent is not running")?;
    engine.changed(true);
    tasks::begin_for(engine, agent_id, Some(&text), true);
    session.touch_input();
    session.send_text(text);
    Ok(())
}

/// Take a queue item out and send it now.
pub fn queue_send_now(engine: &Shared, agent_id: &str, item_id: &str) -> Res<AgentView> {
    let now = engine.now();
    let (session, text, view) = engine
        .with(agent_id, |a| {
            let session = a.host.clone().filter(|_| a.running()).ok_or("agent is not running")?;
            let pos = a.rec.queue.iter().position(|q| q.id == item_id).ok_or("queue item not found")?;
            let item = a.rec.queue.remove(pos);
            a.note_sent(&item.text, now);
            Ok::<_, &str>((session, item.text, a.view()))
        })?
        .map_err(String::from)?;
    engine.changed(true);
    tasks::begin_for(engine, agent_id, Some(&text), true);
    session.send_text(text);
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{record, Harness};

    #[test]
    fn stopped_agents_refuse_io_but_remember_their_size() {
        let h = Harness::new(vec![record("a", "/tmp")]);
        let e = &h.engine;
        assert_eq!(write_input(e, "a", "x").unwrap_err(), "agent is not running");
        assert_eq!(write_input_bytes(e, "a", b"x").unwrap_err(), "agent is not running");
        assert_eq!(write_input_bytes(e, "a", &[0x1b, 0xff]).unwrap_err(), "agent is not running");
        assert_eq!(send_prompt(e, "a", "hi".into()).unwrap_err(), "agent is not running");
        assert!(attach_output(e, "a", Box::new(|_: &[u8]| true)).is_err());
        detach_output(e, "a", 7);
        detach_output(e, "ghost", 7);
        assert!(watch_screen(e, "a", Box::new(|_| true), std::time::Duration::from_millis(100)).is_err());
        unwatch_screen(e, "a", 7);
        unwatch_screen(e, "ghost", 7);
        let item = e.queue_add("a", "later".into()).unwrap().queue[0].id.clone();
        assert_eq!(queue_send_now(e, "a", &item).unwrap_err(), "agent is not running");
        assert_eq!(e.views()[0].queue.len(), 1, "a refused send keeps the item");

        resize(e, "a", 90, 30).unwrap();
        assert_eq!((e.views()[0].cols, e.views()[0].rows), (90, 30));
        assert_eq!(e.records()[0].term_size(), Some((90, 30)));
        assert!(resize(e, "ghost", 90, 30).is_err());
        assert_eq!(answer(e, "a", true, "x").unwrap_err(), "agent is not running");
    }

    /// A Claude Code permission prompt as it draws one (detect's fixtures).
    const PROMPT: &str = "\u{23fa} Bash(touch a.txt)\r\n\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\r\n Bash command\r\n   touch a.txt\r\n Do you want to proceed?\r\n \u{276f} 1. Yes\r\n   2. No\r\n\r\n Esc to cancel \u{b7} Tab to amend";

    /// Agent "a" running Claude Code in a fake terminal showing `screen`.
    fn showing(screen: &str) -> (Harness, crate::testing::FakeTermCtl, String) {
        use crate::provider::TermSize;
        use crate::term::TermHost;
        let h = Harness::new(vec![record("a", "/tmp")]);
        let (term, ctl) = crate::testing::FakeTerm::new(TermSize::new(80, 24), None);
        let host = TermHost::new(Box::new(term), TermSize::new(80, 24), crate::testing::ManualClock::new(1));
        ctl.output(screen.as_bytes());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !host.screen_text().0.contains("Esc to cancel") && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (text, title) = host.screen_text();
        let seen = pitwall_detect::detect("claude", &text, title.as_deref()).excerpt.unwrap_or_default();
        h.engine
            .with("a", |a| {
                a.rec.kind = "claude".into();
                a.host = Some(host);
            })
            .unwrap();
        (h, ctl, seen)
    }

    fn typed(ctl: &crate::testing::FakeTermCtl, want: &[u8]) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while ctl.input() != want && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        ctl.input() == want
    }

    #[test]
    fn a_yes_no_prompt_is_answered_with_its_keys() {
        let (h, ctl, seen) = showing(PROMPT);
        assert!(seen.contains("touch a.txt") && seen.ends_with("Tab to amend"), "{seen}");
        answer(&h.engine, "a", true, &seen).unwrap();
        assert!(typed(&ctl, b"1"), "accept types 1: {:?}", ctl.input());

        let (h, ctl, seen) = showing(PROMPT);
        answer(&h.engine, "a", false, &seen).unwrap();
        assert!(typed(&ctl, b"\x1b"), "reject types Esc: {:?}", ctl.input());
    }

    #[test]
    fn nothing_is_typed_when_the_prompt_is_not_the_one_seen() {
        // Another prompt took its place since the user looked.
        let (h, ctl, seen) = showing(PROMPT);
        let other = seen.replace("touch a.txt", "rm -rf build");
        assert_eq!(answer(&h.engine, "a", true, &other).unwrap_err(), "its prompt changed");
        // A question with several options: blocked, but not a yes/no.
        let question = "\u{2500}\u{2500}\u{2500}\u{2500}\r\nWhich color?\r\n\u{276f} 1. Red\r\n  2. Blue\r\n\u{2500}\u{2500}\u{2500}\u{2500}\r\n  3. Chat about this\r\n\r\nEnter to select \u{b7} \u{2191}/\u{2193} to navigate \u{b7} Esc to cancel";
        let (h2, ctl2, seen2) = showing(question);
        assert!(!seen2.is_empty());
        assert!(answer(&h2.engine, "a", true, &seen2).unwrap_err().contains("can't be answered from here"));
        // The prompt is gone: the agent is at its prompt box again.
        let idle = "\u{2500}\u{2500}\u{2500}\u{2500}\r\n\u{276f} \r\n\u{2500}\u{2500}\u{2500}\u{2500}\r\n  ? for shortcuts  Esc to cancel";
        let (h3, ctl3, _) = showing(idle);
        assert_eq!(answer(&h3.engine, "a", true, &seen).unwrap_err(), "it no longer asks anything");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(ctl.input().is_empty() && ctl2.input().is_empty() && ctl3.input().is_empty());
    }
}
