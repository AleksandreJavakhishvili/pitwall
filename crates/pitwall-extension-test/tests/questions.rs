//! The agents' questions group through the harness, on the tile host's
//! committed questions tile ("Release desk"; rebuilt by
//! crates/pitwall-tile-host/tests/fixtures/build-questions-tile.sh): the
//! agents it reads, its badge, its question and its answers, and what it
//! queues.

use std::path::{Path, PathBuf};

use pitwall_extension_test::{agent, waiting_agent, Harness, Level, Row, Tone};

fn questions() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/questions")
}

fn granted() -> Harness {
    Harness::load(questions()).grant("agents.read").grant("agents.queue").agent(agent("a1", "web")).agent(waiting_agent("a2", "api"))
}

#[test]
fn it_reads_the_agents_badges_the_one_it_asks_about_and_asks() {
    let t = granted().open("main");
    assert!(t.find_text("web").is_some() && t.find_text("Claude Code · blocked").is_some(), "{}", t.text_snapshot());
    assert!(t.find_text("Waiting on you").is_some());
    // The badge is on the agent that waits, and opens the tile.
    let b = t.badges();
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!((&b[0].row, b[0].count, b[0].tone, b[0].open.as_deref()), (&Row::Agent("a2".into()), Some(1), Tone::Warn, Some("main")));
    // The question names it.
    let a = t.asks();
    assert_eq!(a.len(), 1);
    assert_eq!((a[0].id.as_str(), a[0].from.as_deref(), a[0].blocking, a[0].suggestions.len()), ("deploy", Some("a2"), true, 2));
    assert!(a[0].question.starts_with("api finished the release branch"), "{}", a[0].question);
}

#[test]
fn an_answer_comes_back_and_a_refusal_keeps_the_question() {
    let mut t = granted().open("main");
    t.answer_text("deploy", "later");
    assert_eq!(t.reply("deploy"), Some(Some("The release isn't ready yet".into())));
    assert_eq!(t.asks().len(), 1, "a refused answer leaves it open");
    t.answer("deploy", &[0]);
    assert_eq!(t.reply("deploy"), Some(None));
    assert!(t.asks().is_empty() && t.badges().is_empty(), "answered: no question and no badge");
    assert!(t.find_text("Answered: Deploy").is_some(), "{}", t.text_snapshot());
    t.click_text("Ask again").decline("deploy");
    assert!(t.find_text("Declined").is_some());
}

#[test]
fn it_queues_a_prompt_and_follows_the_agents() {
    let mut t = granted().open("main");
    t.click_key("agents/a-a1/queue");
    let q = t.queued();
    assert_eq!((q.len(), q[0].agent.as_str()), (1, "a1"));
    assert!(q[0].text.starts_with("Rebase on main"));
    assert!(t.find_text("Queued for web").is_some(), "{}", t.text_snapshot());
    // Pitwall counts it in the agent's Next up; the tile hears the change.
    let now = t.agents();
    t.set_agents(now);
    assert!(t.find_text("Claude Code · idle · 1 queued").is_some(), "{}", t.text_snapshot());
    // Three wait at most; the fourth is refused, saying why in the Log.
    t.click_key("agents/a-a1/queue").click_key("agents/a-a1/queue").click_key("agents/a-a1/queue");
    assert_eq!(t.queued().len(), 3);
    assert!(t.find_text("Not queued: too many calls at once; try again later").is_some(), "{}", t.text_snapshot());
    assert!(t.logged(Level::Warn, "queue.add: 3 of its items already wait in web's Next up"), "{}", t.log_text());
    // The agent that waited is gone: the badge and the question move on.
    t.set_agents(vec![agent("a1", "web")]);
    assert_eq!(t.badges()[0].row, Row::Agent("a1".into()));
    assert_eq!(t.asks()[0].from.as_deref(), Some("a1"));
}

#[test]
fn without_grants_it_reads_and_queues_nothing() {
    let t = Harness::load(questions()).agent(waiting_agent("a2", "api")).open("main");
    assert!(t.find_text("Can't read the agents: not allowed: the permission isn't granted").is_some(), "{}", t.text_snapshot());
    // It still asks, as the extension (no agent to name).
    assert_eq!(t.asks()[0].from, None);
    assert!(t.badges().is_empty());
}
