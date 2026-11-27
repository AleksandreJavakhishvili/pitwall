//! The files tile: the host's test fixture for `files` (files.wit),
//! written with the guest SDK. Each button (under the key `do`) makes one
//! call on the view's project (under `data`, on the extension's own files
//! folder) and shows what came back in the text keyed `status`; the
//! view's project is in `project`, and what its watch heard in `changed`.
//! The host's, the harness's and the app's tests click these by key.

use pitwall_extension::files::{self, Root};
use pitwall_extension::prelude::*;

#[derive(Default)]
struct Files {
    status: String,
    changed: String,
    watch: Option<files::WatchId>,
}

fn said<T, E: std::fmt::Display>(what: &str, r: Result<T, E>, ok: impl FnOnce(T) -> String) -> String {
    match r {
        Ok(v) => format!("{what}: {}", ok(v)),
        Err(e) => format!("{what}: {e}"),
    }
}

fn project_line(view: &str) -> String {
    match files::project(view) {
        Some(p) => format!("in {} at {}", p.name, p.folder),
        None => "in no project".into(),
    }
}

fn paths(v: Vec<files::Entry>) -> String {
    v.iter().map(|e| if e.is_folder() { format!("{}/", e.path) } else { e.path.clone() }).collect::<Vec<_>>().join(", ")
}

type Act = fn(&mut Files, &Root);

const ACTS: &[(&str, &str, Act)] = &[
    ("list", "List docs", |t, r| t.status = said("list", files::list(r, "docs", true), paths)),
    ("list-root", "List all", |t, r| t.status = said("list", files::list(r, "", false), paths)),
    ("read", "Read", |t, r| t.status = said("read", files::read_text(r, "docs/a.md"), |s| s)),
    ("stat", "Stat", |t, r| t.status = said("stat", files::stat(r, "docs/a.md"), |e| format!("{} {} bytes", e.path, e.size))),
    ("write", "Write", |t, r| t.status = said("write", files::write_text(r, "docs/new.md", "# New\n"), |_| "ok".into())),
    ("create", "Create", |t, r| t.status = said("create", files::create(r, "docs/new.md", b"again"), |_| "ok".into())),
    ("mkdir", "Make folder", |t, r| t.status = said("mkdir", files::make_folder(r, "notes/2026"), |_| "ok".into())),
    ("rename", "Rename", |t, r| t.status = said("rename", files::rename(r, "docs/new.md", "docs/renamed.md"), |_| "ok".into())),
    ("delete", "Delete", |t, r| t.status = said("delete", files::delete(r, "docs/renamed.md"), |_| "ok".into())),
    ("up", "Go up", |t, r| t.status = said("up", files::read(r, "../secret.txt"), |b| format!("{} bytes", b.len()))),
    ("abs", "Absolute", |t, r| t.status = said("abs", files::read(r, "/etc/hosts"), |b| format!("{} bytes", b.len()))),
    ("git", "Git", |t, r| t.status = said("git", files::read(r, ".git/config"), |b| format!("{} bytes", b.len()))),
    ("link", "Link", |t, r| t.status = said("link", files::read(r, "out/secret.txt"), |b| format!("{} bytes", b.len()))),
    ("own", "Pitwall's", |t, r| t.status = said("own", files::write_text(r, ".pitwall.json", "{}"), |_| "ok".into())),
    ("big", "Too big", |t, r| t.status = said("big", files::write(r, "big.bin", &vec![0u8; (8 << 20) + 1]), |_| "ok".into())),
    ("watch", "Watch", |t, r| {
        t.status = match files::watch(r, "") {
            Ok(id) => {
                t.watch = Some(id);
                "watching".into()
            }
            Err(e) => format!("watch: {e}"),
        }
    }),
    ("unwatch", "Unwatch", |t, _| {
        if let Some(id) = t.watch.take() {
            files::unwatch(id);
        }
        t.status = "not watching".into();
    }),
];

impl Tile for Files {
    fn new(_cx: &mut Cx<Self>) -> Self {
        Files::default()
    }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        let mut actions = h_flex().id("do").gap_1();
        for (key, label, act) in ACTS {
            let (view, act) = (view.to_string(), *act);
            actions = actions.child(button(label.to_string()).id(key.to_string()).small().on_click(cx.listener(move |t: &mut Files, _, cx| {
                act(t, &Root::project(&view));
                cx.notify();
            })));
        }
        let data = h_flex()
            .id("data")
            .gap_1()
            .child(button("Keep").id("write").small().on_click(cx.listener(|t: &mut Files, _, cx| {
                t.status = said("data write", files::write_text(&Root::Data, "cache/kept.txt", "kept"), |_| "ok".into());
                cx.notify();
            })))
            .child(button("Read kept").id("read").small().on_click(cx.listener(|t: &mut Files, _, cx| {
                t.status = said("data read", files::read_text(&Root::Data, "cache/kept.txt"), |s| s);
                cx.notify();
            })))
            .child(button("Drop kept").id("delete").small().on_click(cx.listener(|t: &mut Files, _, cx| {
                t.status = said("data delete", files::delete(&Root::Data, "cache"), |_| "ok".into());
                cx.notify();
            })));
        v_flex()
            .gap_2()
            .p_3()
            .child(text(project_line(view)).id("project").text_color(Token::Text))
            .child(text(self.status.clone()).id("status").text_color(Token::Text))
            .child(text(self.changed.clone()).id("changed").text_color(Token::Text2))
            .child(actions)
            .child(data)
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        match event {
            Event::FilesChanged { paths, .. } => self.changed = format!("changed: {}", paths.join(", ")),
            Event::ProjectChanged(view) => {
                self.watch = None;
                self.status = format!("moved {}", project_line(view));
            }
            _ => {}
        }
        cx.notify();
    }
}

pitwall_extension::export!(Files);
