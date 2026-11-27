//! The questions tile ("Release desk"): the test fixture of the agents'
//! questions group. It lists Pitwall's agents (`agents.read`), asks one
//! question about the agent that waits on the user (or the first) in the
//! floating control, with a badge on that agent's row while it is open,
//! and puts a prompt in an agent's Next up when its Queue button is
//! pressed (`agents.queue`). Typing "later" as the answer is refused, to
//! test the refusal; "Ask again" opens the question again. While the question
//! is open, keys 1 and 2 pick its suggestions in the tile too. It reports
//! its status (`status.set`: connected once it read the agents, else an
//! error), and a note written under it is sent with take-back (Undo for 5
//! seconds, then "Sent: …"). Its manifest suggests a shortcut for it.

use pitwall_extension::prelude::*;

/// What Queue puts in Next up.
const PROMPT: &str = "Rebase on main, run the tests and say what failed";

#[derive(Default)]
struct Desk {
    agents: Vec<Agent>,
    /// Why the agents can't be read.
    error: Option<String>,
    /// The question is open.
    asking: bool,
    /// What happened last, for the line under the title.
    last: String,
    /// The note being written.
    note: String,
    /// Notes sent, held for take-back first.
    sending: TakeBack<String>,
}

fn sending(d: &mut Desk) -> &mut TakeBack<String> {
    &mut d.sending
}

impl Desk {
    fn load(&mut self) {
        match agents::list() {
            Ok(list) => {
                self.agents = list;
                self.error = None;
                status::set(status::State::Ready, "");
            }
            Err(e) => {
                self.agents.clear();
                self.error = Some(e.to_string());
                status::set(status::State::Error, &format!("Can't read the agents: {e}"));
            }
        }
    }

    /// The agent the question is about: the first that waits on the user,
    /// else the first.
    fn about(&self) -> Option<&Agent> {
        self.agents.iter().find(|a| a.waiting_on_you).or(self.agents.first())
    }

    /// The question and the badge, as they stand.
    fn publish(&self) {
        let about = self.about();
        let (open, marks) = match (self.asking, about) {
            (true, Some(a)) => (
                vec![Ask::new("deploy", format!("{} finished the release branch. Deploy api to staging?", a.name))
                    .title("Release 0.9")
                    .from_agent(a.id.clone())
                    .suggest_with("Deploy", "Ship 0.9 to staging now")
                    .suggest_with("Not now", "Keep it on the branch")
                    .blocking()],
                vec![Badge::count(Row::Agent(a.id.clone()), 1).tone(Tone::Warn).tooltip("1 question waiting").open("main")],
            ),
            (true, None) => (vec![Ask::new("deploy", "Deploy api to staging?").title("Release 0.9").suggest("Deploy").suggest("Not now").blocking()], vec![]),
            _ => (vec![], vec![]),
        };
        if let Err(e) = asks::set(&open) {
            log::warn(format!("asks: {e}"));
        }
        if let Err(e) = badges::set(&marks) {
            log::warn(format!("badges: {e}"));
        }
    }

    fn answered(&mut self, a: &Answer) {
        if a.text.as_deref().is_some_and(|t| t.trim() == "later") {
            asks::reply(&a.id, Some("The release isn't ready yet"));
            self.last = "Refused an answer".into();
            return;
        }
        asks::reply(&a.id, None);
        let labels = ["Deploy", "Not now"];
        self.last = if a.declined {
            "Declined".into()
        } else {
            let picked: Vec<&str> = a.picks.iter().filter_map(|i| labels.get(*i as usize).copied()).collect();
            let typed = a.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
            match (picked.is_empty(), typed) {
                (false, _) => format!("Answered: {}", picked.join(", ")),
                (true, Some(t)) => format!("Answered: {t}"),
                (true, None) => "Answered".into(),
            }
        };
        self.asking = false;
        self.publish();
    }
}

impl Tile for Desk {
    fn new(_cx: &mut Cx<Self>) -> Self {
        let mut d = Desk { asking: true, ..Desk::default() };
        d.load();
        d.publish();
        d
    }

    fn render(&mut self, _view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        // Number keys pick a suggestion while the question is open (not
        // while the note field has focus: there they type).
        if self.asking {
            for (key, label) in [("1", "Deploy"), ("2", "Not now")] {
                let h = cx.listener(move |d: &mut Desk, _: &KeyPress, cx| {
                    d.last = format!("Answered: {label}");
                    d.asking = false;
                    d.publish();
                    cx.notify();
                });
                cx.bind_key(key, h);
            }
        }
        let note = h_flex()
            .id("note")
            .gap_2()
            .items_center()
            .child(input("text").flex_1().placeholder("A note for the release").value(self.note.clone()).on_change(cx.listener(|d: &mut Desk, v: &str, cx| {
                d.note = v.to_string();
                cx.notify();
            })))
            .child(button("Send").id("send").small().disabled(self.note.trim().is_empty()).on_click(cx.listener(|d: &mut Desk, _, cx| {
                if d.note.trim().is_empty() {
                    return;
                }
                let text = std::mem::take(&mut d.note);
                d.sending.hold(text, "Your note", cx, sending, |d, text, cx| {
                    d.last = format!("Sent: {text}");
                    cx.notify();
                });
            })));
        let held = self.sending.view(cx, sending, |d, text, _| d.note = text);
        let mut list = v_flex().id("agents").gap_1();
        for a in &self.agents {
            let id = a.id.clone();
            let name = a.name.clone();
            let mut info = format!("{} · {}", a.kind_name, state(a.state));
            if a.queued > 0 {
                info.push_str(&format!(" · {} queued", a.queued));
            }
            list = list.child(
                h_flex()
                    .id(format!("a-{}", a.id))
                    .gap_2()
                    .items_center()
                    .p(px(6.0))
                    .rounded_md()
                    .bg(Token::Surface2)
                    .child(
                        v_flex()
                            .flex_1()
                            .child(text(a.name.clone()).id("name").text_color(Token::Text))
                            .child(text(info).id("info").text_sm().text_color(Token::Text3)),
                    )
                    .child(if a.waiting_on_you { text("Waiting on you").id("waiting").text_sm().text_color(Token::Warn) } else { text("").id("waiting") })
                    .child(button("Queue").id("queue").small().icon(Icon::Send).on_click(cx.listener(move |d: &mut Desk, _, cx| {
                        d.last = match agents::queue_add(&id, PROMPT) {
                            Ok(()) => format!("Queued for {name}"),
                            Err(e) => format!("Not queued: {e}"),
                        };
                        cx.notify();
                    }))),
            );
        }
        let empty = match &self.error {
            Some(e) => Some(format!("Can't read the agents: {e}")),
            None if self.agents.is_empty() => Some("No agents".to_string()),
            None => None,
        };
        v_flex()
            .gap_2()
            .p(px(12.0))
            .bg(Token::Surface)
            .child(
                h_flex()
                    .id("head")
                    .items_center()
                    .justify_between()
                    .child(text("Release 0.9").id("title").text_lg().text_color(Token::Text))
                    .child(button("Ask again").id("ask").small().disabled(self.asking).on_click(cx.listener(|d: &mut Desk, _, cx| {
                        d.asking = true;
                        d.last.clear();
                        d.publish();
                        cx.notify();
                    }))),
            )
            .child(text(if self.asking { "A question waits in the floating control" } else { "No question open" }).id("asking").text_sm().text_color(Token::Text2))
            .child(text(self.last.clone()).id("last").text_sm().text_color(Token::Text3))
            .children(empty.map(|e| text(e).id("empty").text_color(Token::Text3)))
            .child(list)
            .child(note)
            .child(held)
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        match event {
            Event::AgentsChanged | Event::GrantsChanged(_) => {
                self.load();
                self.publish();
                cx.notify();
            }
            Event::AskAnswered(a) => {
                self.answered(a);
                cx.notify();
            }
            Event::Theme(_) => cx.notify(),
            _ => {}
        }
    }
}

fn state(s: agents::State) -> &'static str {
    match s {
        agents::State::Working => "working",
        agents::State::Blocked => "blocked",
        agents::State::Idle => "idle",
        agents::State::Done => "done",
        agents::State::Stopped => "stopped",
        agents::State::Unknown => "unknown",
    }
}

pitwall_extension::export!(Desk);
