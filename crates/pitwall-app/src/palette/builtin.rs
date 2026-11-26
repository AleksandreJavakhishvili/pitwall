//! The React palette's rows (`CommandPalette.tsx`), in its order: go to
//! and remove each agent, next blocked, new agent, terminals, Wall and
//! Review, files, window, layout, density, panels, Settings, quit
//! and theme.

use gpui::{App, Context, SharedString, Window};
use pitwall_proto::Status;

use crate::agents::status_word;
use crate::explorer::Explorer;
use crate::main_screen::tree::Preset;
use crate::main_screen::MainScreen;
use crate::menu::{OpenSettings, Quit, QuitAndStopAgents};
use crate::theme::{Density, ThemePref};

use super::matching::looks_like_path;
use super::registry::{rank, Command, Hint, Lead, PaletteContext, Span};

/// Run `f` on the window's main screen.
fn on_screen(
    pc: &PaletteContext,
    f: impl Fn(&mut MainScreen, &mut Window, &mut Context<MainScreen>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let screen = pc.screen.clone();
    move |window, cx| {
        if let Some(s) = screen.as_ref().and_then(|s| s.upgrade()) {
            s.update(cx, |s, cx| f(s, window, cx));
        }
    }
}

/// Run `f` on the window's explorer.
fn on_explorer(
    pc: &PaletteContext,
    f: impl Fn(&mut Explorer, &mut Window, &mut Context<Explorer>) + 'static,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let explorer = pc.explorer.clone();
    move |window, cx| {
        if let Some(e) = explorer.as_ref().and_then(|e| e.upgrade()) {
            e.update(cx, |e, cx| f(e, window, cx));
        }
    }
}

/// `PRESETS` ids, as typed in the search text.
fn preset_key(p: Preset) -> &'static str {
    match p {
        Preset::One => "1",
        Preset::Two => "2",
        Preset::Three => "3",
        Preset::G2x2 => "2x2",
        Preset::G3x2 => "3x2",
        Preset::G3x3 => "3x3",
        Preset::G4x3 => "4x3",
        Preset::G4x4 => "4x4",
        Preset::Auto => "auto",
    }
}

fn density_key(d: Density) -> &'static str {
    match d {
        Density::Comfortable => "comfortable",
        Density::Compact => "compact",
        Density::Dense => "dense",
    }
}

fn theme_key(t: ThemePref) -> &'static str {
    match t {
        ThemePref::System => "system",
        ThemePref::Dark => "dark",
        ThemePref::Light => "light",
    }
}

fn s(text: impl Into<SharedString>) -> Span {
    Span::Text(text.into())
}

fn sub(text: impl Into<SharedString>) -> Span {
    Span::Sub(text.into())
}

/// Every built-in row (the registry sorts them by rank).
pub fn commands(pc: &PaletteContext, cx: &App) -> Vec<Command> {
    let st = pc.state;
    let q = pc.query;
    let mut out = vec![];

    // Go to / remove each agent.
    for a in &st.agents {
        let id = a.id.clone();
        let mut go = Command::new(
            format!("go-{}", a.id),
            a.name.clone(),
            format!(
                "go jump {} {} {}",
                a.name,
                status_key(a.status),
                a.project_display
            ),
            on_screen(pc, move |s, w, cx| s.show_agent(&id, w, cx)),
        )
        .label(vec![
            s(a.name.clone()),
            sub(format!("{} · {}", status_word(a.status), a.project_display)),
        ])
        .lead(Lead::Status(a.status))
        .rank(rank::AGENTS);
        if st.selected.as_deref() == Some(a.id.as_str()) {
            go = go.hint(Hint::Sub("current".into()));
        }
        out.push(go);
    }
    for a in &st.agents {
        let id = a.id.clone();
        out.push(
            Command::new(
                format!("remove-{}", a.id),
                "",
                format!("remove delete close agent {} {}", a.name, a.project_display),
                on_screen(pc, move |s, _, cx| s.cmd_remove(&id, cx)),
            )
            .label(vec![
                s(format!("Remove {}…", a.name)),
                sub(a.project_display.clone()),
            ])
            .glyph("✕")
            .rank(rank::REMOVE),
        );
    }
    if st.agents.iter().any(|a| a.status == Status::Blocked) {
        out.push(
            Command::new(
                "next-blocked",
                "Jump to next blocked",
                "jump next blocked needs you",
                on_screen(pc, |s, w, cx| s.cmd_next_blocked(w, cx)),
            )
            .lead(Lead::Warn("▲".into()))
            .keys("⌘J")
            .rank(rank::BLOCKED),
        );
    }
    out.push(
        Command::new(
            "new",
            "New agent",
            "new agent create start",
            on_screen(pc, |s, w, cx| s.cmd_new_agent(w, cx)),
        )
        .glyph("+")
        .keys("⌘N")
        .rank(rank::NEW),
    );

    // Terminals.
    if looks_like_path(q) {
        let path = q.trim().to_string();
        out.push(
            Command::new(
                "term-at",
                "",
                "",
                on_screen(pc, {
                    let path = path.clone();
                    move |s, w, cx| s.cmd_new_terminal(Some(path.clone()), w, cx)
                }),
            )
            .label(vec![s("Terminal at "), Span::Mono(path.into())])
            .icon("terminal")
            .pinned()
            .rank(rank::TERMINALS),
        );
    }
    out.push(
        Command::new(
            "term-here",
            "New terminal here",
            "new terminal shell here open",
            on_screen(pc, |s, w, cx| s.cmd_new_terminal(None, w, cx)),
        )
        .icon("terminal")
        .keys("⌘T")
        .rank(rank::TERMINALS),
    );
    out.push(
        Command::new(
            "term-choose",
            "New terminal in folder…",
            "new terminal shell folder path choose at",
            on_screen(pc, |s, w, cx| s.cmd_new_terminal_at(w, cx)),
        )
        .icon("terminal")
        .keys("⌘⇧T")
        .rank(rank::TERMINALS),
    );
    out.push(
        Command::new(
            "web-page",
            "Open web page…",
            "open web page browser url address site tile localhost",
            on_screen(pc, |s, w, cx| s.cmd_open_web_page(w, cx)),
        )
        .icon("browser")
        .keys("⌘L")
        .rank(rank::TERMINALS),
    );
    out.push(
        Command::new(
            "settings-tile",
            "Open Settings as a tile",
            "open settings as a tile preferences pane slot",
            on_screen(pc, |s, w, cx| s.open_settings_tile(None, w, cx)),
        )
        .icon("gear")
        .query_only()
        .rank(rank::TERMINALS),
    );
    // The text editor (docs/spec/editor.md).
    out.push(
        Command::new(
            "editor-new",
            "New file",
            "new file untitled text editor write create",
            on_screen(pc, |s, w, cx| s.new_file_in_editor(w, cx)),
        )
        .icon("file")
        .query_only()
        .rank(rank::TERMINALS),
    );
    out.push(
        Command::new(
            "editor-open",
            "Open file in editor…",
            "open file in editor text edit code",
            on_screen(pc, |s, w, cx| s.pick_files_for_editor(w, cx)),
        )
        .icon("file")
        .query_only()
        .rank(rank::TERMINALS),
    );
    if crate::main_screen::editor_tile::placed(cx) {
        out.push(
            Command::new(
                "editor-show",
                "Show the editor",
                "editor text files tabs show focus",
                on_screen(pc, |s, w, cx| s.open_editor_tile(w, cx)),
            )
            .icon("file")
            .query_only()
            .rank(rank::TERMINALS),
        );
    }
    // Device screens (docs/spec/device.md): each device adb and simctl
    // listed that can be shown now, from the list made when the palette
    // opened.
    if crate::main_screen::device_tile::enabled(cx) {
        let devices: Vec<_> = crate::main_screen::device_tile::known(cx).into_iter().filter(|d| d.state == crate::device::State::Ready).collect();
        for d in &devices {
            let key = d.key();
            let what = match (d.platform, d.virtual_device) {
                (crate::device::Platform::Ios, _) => "iOS Simulator",
                (crate::device::Platform::Android, true) => "Android emulator",
                (crate::device::Platform::Android, false) => "Android device",
            };
            out.push(
                Command::new(
                    format!("device-{key}"),
                    format!("Open device: {}", d.name),
                    format!("open device screen phone emulator simulator android ios preview {} {}", d.name, d.id),
                    on_screen(pc, move |s, w, cx| s.open_device(&key, w, cx)),
                )
                .label(vec![s(format!("Open device: {}", d.name)), sub(what)])
                .icon("phone")
                .query_only()
                .rank(rank::TERMINALS),
            );
        }
        if devices.is_empty() {
            out.push(
                Command::new(
                    "device-none",
                    "Open device…",
                    "open device screen phone emulator simulator android ios preview",
                    on_screen(pc, |s, _, cx| {
                        s.toast(crate::main_screen::strip::Toast::info("No device to show", Some("Start an Android emulator, connect a phone with USB debugging, or boot an iOS Simulator, then open the palette again.".into())), cx)
                    }),
                )
                .icon("phone")
                .query_only()
                .rank(rank::TERMINALS),
            );
        }
    }
    // Each extension's inbox (the first one on ⌘I).
    for (i, m) in crate::extensions::inboxes(cx).into_iter().enumerate() {
        let id = m.id.clone();
        let cmd = Command::new(
            format!("inbox-{}", m.id),
            format!("Open {} inbox", m.name),
            format!("open inbox questions asks threads extension {}", m.name.to_lowercase()),
            on_screen(pc, move |s, w, cx| s.open_inbox(Some(&id), None, w, cx)),
        )
        .icon("inbox")
        .rank(rank::TERMINALS);
        out.push(if i == 0 { cmd.keys("⌘I") } else { cmd });
    }
    // Each running extension's views and commands (labelled with its
    // name; no keys of their own).
    let focused = pc.screen.as_ref().and_then(|s| s.upgrade()).and_then(|s| s.read(cx).focused_agent_id());
    for m in crate::extensions::running(cx) {
        for v in &m.contributes.views {
            let (ext, view) = (m.id.clone(), v.id.clone());
            let icon = v.icon.as_deref().and_then(crate::kit::icons::known).or_else(|| crate::kit::icons::known(crate::extensions::icon_of(&m))).unwrap_or("extension");
            out.push(
                Command::new(format!("view-{}/{}", m.id, v.id), format!("Open {} ({})", v.title, m.name), format!("open view {} {} extension", v.title.to_lowercase(), m.name.to_lowercase()), on_screen(pc, move |s, w, cx| s.open_view(&ext, &view, None, w, cx)))
                    .icon(icon)
                    .rank(rank::TERMINALS),
            );
        }
        for w in &m.contributes.web_views {
            let (ext, view) = (m.id.clone(), w.id.clone());
            let icon = w.icon.as_deref().and_then(crate::kit::icons::known).unwrap_or("browser");
            out.push(
                Command::new(format!("web-view-{}/{}", m.id, w.id), format!("Open {} ({})", w.title, m.name), format!("open web page {} {} extension", w.title.to_lowercase(), m.name.to_lowercase()), on_screen(pc, move |s, _, cx| s.open_ext_web(&ext, &view, cx)))
                    .icon(icon)
                    .rank(rank::TERMINALS),
            );
        }
        let reads_agents = crate::extensions::calls::effective(&m, &crate::ui_state::get(cx).extension(&m.id).grants).iter().any(|g| g == pitwall_core::extensions::permissions::AGENTS_READ);
        for c in &m.contributes.commands {
            let for_agent = c.context == pitwall_core::extensions::CommandContext::Agent;
            if for_agent && focused.is_none() {
                continue;
            }
            let (ext, id, title) = (m.id.clone(), c.id.clone(), c.title.clone());
            out.push(
                Command::new(format!("ext-command-{}-{}", m.id, c.id), format!("{}: {}", m.name, c.title), format!("{} {} extension command", m.name.to_lowercase(), c.title.to_lowercase()), on_screen(pc, move |s, _, cx| s.extension_command(&ext, &id, &title, for_agent && reads_agents, cx)))
                    .icon(crate::kit::icons::known(crate::extensions::icon_of(&m)).unwrap_or("extension"))
                    .rank(rank::TERMINALS),
            );
        }
    }
    for (path, display) in &st.projects {
        let p = path.clone();
        out.push(
            Command::new(
                format!("term-in-{path}"),
                "",
                format!("terminal shell in {display} {path}"),
                on_screen(pc, move |s, w, cx| {
                    s.cmd_new_terminal(Some(p.clone()), w, cx)
                }),
            )
            .label(vec![s(format!("Terminal in {display}")), sub(path.clone())])
            .icon("terminal")
            .query_only()
            .rank(rank::TERMINALS),
        );
    }

    // Views.
    out.push(
        Command::new(
            "wall",
            "Toggle Wall (all terminals)",
            "wall overview all terminals expose",
            on_screen(pc, |s, _, cx| s.cmd_toggle_wall(cx)),
        )
        .glyph("▦")
        .keys("⌘E")
        .rank(rank::VIEWS),
    );
    out.push(
        Command::new(
            "review",
            "Review changes (diffs, comments, merge)",
            "review changes diff merge commit comments",
            on_screen(pc, |s, _, cx| s.cmd_toggle_review(cx)),
        )
        .glyph("±")
        .keys("⌘R")
        .rank(rank::VIEWS),
    );

    // Files: only for an agent whose files can be read.
    if pc.explorer.is_some() && pc.selected().is_some_and(|a| a.caps.explorer) {
        out.push(
            Command::new(
                "quick-open",
                "Go to file…",
                "go to file open quick find path explorer",
                on_explorer(pc, |e, w, cx| e.go_to_file_with("", w, cx)),
            )
            .icon("file")
            .keys("⌘P")
            .rank(rank::FILES),
        );
        out.push(
            Command::new(
                "search-files",
                "Search in files",
                "search find grep text in files explorer",
                on_explorer(pc, |e, w, cx| e.search_for("", w, cx)),
            )
            .icon("search")
            .keys("⌘⇧F")
            .rank(rank::FILES),
        );
        out.push(
            Command::new(
                "browse-files",
                "Browse files (read-only)",
                "browse files explorer tree folder code view read",
                on_explorer(pc, |e, w, cx| e.browse_files(w, cx)),
            )
            .icon("folder")
            .rank(rank::FILES),
        );
    }

    // The root of the focused files tile (else the focused agent's files
    // tile or window): its root menu, on this computer.
    let files_of = st.files.clone().or_else(|| st.selected.clone()).filter(|id| crate::explorer::show::can_switch_root_of(id, cx));
    if let Some(owner) = files_of {
        use crate::explorer::root_menu::Start;
        for (id, title, words, icon, start) in [
            ("files-open-folder", "Files: open folder…", "files open folder root viewer tree path directory", "folder", Start::OpenFolder),
            ("files-switch-worktree", "Files: switch worktree…", "files switch worktree root branch viewer tree", "branch", Start::Worktrees),
        ] {
            let owner = owner.clone();
            out.push(
                Command::new(id, title, words, move |_, cx| {
                    let owner = owner.clone();
                    // (After the palette's own update: the viewer may be in
                    // this very window.)
                    cx.defer(move |cx| crate::explorer::show::open_root_menu_for(&owner, start, cx));
                })
                .icon(icon)
                .rank(rank::FILES),
            );
        }
    }

    out.push(
        Command::new(
            "window",
            "Move space to new window",
            "move space new window monitor",
            on_screen(pc, |s, _, cx| s.cmd_move_space_to_window(cx)),
        )
        .glyph("⧉")
        .keys("⌘⇧N")
        .rank(rank::WINDOW),
    );
    for &p in &st.presets {
        out.push(
            Command::new(
                format!("preset-{}", preset_key(p)),
                format!("Tile layout: {}", p.label()),
                format!(
                    "layout tile preset {} {} split grid",
                    preset_key(p),
                    p.label()
                ),
                on_screen(pc, move |s, w, cx| s.cmd_preset(p, w, cx)),
            )
            .glyph("▤")
            .query_only()
            .rank(rank::LAYOUT),
        );
    }
    for d in Density::ALL {
        let c = d.cells();
        let size = format!("{}×{}", c.cols, c.rows);
        let key = density_key(d);
        out.push(
            Command::new(
                format!("density-{key}"),
                format!("Density: {} ({size}, default for all spaces)", d.label()),
                format!("density tiles small tile size {key} global default"),
                on_screen(pc, move |s, _, cx| s.cmd_density(Some(d), false, cx)),
            )
            .glyph("▦")
            .query_only()
            .rank(rank::DENSITY),
        );
        out.push(
            Command::new(
                format!("density-space-{key}"),
                format!("Density for this space: {} ({size})", d.label()),
                format!("density tiles small tile size {key} this space"),
                on_screen(pc, move |s, _, cx| s.cmd_density(Some(d), true, cx)),
            )
            .glyph("▦")
            .query_only()
            .rank(rank::DENSITY),
        );
    }
    out.push(
        Command::new(
            "density-space-default",
            "Density for this space: use default",
            "density tiles this space default reset",
            on_screen(pc, |s, _, cx| s.cmd_density(None, true, cx)),
        )
        .glyph("▦")
        .query_only()
        .rank(rank::DENSITY),
    );

    out.push(
        Command::new(
            "sidebar",
            "Toggle agents sidebar",
            "toggle sidebar agents panel hide show",
            on_screen(pc, |s, w, cx| s.cmd_toggle_sidebar(w, cx)),
        )
        .glyph("⇤")
        .keys("⌘B")
        .rank(rank::PANELS),
    );
    out.push(
        Command::new(
            "settings",
            "Settings",
            "settings codex hooks preferences",
            |w, cx| w.dispatch_action(Box::new(OpenSettings), cx),
        )
        .glyph("⚙")
        .keys("⌘,")
        .rank(rank::SETTINGS),
    );
    out.push(
        Command::new(
            "quit",
            "Quit Pitwall (agents keep running)",
            "quit exit close app leave running",
            |w, cx| w.dispatch_action(Box::new(Quit), cx),
        )
        .glyph("⏻")
        .rank(rank::QUIT),
    );
    out.push(
        Command::new(
            "quit-stop",
            "Quit and stop all agents",
            "quit exit stop all agents end",
            |w, cx| w.dispatch_action(Box::new(QuitAndStopAgents), cx),
        )
        .glyph("⏻")
        .rank(rank::QUIT),
    );
    for t in ThemePref::ALL {
        let key = theme_key(t);
        let extra = if t == ThemePref::System {
            "auto os macos follow".to_string()
        } else {
            format!("{key} mode")
        };
        out.push(
            Command::new(
                format!("theme-{key}"),
                format!("Theme: {}", t.label()),
                format!("theme appearance {key} {extra} color scheme"),
                move |w, cx| crate::settings::set_prefs(w, cx, |p| p.theme = t),
            )
            .glyph("◐")
            .query_only()
            .rank(rank::THEME),
        );
    }
    out
}

/// `AgentView.status` as the web UI writes it (the search text).
fn status_key(s: Status) -> &'static str {
    match s {
        Status::Working => "working",
        Status::Blocked => "blocked",
        Status::Done => "done",
        Status::Idle => "idle",
        Status::Unknown => "unknown",
        Status::Exited => "exited",
        Status::Stopped => "stopped",
    }
}
