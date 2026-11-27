//! The questions tile ("Release desk"): the test fixture of the agents'
//! questions group. It lists Pitwall's agents (`agents.read`), asks one
//! question about the agent that waits on the user (or the first) in the
//! floating control, with a badge on that agent's row while it is open,
//! and puts a prompt in an agent's Next up when its Queue button is
//! pressed (`agents.queue`). Typing "later" as the answer is refused, to
//! test the refusal; "Ask again" opens the question again.

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
}

impl Desk {
    fn load(&mut self) {
        match agents::list() {
            Ok(list) => {
                self.agents = list;
                self.error = None;
            }
            Err(e) => {
                self.agents.clear();
                self.error = Some(e.to_string());
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
