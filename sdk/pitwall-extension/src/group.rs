//! The agents' questions group: Pitwall's agents and their Next up
//! ([`agents`]), counts and notes on sidebar rows ([`badges`]), and
//! questions in the floating control ([`asks`]).

use crate::bindings::pitwall::tile as wit;
use crate::host::{HostError, Tone};

/// Pitwall's agents, and their Next up. Reading them needs the
/// `agents.read` permission, queueing text `agents.queue`; without the
/// grant a call returns [`HostError::Denied`].
///
/// ```ignore
/// use pitwall_extension::prelude::*;
///
/// // Who waits on the user?
/// let waiting: Vec<agents::Agent> = agents::list()?.into_iter().filter(|a| a.waiting_on_you).collect();
/// // Put a prompt in the first one's Next up, marked with the extension's name.
/// agents::queue_add(&waiting[0].id, "Rebase on main and run the tests")?;
/// ```
pub mod agents {
    use super::{wit, HostError};

    /// What an agent is doing now.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum State {
        /// Working on its turn.
        Working,
        /// It asks the user something and is held up until answered.
        Blocked,
        /// Waiting for its next prompt.
        Idle,
        /// Done with its turn.
        Done,
        /// Not running: it exited or was stopped.
        Stopped,
        /// Pitwall can't tell yet.
        Unknown,
    }

    /// An agent (or a terminal) Pitwall shows.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Agent {
        /// Pitwall's id for it: what [`queue_add`] and a badge's
        /// [`Row::Agent`](crate::badges::Row::Agent) take.
        pub id: String,
        /// Its name, as the sidebar shows it.
        pub name: String,
        /// Its kind's id (`claude`, `codex`, `shell` for a plain terminal…).
        pub kind: String,
        /// Its kind's name ("Claude Code").
        pub kind_name: String,
        /// Its project's folder.
        pub project: String,
        /// The git branch it works on, when known.
        pub branch: Option<String>,
        /// What it is doing.
        pub state: State,
        /// It waits on the user (blocked on an approval or a question).
        pub waiting_on_you: bool,
        /// Items waiting in its Next up.
        pub queued: u32,
    }

    impl From<wit::agents::Agent> for Agent {
        fn from(a: wit::agents::Agent) -> Agent {
            use wit::agents::AgentState as S;
            Agent {
                id: a.id,
                name: a.name,
                kind: a.kind,
                kind_name: a.kind_name,
                project: a.project,
                branch: a.branch,
                state: match a.state {
                    S::Working => State::Working,
                    S::Blocked => State::Blocked,
                    S::Idle => State::Idle,
                    S::Done => State::Done,
                    S::Stopped => State::Stopped,
                    S::Unknown => State::Unknown,
                },
                waiting_on_you: a.waiting_on_you,
                queued: a.queued,
            }
        }
    }

    /// Every agent Pitwall shows, in the sidebar's order. Needs
    /// `agents.read`. The tile hears
    /// [`Event::AgentsChanged`](crate::Event::AgentsChanged) when this
    /// changes.
    pub fn list() -> Result<Vec<Agent>, HostError> {
        Ok(wit::agents::list().map_err(HostError::from)?.into_iter().map(Agent::from).collect())
    }

    /// The most characters [`queue_add`] takes.
    pub const MAX_TEXT: usize = 4000;

    /// Put `text` in agent `agent`'s Next up, marked "From <extension
    /// name>" and waiting for the user's Send (Auto-send sends it only
    /// when the user lets the extension send without asking). Needs
    /// `agents.queue`. At most [`MAX_TEXT`] characters
    /// ([`HostError::TooLarge`]); at most 3 of the extension's items wait
    /// in one agent's Next up and one is queued per agent every 10 seconds
    /// ([`HostError::Busy`], the reason in the tile's Log).
    pub fn queue_add(agent: &str, text: impl AsRef<str>) -> Result<(), HostError> {
        wit::agents::queue_add(agent, text.as_ref()).map_err(HostError::from)
    }
}

/// Counts and short notes on sidebar rows, beside Pitwall's own waiting
/// badges. The manifest must say `"badges": true` in `contributes`; a
/// badge on an agent needs `agents.read`, on a project `projects.read`.
///
/// ```ignore
/// badges::set(&[Badge::count(Row::Agent(agent.id.clone()), 2).tone(Tone::Bad).tooltip("2 failing runs").open("main")])?;
/// ```
pub mod badges {
    use super::{wit, HostError, Tone};

    /// Which sidebar row a badge is on.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Row {
        /// An agent, by Pitwall's id ([`agents::Agent::id`](crate::agents::Agent::id)).
        Agent(String),
        /// A project, by its folder.
        Project(String),
    }

    /// A count and/or a few words on a row.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Badge {
        /// The row.
        pub row: Row,
        /// A number (at most 9999).
        pub count: Option<u32>,
        /// A few words (at most 16 characters).
        pub note: Option<String>,
        /// Its colour by meaning ([`Tone::Info`] is neutral).
        pub tone: Tone,
        /// Shown on hover after the extension's name.
        pub tooltip: Option<String>,
        /// One of the extension's tiles that a click opens.
        pub open: Option<String>,
    }

    impl Badge {
        /// A count on `row`.
        pub fn count(row: Row, count: u32) -> Badge {
            Badge { row, count: Some(count), note: None, tone: Tone::Info, tooltip: None, open: None }
        }

        /// A few words on `row`.
        pub fn note(row: Row, note: impl Into<String>) -> Badge {
            Badge { row, count: None, note: Some(note.into()), tone: Tone::Info, tooltip: None, open: None }
        }

        /// Its tone.
        pub fn tone(mut self, tone: Tone) -> Badge {
            self.tone = tone;
            self
        }

        /// Words shown on hover.
        pub fn tooltip(mut self, tooltip: impl Into<String>) -> Badge {
            self.tooltip = Some(tooltip.into());
            self
        }

        /// Open tile `tile` (an id from the manifest) when clicked.
        pub fn open(mut self, tile: impl Into<String>) -> Badge {
            self.open = Some(tile.into());
            self
        }

        fn wit(&self) -> wit::badges::Badge {
            wit::badges::Badge {
                row: match &self.row {
                    Row::Agent(id) => wit::badges::Row::Agent(id.clone()),
                    Row::Project(p) => wit::badges::Row::Project(p.clone()),
                },
                count: self.count,
                note: self.note.clone(),
                tone: self.tone.wit(),
                tooltip: self.tooltip.clone(),
                open: self.open.clone(),
            }
        }
    }

    /// The whole set of badges, each time (`&[]` clears them). At most
    /// 500; at most four sets a second are shown (the newest waits). A
    /// badge on a row the tile may not name is dropped and noted in its
    /// Log.
    pub fn set(badges: &[Badge]) -> Result<(), HostError> {
        let list: Vec<wit::badges::Badge> = badges.iter().map(Badge::wit).collect();
        wit::badges::set(&list).map_err(HostError::from)
    }
}

/// Questions in Pitwall's floating control, shown as an inbox's are: who
/// asks, the question, the suggestions, a field, Send, Decline and "Open
/// <tile>". The manifest must say `"asks": true` in `contributes`. The
/// answer arrives as [`Event::AskAnswered`](crate::Event::AskAnswered);
/// reply with [`reply`].
///
/// ```ignore
/// asks::set(&[Ask::new("deploy-42", "Deploy api to staging?").title("Release 0.9").suggest("Deploy").suggest("Not now").blocking()])?;
/// ```
pub mod asks {
    use super::{wit, HostError};

    /// An answer the user can pick with one click.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Suggestion {
        /// Its words (at most 80 characters).
        pub label: String,
        /// A line under it.
        pub description: Option<String>,
    }

    /// A question the tile asks the user.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Ask {
        /// The tile's own id for it, stable while it is open.
        pub id: String,
        /// Who asks: an agent by Pitwall's id (needs `agents.read`);
        /// `None`: the extension.
        pub from: Option<String>,
        /// What it is about (at most 200 characters).
        pub title: String,
        /// The question (at most 2 000 characters).
        pub question: String,
        /// Answers to pick (at most 8).
        pub suggestions: Vec<Suggestion>,
        /// Several suggestions may be picked.
        pub multiple: bool,
        /// It holds something up: shown first, "Waiting on you".
        pub blocking: bool,
    }

    impl Ask {
        /// Question `question`, known by `id`.
        pub fn new(id: impl Into<String>, question: impl Into<String>) -> Ask {
            Ask { id: id.into(), from: None, title: String::new(), question: question.into(), suggestions: Vec::new(), multiple: false, blocking: false }
        }

        /// What it is about.
        pub fn title(mut self, title: impl Into<String>) -> Ask {
            self.title = title.into();
            self
        }

        /// Asked on behalf of agent `agent` (its id).
        pub fn from_agent(mut self, agent: impl Into<String>) -> Ask {
            self.from = Some(agent.into());
            self
        }

        /// A suggestion.
        pub fn suggest(mut self, label: impl Into<String>) -> Ask {
            self.suggestions.push(Suggestion { label: label.into(), description: None });
            self
        }

        /// A suggestion with a line under it.
        pub fn suggest_with(mut self, label: impl Into<String>, description: impl Into<String>) -> Ask {
            self.suggestions.push(Suggestion { label: label.into(), description: Some(description.into()) });
            self
        }

        /// Several suggestions may be picked.
        pub fn multiple(mut self) -> Ask {
            self.multiple = true;
            self
        }

        /// It holds something up: shown first, "Waiting on you".
        pub fn blocking(mut self) -> Ask {
            self.blocking = true;
            self
        }

        fn wit(&self) -> wit::asks::Ask {
            wit::asks::Ask {
                id: self.id.clone(),
                from: self.from.clone(),
                title: self.title.clone(),
                question: self.question.clone(),
                suggestions: self.suggestions.iter().map(|s| wit::asks::Suggestion { label: s.label.clone(), description: s.description.clone() }).collect(),
                multiple: self.multiple,
                blocking: self.blocking,
            }
        }
    }

    /// The user's answer to one of the tile's questions.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Answer {
        /// The question's id.
        pub id: String,
        /// The suggestions picked, by their place in its list (from 0).
        pub picks: Vec<u32>,
        /// What the user typed, when anything.
        pub text: Option<String>,
        /// The user declined to answer.
        pub declined: bool,
    }

    impl From<wit::types::AskAnswer> for Answer {
        fn from(a: wit::types::AskAnswer) -> Answer {
            Answer { id: a.id, picks: a.picks, text: a.text, declined: a.declined }
        }
    }

    /// The whole set of open questions, each time (`&[]` clears them). At
    /// most 50. Pitwall keeps them while the tile is hidden or let go, so
    /// they stay in the control.
    pub fn set(open: &[Ask]) -> Result<(), HostError> {
        let list: Vec<wit::asks::Ask> = open.iter().map(Ask::wit).collect();
        wit::asks::set(&list).map_err(HostError::from)
    }

    /// After an answer: `None` when it was taken (then send a set without
    /// the question), or why not, which the card shows ("Not sent: …") so
    /// the user can answer again.
    pub fn reply(id: &str, refused: Option<&str>) {
        wit::asks::reply(id, refused);
    }
}
