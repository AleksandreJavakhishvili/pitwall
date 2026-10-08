//! Draft review comments per agent, kept for the app session so leaving
//! Review doesn't lose them (Tauri: `src/components/review/comments.ts`).
//! Nothing is sent until the user confirms the composed prompt.

use std::collections::HashMap;

use gpui::{App, Global};

use super::model::{sort_comments, Comment};

#[derive(Default)]
pub struct Comments {
    by_agent: HashMap<String, Vec<Comment>>,
    seq: u64,
}

impl Global for Comments {}

impl Comments {
    /// An agent's comments, in file/line order.
    pub fn for_agent(cx: &App, agent: &str) -> Vec<Comment> {
        cx.try_global::<Comments>()
            .and_then(|c| c.by_agent.get(agent))
            .map(|l| sort_comments(l))
            .unwrap_or_default()
    }

    pub fn add(cx: &mut App, agent: &str, path: String, line: usize, text: String) {
        let c = cx.default_global::<Comments>();
        c.seq += 1;
        let id = c.seq;
        c.by_agent
            .entry(agent.to_string())
            .or_default()
            .push(Comment {
                id,
                path,
                line,
                text,
            });
    }

    pub fn remove(cx: &mut App, agent: &str, id: u64) {
        if let Some(l) = cx.default_global::<Comments>().by_agent.get_mut(agent) {
            l.retain(|c| c.id != id);
        }
    }

    pub fn clear(cx: &mut App, agent: &str) {
        cx.default_global::<Comments>().by_agent.remove(agent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn comments_are_kept_per_agent(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            Comments::add(cx, "a", "z.rs".into(), 4, "later".into());
            Comments::add(cx, "a", "b.rs".into(), 9, "first".into());
            Comments::add(cx, "b", "x.rs".into(), 1, "other".into());
            let a = Comments::for_agent(cx, "a");
            assert_eq!(
                a.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
                ["b.rs", "z.rs"]
            );
            Comments::remove(cx, "a", a[0].id);
            assert_eq!(Comments::for_agent(cx, "a").len(), 1);
            Comments::clear(cx, "a");
            assert!(Comments::for_agent(cx, "a").is_empty());
            assert_eq!(Comments::for_agent(cx, "b").len(), 1);
        });
    }
}
