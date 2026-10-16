//! The plain-language summary of a share (collaboration/sharing.md §5.4):
//! fixed sentences made from the answers, never written by a model. The
//! Share dialog's Summary step, `pitwall share create`'s output and the
//! go-live approval all show it.

use pitwall_proto::api::collab::{ShareSummary, SummaryPart, SummarySentence, SummaryTopic};
use pitwall_proto::collab::{Capability, Programs, ShareSettings, Start};

/// What the summary needs besides the settings: names and counts the
/// owner's Pitwall knows.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Each audience team's name and member count, in the audience's order
    /// (`None`: a team this Pitwall doesn't know any more).
    pub teams: Vec<Option<(String, u32)>>,
    /// Files in the copy after exclusions, when counted.
    pub files: Option<u32>,
    /// The agent kind's name (`Claude Code`).
    pub kind_name: String,
    /// The account's name (`work`), when sessions run on one.
    pub account: Option<String>,
}

struct Sentence {
    topic: SummaryTopic,
    parts: Vec<SummaryPart>,
}

impl Sentence {
    fn new(topic: SummaryTopic) -> Sentence {
        Sentence { topic, parts: Vec::new() }
    }

    fn t(mut self, text: impl Into<String>) -> Sentence {
        self.parts.push(SummaryPart { text: text.into(), strong: false });
        self
    }

    fn b(mut self, text: impl Into<String>) -> Sentence {
        self.parts.push(SummaryPart { text: text.into(), strong: true });
        self
    }

    fn done(self) -> SummarySentence {
        SummarySentence { topic: self.topic, parts: self.parts }
    }
}

/// `a`, `a and b`, `a, b and c`.
pub fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn code_list(items: &[String]) -> String {
    items.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")
}

/// Who may send messages, as a phrase (`bob@example.com and team Platform (6 people)`).
pub fn who_phrase(s: &ShareSettings, facts: &Facts) -> Option<String> {
    let a = &s.audience;
    let mut who: Vec<String> = a.people.iter().map(|p| p.email.clone().unwrap_or_else(|| p.subject.clone())).collect();
    for (i, t) in a.teams.iter().enumerate() {
        who.push(match facts.teams.get(i).cloned().flatten() {
            Some((name, n)) => format!("team {name} ({})", plural(n, "person", "people")),
            None => format!("team {}", t.as_str()),
        });
    }
    who.extend(a.groups.iter().map(|g| format!("group {} on {}", g.id, g.network.as_str().split_once(':').map_or(g.network.as_str(), |(_, n)| n))));
    who.extend(a.networks.iter().map(|n| format!("everyone on {}", n.as_str().split_once(':').map_or(n.as_str(), |(_, x)| x))));
    (!who.is_empty()).then(|| and_list(&who))
}

/// The repository's folder name (`~/code/api` → `api`).
fn repo_name(repo: &str) -> &str {
    repo.trim_end_matches('/').rsplit('/').next().unwrap_or(repo)
}

/// The summary of `s`.
pub fn summarize(s: &ShareSettings, facts: &Facts) -> ShareSummary {
    let mut out: Vec<SummarySentence> = Vec::new();

    // Who.
    out.push(match who_phrase(s, facts) {
        Some(who) => Sentence::new(SummaryTopic::Who).b(who).t(" can send messages.").done(),
        None => Sentence::new(SummaryTopic::Who).b("Nobody yet").t(" can send messages: add people or a team.").done(),
    });

    // Capability.
    out.push(match s.capability {
        Capability::Read => Sentence::new(SummaryTopic::Capability).t("It can read its own copy; ").b("it can't change any code").t(".").done(),
        Capability::Write => Sentence::new(SummaryTopic::Capability).t("It can change code in its own copy; ").b("every change is sent back as a patch").t(".").done(),
        Capability::WriteApproval => {
            Sentence::new(SummaryTopic::Capability).t("It can change code in its own copy; ").b("every change is sent only after you approve it").t(".").done()
        }
    });

    // What it can see.
    let sc = &s.scope;
    let mut see = Sentence::new(SummaryTopic::Scope).t("The agent sees ");
    see = match facts.files {
        Some(n) => see.b(plural(n, "file", "files")),
        None => see.t("the files"),
    };
    see = see.t(format!(" in `{}`: ", repo_name(&sc.repo)));
    see = if sc.paths.is_empty() { see.t("all tracked files") } else { see.t(code_list(&sc.paths)) };
    if !sc.exclude.is_empty() {
        let n = sc.exclude.len() as u32;
        see = see.t(", ").b(format!("without {}", plural(n, "sensitive file", "sensitive files"))).t(format!(" ({})", code_list(&sc.exclude)));
    }
    see = see.t(".");
    if sc.uncommitted {
        see = see.t(" ").b("Uncommitted changes included").t(".");
    }
    see = if sc.history { see.t(" ").b("With git history").t(".") } else { see.t(" No git history.") };
    out.push(see.done());

    // Start.
    out.push(match &s.start {
        Start::Fresh => Sentence::new(SummaryTopic::Start).t("It starts ").b("fresh").t(".").done(),
        Start::Summary { .. } => Sentence::new(SummaryTopic::Start).t("It starts ").b("from the summary you approved").t(".").done(),
    });

    // Secrets and access.
    let acc = &s.access;
    let mut access = Sentence::new(SummaryTopic::Access).t("It has ");
    access = if acc.env.is_empty() {
        access.b("none of your tokens or logins")
    } else {
        access.b(format!("{} of your environment variables", acc.env.len())).t(format!(" ({})", code_list(&acc.env)))
    };
    access = access.t(", and ");
    access = if acc.network.allow.is_empty() {
        access.b("no network")
    } else {
        access.b(format!("network only to {}", and_list(&acc.network.allow)))
    };
    access = match &acc.programs {
        Some(Programs::Allow(list)) if !list.is_empty() => access.t(format!("; it may run only {}", code_list(list))),
        Some(Programs::Deny(list)) if !list.is_empty() => access.t(format!("; it may not run {}", code_list(list))),
        _ => access,
    };
    out.push(access.t(".").done());

    // Account.
    let kind = if facts.kind_name.is_empty() { "agent" } else { facts.kind_name.as_str() };
    out.push(match facts.account.as_deref().or(acc.account.as_deref()) {
        Some(name) => Sentence::new(SummaryTopic::Account).t(format!("It runs on your {kind} account \"{name}\".")).done(),
        None => Sentence::new(SummaryTopic::Account).t(format!("It runs on your own {kind} login.")).done(),
    });

    // Limits.
    let l = &s.limits;
    out.push(
        Sentence::new(SummaryTopic::Limits)
            .t(format!(
                "Each person: {} a day; {} at once.",
                plural(l.per_caller.messages_per_day, "message", "messages"),
                plural(l.sessions_running, "session", "sessions")
            ))
            .done(),
    );

    // Soft.
    let soft = s.off_limits.trim();
    if !soft.is_empty() {
        out.push(Sentence::new(SummaryTopic::Soft).b("Not guaranteed:").t(format!(" \"{soft}\".")).done());
    }

    let text = out.iter().map(|x| x.text()).collect::<Vec<_>>().join(" ");
    ShareSummary { sentences: out, text }
}

#[cfg(test)]
mod tests {
    use pitwall_proto::collab::{Audience, Identity, Scope, ShareState, TeamId};

    use super::*;

    /// §5.4's example share.
    fn example() -> (ShareSettings, Facts) {
        let mut s = ShareSettings {
            agent: "a1".into(),
            slug: "api-expert".into(),
            name: "API expert".into(),
            description: String::new(),
            state: ShareState::Draft,
            capability: Capability::WriteApproval,
            audience: Audience {
                people: vec![Identity::new("tailscale", "bob@example.com"), Identity::new("tailscale", "carol@example.com")],
                teams: vec![TeamId("t_9f2c".into())],
                ..Default::default()
            },
            scope: Scope {
                repo: "~/code/api".into(),
                git_ref: "main".into(),
                paths: vec!["src/".into(), "docs/".into()],
                exclude: vec![".env".into(), "config/master.key".into(), "deploy/id_ed25519".into()],
                ..Default::default()
            },
            start: Start::Fresh,
            access: Default::default(),
            off_limits: "don't discuss pricing or the reorg".into(),
            limits: Default::default(),
        };
        s.access.account = Some("work".into());
        let facts = Facts { teams: vec![Some(("Platform".into(), 6))], files: Some(412), kind_name: "Claude Code".into(), account: None };
        (s, facts)
    }

    #[test]
    fn the_spec_example_word_for_word() {
        let (s, facts) = example();
        let sum = summarize(&s, &facts);
        assert_eq!(
            sum.text,
            "bob@example.com, carol@example.com and team Platform (6 people) can send messages. \
             It can change code in its own copy; every change is sent only after you approve it. \
             The agent sees 412 files in `api`: `src/`, `docs/`, without 3 sensitive files \
             (`.env`, `config/master.key`, `deploy/id_ed25519`). No git history. It starts fresh. \
             It has none of your tokens or logins, and no network. \
             It runs on your Claude Code account \"work\". \
             Each person: 20 messages a day; 2 sessions at once. \
             Not guaranteed: \"don't discuss pricing or the reorg\"."
        );
        // The bold parts are the spec's.
        let strong: Vec<String> = sum.sentences.iter().flat_map(|s| s.parts.iter().filter(|p| p.strong).map(|p| p.text.clone())).collect();
        assert_eq!(
            strong,
            [
                "bob@example.com, carol@example.com and team Platform (6 people)",
                "every change is sent only after you approve it",
                "412 files",
                "without 3 sensitive files",
                "fresh",
                "none of your tokens or logins",
                "no network",
                "Not guaranteed:",
            ]
        );
        assert_eq!(sum.sentences.last().unwrap().topic, SummaryTopic::Soft);
    }

    #[test]
    fn other_answers_other_fixed_sentences() {
        let (mut s, mut facts) = example();
        s.audience = Default::default();
        s.capability = Capability::Read;
        s.scope.paths.clear();
        s.scope.exclude.clear();
        s.scope.uncommitted = true;
        s.start = Start::Summary { text: "x".into(), approved_at: "t".into() };
        s.access.env = vec!["NPM_TOKEN".into()];
        s.access.network.allow = vec!["registry.npmjs.org".into()];
        s.access.programs = Some(Programs::Allow(vec!["git".into(), "rg".into()]));
        s.access.account = None;
        s.off_limits = "  ".into();
        s.limits.per_caller.messages_per_day = 1;
        s.limits.sessions_running = 1;
        facts.files = None;
        facts.teams.clear();
        let text = summarize(&s, &facts).text;
        assert_eq!(
            text,
            "Nobody yet can send messages: add people or a team. \
             It can read its own copy; it can't change any code. \
             The agent sees the files in `api`: all tracked files. Uncommitted changes included. No git history. \
             It starts from the summary you approved. \
             It has 1 of your environment variables (`NPM_TOKEN`), and network only to registry.npmjs.org; it may run only `git`, `rg`. \
             It runs on your own Claude Code login. \
             Each person: 1 message a day; 1 session at once."
        );
        s.audience.networks = vec![pitwall_proto::collab::NetworkId::new("tailscale", "example.ts.net")];
        assert!(summarize(&s, &facts).text.starts_with("everyone on example.ts.net can send messages."));
        assert_eq!(and_list(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }
}
