//! Talking to a running agent: its output stream, keystrokes, size, prompts.

use super::{tasks, Engine, Shared};
use crate::model::AgentView;
use crate::term::{FrameSink, OutputSink};

type Res<T> = Result<T, String>;

/// Replay the agent's buffered output to `sink`, then stream live output.
/// Returns the subscription id for [`detach_output`].
pub fn attach_output(engine: &Engine, agent_id: &str, sink: OutputSink) -> Res<u64> {
    Ok(engine.host(agent_id)?.attach(sink))
}

pub fn detach_output(engine: &Engine, agent_id: &str, subscription_id: u64) {
    // Detaching from an agent that is gone or restarted is not an error.
    if let Ok(Some(host)) = engine.with(agent_id, |a| a.host.clone()) {
        host.detach(subscription_id);
    }
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
    }
}
