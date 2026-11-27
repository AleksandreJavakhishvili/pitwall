//! The gallery's other pages: the form controls, feedback, layout, data
//! display and charts (docs/spec/tile-sdk/ui.md §3.10), each with working
//! state. The docs include each demo by its `docs:start gallery-<name>`
//! mark, and the app's screenshot test opens a view per component with
//! the content `{"component": "<name>"}` ([`Widgets::demo`]).

use pitwall_extension::prelude::*;

use crate::Hello;

/// The pages and their components, in the gallery's order.
pub const PAGES: &[(&str, &str, &[&str])] = &[
    ("forms", "Form controls", &["checkbox", "switch", "radio-group", "segmented", "slider", "number-field", "search-field", "form-row", "disabled"]),
    ("feedback", "Feedback", &["progress", "spinner", "skeleton", "empty-state", "banner", "badge", "chip", "kbd", "tooltip"]),
    ("layout", "Layout", &["split", "disclosure", "card", "divider", "breadcrumbs", "toolbar-row", "popover", "confirm-sheet"]),
    ("data", "Data display", &["key-values", "avatar", "relative-time", "copyable", "link"]),
    ("charts", "Charts", &["sparkline", "bar-chart", "line-chart", "status-timeline"]),
];

pub struct Widgets {
    // Form controls.
    only_failures: bool,
    all_branches: Option<bool>,
    notify: bool,
    scope: String,
    range: String,
    volume: f64,
    retries: Option<f64>,
    query: String,
    branch: String,
    // Feedback.
    banner_shown: bool,
    tags: Vec<&'static str>,
    flaky_only: bool,
    // Layout.
    env_open: bool,
    pub popover_open: bool,
    pub asking: bool,
    deleted: u32,
    path: Vec<&'static str>,
    split_share: Option<f64>,
    /// What happened last.
    pub last: String,
}

impl Default for Widgets {
    fn default() -> Self {
        Widgets {
            only_failures: true,
            all_branches: None,
            notify: true,
            scope: "mine".into(),
            range: "week".into(),
            volume: 60.,
            retries: Some(2.),
            query: String::new(),
            branch: String::new(),
            banner_shown: true,
            tags: vec!["web", "api", "release"],
            flaky_only: false,
            env_open: true,
            popover_open: false,
            asking: false,
            deleted: 0,
            path: vec!["acme", "web", "runs", "412"],
            split_share: None,
            last: String::new(),
        }
    }
}

fn said(t: &mut Hello, what: impl Into<String>, cx: &mut Cx<Hello>) {
    t.widgets.last = what.into();
    cx.notify();
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl Widgets {
    /// Page `page`: its components, each under its name.
    pub fn page(&self, page: &str, cx: &mut Cx<Hello>) -> AnyElement {
        let Some((_, title, names)) = PAGES.iter().find(|(k, _, _)| *k == page) else { return text(format!("No page “{page}”")).into_any() };
        let mut col = v_flex().gap_4().child(text(*title).text_lg().font_semibold());
        for name in names.iter() {
            col = col.child(v_flex().id(*name).gap_1().child(text(*name).text_xs().text_color(Token::Text3)).child(self.demo(name, cx)));
        }
        let last = if self.last.is_empty() { "Nothing yet".to_string() } else { self.last.clone() };
        let share = self.split_share.map(|s| format!(" · split at {:.0} %", s * 100.)).unwrap_or_default();
        v_flex()
            .size_full()
            .child(div().overflow_y_scroll().id("page").flex_1().child(col.p(px(4.0))))
            .child(text(format!("Last: {last}{share} · deleted {}", self.deleted)).id("last").text_sm().text_color(Token::Text3))
            .into_any()
    }

    /// The demo of component `name`.
    pub fn demo(&self, name: &str, cx: &mut Cx<Hello>) -> AnyElement {
        match name {
            "checkbox" => {
                let some = self.all_branches;
                // docs:start gallery-checkbox
                let boxes = v_flex()
                    .gap_2()
                    .child(checkbox("Only failures").id("failures").checked(self.only_failures).on_change(cx.listener(|t: &mut Hello, on: &bool, cx| {
                        t.widgets.only_failures = *on;
                        cx.notify();
                    })))
                    // Some of a group on: a dash; a click turns them all on.
                    .child(checkbox("All branches").id("branches").checked(some == Some(true)).mixed(some.is_none()).on_change(cx.listener(|t: &mut Hello, on: &bool, cx| {
                        t.widgets.all_branches = Some(*on);
                        cx.notify();
                    })));
                // docs:end
                boxes.into_any()
            }
            "switch" => {
                // docs:start gallery-switch
                let notify = switch("Notify me when a run fails").id("notify").checked(self.notify).on_change(cx.listener(|t: &mut Hello, on: &bool, cx| {
                    t.widgets.notify = *on;
                    cx.notify();
                }));
                // docs:end
                notify.into_any()
            }
            "radio-group" => {
                // docs:start gallery-radio-group
                let scope = radio_group()
                    .id("scope")
                    .label("Which runs")
                    .option("all", "All runs")
                    .option("mine", "My runs")
                    .option("failed", "Failed runs")
                    .selected(self.scope.clone())
                    .on_change(cx.listener(|t: &mut Hello, key: &str, cx| {
                        t.widgets.scope = key.to_string();
                        cx.notify();
                    }));
                // docs:end
                scope.into_any()
            }
            "segmented" => {
                // docs:start gallery-segmented
                let range = segmented()
                    .id("range")
                    .label("Range")
                    .option("day", "Day")
                    .option("week", "Week")
                    .option("month", "Month")
                    .selected(self.range.clone())
                    .on_change(cx.listener(|t: &mut Hello, key: &str, cx| {
                        t.widgets.range = key.to_string();
                        cx.notify();
                    }));
                // docs:end
                range.into_any()
            }
            "slider" => {
                // docs:start gallery-slider
                let volume = slider(self.volume, 0.0..=100.0).id("volume").label("Volume").step(5.0).show_value().w(px(240.)).on_change(cx.listener(|t: &mut Hello, v: &f64, cx| {
                    t.widgets.volume = *v;
                    cx.notify();
                }));
                // docs:end
                volume.into_any()
            }
            "number-field" => {
                // docs:start gallery-number-field
                let retries = number_field(self.retries).id("retries").label("Retries").min(0.).max(10.).placeholder("None").w(px(140.)).on_change(cx.listener(|t: &mut Hello, v: &Option<f64>, cx| {
                    t.widgets.retries = *v;
                    cx.notify();
                }));
                // docs:end
                retries.into_any()
            }
            "search-field" => {
                let shown = ["main", "release/0.9", "fix/login-flicker", "feat/tiles"].into_iter().filter(|b| b.contains(self.query.as_str())).count();
                // docs:start gallery-search-field
                let search = search_field("query").placeholder("Filter branches").on_change(cx.listener(|t: &mut Hello, q: &str, cx| {
                    t.widgets.query = q.to_string();
                    cx.notify();
                }));
                // docs:end
                v_flex().gap_2().w(px(280.)).child(search).child(text(format!("{shown} of 4 branches")).id("shown").text_sm().text_color(Token::Text3)).into_any()
            }
            "form-row" => {
                let missing = self.branch.trim().is_empty();
                // docs:start gallery-form-row
                let branch = form_row("Branch")
                    .id("branch-row")
                    .required()
                    .help("Runs start from this branch")
                    .error(missing.then_some("Name a branch"))
                    .child(input("branch").placeholder("main").on_change(cx.listener(|t: &mut Hello, v: &str, cx| {
                        t.widgets.branch = v.to_string();
                        cx.notify();
                    })));
                let retries = form_row("Retries").id("retries-row").inline().child(number_field(self.retries).id("retries").min(0.).max(10.).w(px(120.)));
                // docs:end
                v_flex().gap_3().w(px(320.)).child(branch).child(retries).into_any()
            }
            "disabled" => {
                // docs:start gallery-disabled
                let off = v_flex()
                    .gap_2()
                    .child(checkbox("Can't change this").id("box").checked(true).disabled(true))
                    .child(switch("Turned off by your admin").id("switch").disabled(true))
                    .child(segmented().id("seg").option("a", "On").option("b", "Off").selected("a").disabled(true))
                    .child(slider(40., 0.0..=100.0).id("slider").disabled(true).w(px(200.)))
                    .child(number_field(Some(3.)).id("number").readonly(true).w(px(120.)))
                    .child(input("locked").value("Read only").disabled(true).w(px(200.)));
                // docs:end
                off.into_any()
            }
            "progress" => {
                // docs:start gallery-progress
                let bars = v_flex()
                    .gap_3()
                    .w(px(280.))
                    .child(progress(Some(0.62)).id("indexing").label("Indexing files"))
                    .child(progress(Some(1.)).id("uploaded").tone(Tone::Good).label("Uploaded"))
                    // Still going, with no share known.
                    .child(progress(None).id("waiting").label("Waiting for the runner"));
                // docs:end
                bars.into_any()
            }
            "spinner" => {
                // docs:start gallery-spinner
                let loading = h_flex().gap_2().items_center().child(spinner().id("spin").label("Loading")).child(text("Loading runs…").text_color(Token::Text3));
                // docs:end
                loading.into_any()
            }
            "skeleton" => {
                // docs:start gallery-skeleton
                let placeholder = h_flex().gap_3().items_start().w(px(300.)).child(skeleton(SkeletonShape::Circle).size(px(28.))).child(v_flex().flex_1().child(skeleton(SkeletonShape::Lines(3))));
                // docs:end
                placeholder.into_any()
            }
            "empty-state" => {
                // docs:start gallery-empty-state
                let empty = empty_state("No runs yet")
                    .id("empty")
                    .icon(Icon::Folder)
                    .text("Runs show here when CI starts one.")
                    .action("Refresh", cx.listener(|t: &mut Hello, _: &Click, cx| said(t, "Refreshed", cx)));
                // docs:end
                empty.h(px(200.)).w(px(360.)).into_any()
            }
            "banner" => {
                let mut col = v_flex().gap_2().w(px(420.));
                // docs:start gallery-banner
                if self.banner_shown {
                    col = col.child(
                        banner(Tone::Bad, "The CI token was refused. Sign in again to load runs.")
                            .id("token")
                            .title("Can't load runs")
                            .action("Sign in", cx.listener(|t: &mut Hello, _: &Click, cx| said(t, "Signing in", cx)))
                            .on_dismiss(cx.listener(|t: &mut Hello, _: &Dismissed, cx| {
                                t.widgets.banner_shown = false;
                                cx.notify();
                            })),
                    );
                }
                col = col
                    .child(banner(Tone::Info, "Runs older than 90 days are archived.").id("info"))
                    .child(banner(Tone::Good, "Deployed to staging.").id("good"))
                    .child(banner(Tone::Warn, "Two runners are offline.").id("warn"));
                // docs:end
                col.into_any()
            }
            "badge" => {
                // docs:start gallery-badge
                let badges = h_flex()
                    .gap_2()
                    .child(badge("12"))
                    .child(badge("3 failing").tone(Tone::Bad))
                    .child(badge("passing").tone(Tone::Good))
                    .child(badge("flaky").tone(Tone::Warn))
                    .child(badge("beta").tone(Tone::Info));
                // docs:end
                badges.into_any()
            }
            "chip" => {
                // docs:start gallery-chip
                let mut chips = h_flex().gap_2().flex_wrap().w(px(360.)).child(chip("Flaky only").id("flaky").icon(Icon::Restart).selected(self.flaky_only).on_click(cx.listener(|t: &mut Hello, _: &Click, cx| {
                    t.widgets.flaky_only = !t.widgets.flaky_only;
                    cx.notify();
                })));
                for tag in self.tags.iter().copied() {
                    chips = chips.child(chip(tag).id(tag).on_remove(cx.listener(move |t: &mut Hello, _: &Dismissed, cx| {
                        t.widgets.tags.retain(|have| *have != tag);
                        cx.notify();
                    })));
                }
                let chips = chips.child(chip("main").id("main").tone(Tone::Good));
                // docs:end
                chips.into_any()
            }
            "kbd" => {
                // docs:start gallery-kbd
                let hint = h_flex().gap_2().items_center().child(text("Search").text_color(Token::Text3)).child(kbd("cmd-k")).child(text("or").text_color(Token::Text3)).child(kbd("/"));
                // docs:end
                hint.into_any()
            }
            "tooltip" => {
                // docs:start gallery-tooltip
                let tips = h_flex()
                    .gap_2()
                    .items_center()
                    .child(button("Deploy").id("deploy").tooltip("Deploys main to staging"))
                    .child(icon_button(Icon::Refresh, "Refresh runs").id("refresh"))
                    .child(badge("3").id("failed").tone(Tone::Bad).tooltip("3 runs failed today"));
                // docs:end
                tips.into_any()
            }
            "split" => {
                // docs:start gallery-split
                let panes = h_split()
                    .id("panes")
                    .initial(0.4)
                    .min_sizes(120., 160.)
                    .first(v_flex().p(px(12.)).gap_1().child(text("Runs").font_semibold()).child(text("#412 main").text_sm()).child(text("#411 fix/login").text_sm()))
                    .second(v_flex().p(px(12.)).gap_1().child(text("Run #412").font_semibold()).child(text("Passed in 3 min").text_color(Token::Text3)))
                    .on_resize(cx.listener(|t: &mut Hello, share: &f64, cx| {
                        t.widgets.split_share = Some(*share);
                        cx.notify();
                    }));
                // docs:end
                panes.h(px(150.)).w(px(440.)).border_1().border_color(Token::Line).rounded_md().into_any()
            }
            "disclosure" => {
                // docs:start gallery-disclosure
                let env = disclosure("Environment", self.env_open)
                    .id("env")
                    .detail("3 variables")
                    .on_toggle(cx.listener(|t: &mut Hello, open: &bool, cx| {
                        t.widgets.env_open = *open;
                        cx.notify();
                    }))
                    .child(key_values().entry("CI", "true").mono().entry("NODE_ENV", "test").mono().entry("RUST_LOG", "info").mono());
                // docs:end
                env.w(px(360.)).into_any()
            }
            "card" => {
                // docs:start gallery-card
                let run = card()
                    .id("run")
                    .title("Last run")
                    .detail("main")
                    .on_click(cx.listener(|t: &mut Hello, _: &Click, cx| said(t, "Opened run #412", cx)))
                    .child(h_flex().gap_2().items_center().child(badge("passed").tone(Tone::Good)).child(text("#412 · 3 min").text_color(Token::Text2)));
                // docs:end
                run.w(px(300.)).into_any()
            }
            "divider" => {
                // docs:start gallery-divider
                let lines = v_flex()
                    .gap_2()
                    .w(px(280.))
                    .child(text("Today"))
                    .child(divider())
                    .child(h_flex().gap_2().items_center().h(px(20.)).child(text("12 runs")).child(vertical_divider()).child(text("2 failed")));
                // docs:end
                lines.into_any()
            }
            "breadcrumbs" => {
                // docs:start gallery-breadcrumbs
                let crumbs = self
                    .path
                    .iter()
                    .fold(breadcrumbs().id("crumbs"), |b, level| b.crumb(*level, *level))
                    .on_pick(cx.listener(|t: &mut Hello, key: &str, cx| {
                        // Back up to the level clicked.
                        if let Some(at) = t.widgets.path.iter().position(|p| *p == key) {
                            t.widgets.path.truncate(at + 1);
                        }
                        cx.notify();
                    }));
                // docs:end
                crumbs.into_any()
            }
            "toolbar-row" => {
                // docs:start gallery-toolbar-row
                let bar = toolbar()
                    .id("bar")
                    .child(search_field("filter").placeholder("Filter").w(px(180.)))
                    .child(segmented().id("which").option("all", "All").option("failed", "Failed").selected("all"))
                    .child(div().flex_1())
                    .child(icon_button(Icon::Refresh, "Refresh").id("refresh"))
                    .child(button("New run").id("new").small().primary());
                // docs:end
                bar.w(px(520.)).into_any()
            }
            "popover" => {
                // docs:start gallery-popover
                let mut filters = div().id("filters").child(button("Filters").id("open").icon(Icon::ChevronDown).on_click(cx.listener(|t: &mut Hello, _: &Click, cx| {
                    t.widgets.popover_open = !t.widgets.popover_open;
                    cx.notify();
                })));
                if self.popover_open {
                    filters = filters.child(
                        popover()
                            .id("pop")
                            .width(240.)
                            .on_dismiss(cx.listener(|t: &mut Hello, _: &Dismissed, cx| {
                                t.widgets.popover_open = false;
                                cx.notify();
                            }))
                            .child(v_flex().gap_2().p(px(12.)).child(text("Show").font_semibold()).child(checkbox("Failed runs").id("failed").checked(true)).child(checkbox("Cancelled runs").id("cancelled"))),
                    );
                }
                // docs:end
                div().h(px(if self.popover_open { 150. } else { 40. })).child(filters).into_any()
            }
            "confirm-sheet" => {
                // docs:start gallery-confirm-sheet
                let mut notes = v_flex().id("notes").size_full().gap_2().p(px(12.)).child(button("Delete 3 notes…").id("delete").danger().on_click(cx.listener(|t: &mut Hello, _: &Click, cx| {
                    t.widgets.asking = true;
                    cx.notify();
                })));
                if self.asking {
                    notes = notes.child(
                        confirm_sheet("Delete 3 notes?", "Delete")
                            .id("ask")
                            .text("They can't be brought back.")
                            .danger()
                            .on_confirm(cx.listener(|t: &mut Hello, _: &Click, cx| {
                                t.widgets.deleted += 3;
                                t.widgets.asking = false;
                                cx.notify();
                            }))
                            .on_cancel(cx.listener(|t: &mut Hello, _: &Dismissed, cx| {
                                t.widgets.asking = false;
                                cx.notify();
                            })),
                    );
                }
                // docs:end
                let deleted = text(format!("{} notes deleted", self.deleted)).id("deleted").text_sm().text_color(Token::Text3);
                v_flex().w(px(420.)).h(px(200.)).border_1().border_color(Token::Line).rounded_md().child(notes.child(deleted)).into_any()
            }
            "key-values" => {
                // docs:start gallery-key-values
                let facts = key_values().entry("Branch", "main").entry("Commit", "4f2a9c1e").mono().copyable().entry("Runner", "macos-14 (arm64)").entry("Started by", "Ada Lovelace");
                // docs:end
                facts.id("facts").w(px(320.)).into_any()
            }
            "avatar" => {
                // docs:start gallery-avatar
                let people = h_flex().gap_2().items_center().child(avatar("Ada Lovelace").small()).child(avatar("Grace Hopper")).child(avatar("ci-bot")).child(avatar("Linus Torvalds").large());
                // docs:end
                people.into_any()
            }
            "relative-time" => {
                let now = now_ms();
                // docs:start gallery-relative-time
                let times = v_flex()
                    .gap_1()
                    .child(h_flex().gap_1().child(text("Started")).child(relative_time_ms(now - 5 * 60_000).id("started")))
                    .child(h_flex().gap_1().child(text("Last failure")).child(relative_time_ms(now - 26 * 3_600_000).id("failed")));
                // docs:end
                times.into_any()
            }
            "copyable" => {
                // docs:start gallery-copyable
                let copies = v_flex().gap_2().child(copyable("4f2a9c1e7b").id("sha").mono()).child(copyable("https://ci.example.com/runs/412").id("url"));
                // docs:end
                copies.into_any()
            }
            "link" => {
                // docs:start gallery-link
                let links = v_flex()
                    .gap_2()
                    .child(link("Open run #412 in CI").id("web").url("https://ci.example.com/runs/412"))
                    // No address: the click is the tile's.
                    .child(link("Show all runs").id("all").on_click(cx.listener(|t: &mut Hello, _: &Click, cx| said(t, "All runs", cx))));
                // docs:end
                links.into_any()
            }
            "sparkline" => {
                // docs:start gallery-sparkline
                let trend = h_flex()
                    .gap_3()
                    .items_center()
                    .child(text("Build time").text_color(Token::Text3))
                    .child(sparkline([4.1, 3.8, 4.4, 5.0, 4.2, 3.9, 3.6, 3.7, 3.2, 3.4]).id("trend").label("Build time, last 10 runs").color(Token::Good).filled())
                    .child(text("3.4 min").font_mono());
                // docs:end
                trend.into_any()
            }
            "bar-chart" => {
                // docs:start gallery-bar-chart
                let runs = bar_chart(["Mon", "Tue", "Wed", "Thu", "Fri"])
                    .id("runs")
                    .label("Runs this week")
                    .series_colored("Passed", [12., 18., 9., 14., 11.], Token::Good)
                    .series_colored("Failed", [1., 0., 3., 2., 1.], Token::Bad)
                    .stacked()
                    .unit("runs")
                    .h(px(180.));
                // docs:end
                runs.w(px(420.)).into_any()
            }
            "line-chart" => {
                // docs:start gallery-line-chart
                let latency = line_chart(["1 Mar", "2 Mar", "3 Mar", "4 Mar", "5 Mar", "6 Mar", "7 Mar"])
                    .id("latency")
                    .label("Response time")
                    .series("p50", [210., 190., 230., 205., 180., 175., 182.])
                    .series("p95", [480., 520., 610., 450., 430., 470., 440.])
                    .unit("ms")
                    .h(px(200.));
                // docs:end
                latency.w(px(460.)).into_any()
            }
            "status-timeline" => {
                let runs = [(Tone::Good, 3.0), (Tone::Good, 3.2), (Tone::Bad, 1.1), (Tone::Good, 3.4), (Tone::Warn, 6.0), (Tone::Good, 3.1), (Tone::Good, 2.9), (Tone::Bad, 0.8), (Tone::Good, 3.3)];
                // docs:start gallery-status-timeline
                let timeline = runs
                    .iter()
                    .enumerate()
                    .fold(status_timeline().id("history").label("Last 9 runs"), |line, (i, (tone, mins))| {
                        let run = 404 + i;
                        let what = match tone {
                            Tone::Good => "passed",
                            Tone::Bad => "failed",
                            _ => "was slow",
                        };
                        line.segment_weighted(run.to_string(), format!("#{run} {what} in {mins} min"), Some(*tone), *mins as f32)
                    })
                    .on_pick(cx.listener(|t: &mut Hello, run: &str, cx| said(t, format!("Run #{run}"), cx)));
                // docs:end
                timeline.h(px(20.)).w(px(420.)).into_any()
            }
            other => text(format!("No demo “{other}”")).into_any(),
        }
    }
}
