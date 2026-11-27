//! The services tile: the host's test fixture for the minimum's services
//! (platform-services.md §14.2), written with the guest SDK. Each button
//! (under the key `do`) calls one service and shows what came back in the
//! text keyed `status`; the database's notes are counted in `notes`. The
//! toolbar popover (`toolbar:popover`) has a Close button. The harness's
//! tests (crates/pitwall-extension-test/tests/services.rs) click these by
//! key.

use pitwall_extension::prelude::*;
use pitwall_extension::{net, notify, service, show, sql, toolbar};

#[derive(Default)]
struct Services {
    status: String,
    notes: i64,
    badge: u32,
}

const MIGRATIONS: &[&str] = &[
    "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL);",
    "ALTER TABLE notes ADD COLUMN done INTEGER NOT NULL DEFAULT 0;",
];

fn count() -> i64 {
    sql::query("SELECT count(*) AS n FROM notes", ()).ok().and_then(|r| r.iter().next().and_then(|row| row.get::<i64>("n").ok())).unwrap_or(-1)
}

fn said<E: std::fmt::Display>(what: &str, r: Result<(), E>) -> String {
    match r {
        Ok(()) => format!("{what}: ok"),
        Err(e) => format!("{what}: {e}"),
    }
}

impl Services {
    fn fetch(&mut self, url: String, cx: &mut Cx<Self>) {
        let sent = cx.fetch(net::Request::get(url).header("Accept", "text/plain"), |t, answer, cx| {
            t.status = match answer {
                Ok(r) => format!("fetched {}: {}", r.status, r.text()),
                Err(e) => format!("fetch failed: {e}"),
            };
            cx.notify();
        });
        self.status = match sent {
            Ok(_) => "fetching".into(),
            Err(e) => format!("refused: {e}"),
        };
        cx.notify();
    }
}

impl Tile for Services {
    fn new(_cx: &mut Cx<Self>) -> Self {
        let status = match sql::migrate(MIGRATIONS) {
            Ok(v) => format!("schema {v}"),
            Err(e) => format!("migrate failed: {e}"),
        };
        Services { status, notes: count(), badge: 0 }
    }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        if view == toolbar::POPOVER_VIEW {
            return v_flex()
                .gap_2()
                .p_3()
                .child(text("Popover").id("title").text_color(Token::Text))
                .child(text(self.status.clone()).id("status").text_color(Token::Text2))
                .child(button("Close").id("close").small().on_click(cx.listener(|_: &mut Services, _, _| toolbar::close_popover())))
                .into_any();
        }
        let b = |cx: &mut Cx<Self>, key: &str, label: &str, f: fn(&mut Services, &mut Cx<Services>)| {
            button(label.to_string()).id(key.to_string()).small().on_click(cx.listener(move |t: &mut Services, _, cx| {
                f(t, cx);
                cx.notify();
            }))
        };
        let actions = h_flex()
            .id("do")
            .gap_1()
            .child(b(cx, "fetch", "Fetch", |t, cx| t.fetch("https://api.example.com/items?x=1".into(), cx)))
            .child(b(cx, "fetch-other", "Fetch elsewhere", |t, cx| t.fetch("https://other.example.org/".into(), cx)))
            .child(b(cx, "fetch-own", "Ask the server", |t, cx| match service::url("/api/ping") {
                Some(u) => t.fetch(u, cx),
                None => t.status = "no server address".into(),
            }))
            .child(b(cx, "add", "Add a note", |t, _| {
                t.status = match sql::execute("INSERT INTO notes (title) VALUES (?)", ("a note",)) {
                    Ok(n) => format!("added {n}"),
                    Err(e) => format!("add failed: {e}"),
                };
                t.notes = count();
            }))
            .child(b(cx, "tx", "Clear in a transaction", |t, _| {
                let r = sql::transaction(|| {
                    sql::execute("DELETE FROM notes", ())?;
                    sql::execute("INSERT INTO nope VALUES (1)", ())
                });
                t.status = said("transaction", r.map(|_| ()));
                t.notes = count();
            }))
            .child(b(cx, "open-tx", "Leave a transaction open", |t, _| {
                t.status = said("begin", sql::execute("BEGIN", ()).map(|_| ()));
                let _ = sql::execute("DELETE FROM notes", ());
            }))
            .child(b(cx, "notify", "Notify", |t, _| t.status = said("notify", notify::send("Build failed", "api: 3 tests failed"))))
            .child(b(cx, "open-file", "Open a file", |t, _| t.status = said("file", show::file("/tmp/project/src/main.rs", Some(12)))))
            .child(b(cx, "open-web", "Open a page", |t, _| t.status = said("web", show::web("https://example.com/docs"))))
            .child(b(cx, "badge", "Badge", |t, _| {
                t.badge += 1;
                toolbar::set_badge(toolbar::Badge::Count(t.badge), toolbar::Tone::Attention);
                toolbar::set_tooltip(&format!("{} unread", t.badge));
                t.status = format!("badge {}", t.badge);
            }))
            .child(b(cx, "hide", "Hide the button", |t, _| {
                toolbar::set_shown(false);
                t.status = "hidden".into();
            }))
            .child(b(cx, "restart", "Restart the server", |t, _| {
                service::restart();
                t.status = "restart asked".into();
            }));
        v_flex()
            .gap_2()
            .p_3()
            .child(text(self.status.clone()).id("status").text_color(Token::Text))
            .child(text(format!("Notes: {}", self.notes)).id("notes").text_color(Token::Text2))
            .child(actions)
            .into_any()
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        match event {
            Event::ToolbarClicked { project } => self.status = format!("clicked {}", project.as_deref().unwrap_or("-")),
            Event::ServiceChanged(info) => self.status = format!("service {:?} {}", info.state, info.address.as_deref().unwrap_or("-")),
            _ => {}
        }
        cx.notify();
    }
}

pitwall_extension::export!(Services);
