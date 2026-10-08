//! Where onboarding's data comes from: the hosted engine's scan and macOS'
//! folder access (Tauri: `scan_environment`, `permissions_status`,
//! `open_privacy_settings`), behind a trait so tests and screenshots use a
//! fixture instead of the machine's real projects and processes.
//!
//! Debug builds and tests: `PITWALL_ONBOARDING_FIXTURE` forces a fixture —
//! `mock` (the browser mock's data, `mockOnboarding.ts`, folder access
//! granted), `mock-fda` (the same, starting at folder access), inline JSON
//! (`{…}`) or a JSON file's path. Release builds compile the fixture out.

use std::sync::Arc;

use gpui::{App, Global};

use pitwall_core::onboarding::scan::{ScanProgress, ScanResult};
use pitwall_core::permissions::PermissionsStatus;
use pitwall_core::Shared;

/// A scan step as it goes (the engine's own come through the store).
pub type Progress<'a> = &'a (dyn Fn(ScanProgress) + Sync);

pub trait OnboardingSource: Send + Sync + 'static {
    /// Run the scan (blocking). Sources other than the engine report steps
    /// through `progress`.
    fn scan(&self, progress: Progress) -> Result<ScanResult, String>;
    /// Folder access now (blocking).
    fn permissions(&self) -> PermissionsStatus;
    /// "Open Settings" on the folder access step.
    fn open_privacy(&self) -> Result<(), String>;
    /// A fixture: no real data, and the debug-only `$HOME` filter is off.
    fn is_fixture(&self) -> bool {
        false
    }
}

/// The hosted engine.
pub struct EngineSource(pub Option<Shared>);

impl OnboardingSource for EngineSource {
    fn scan(&self, _: Progress) -> Result<ScanResult, String> {
        let engine = self.0.as_ref().ok_or("Pitwall's engine isn't running")?;
        Ok(pitwall_core::onboarding::scan_environment(engine))
    }
    fn permissions(&self) -> PermissionsStatus {
        pitwall_core::permissions::status()
    }
    fn open_privacy(&self) -> Result<(), String> {
        super::super::permissions::open_full_disk_access()
    }
}

/// A source set by tests (wins over everything else).
#[derive(Clone)]
pub struct SourceOverride(pub Arc<dyn OnboardingSource>);

impl Global for SourceOverride {}

/// The source onboarding uses: a test's, a fixture (debug builds), else the
/// engine.
pub fn get(cx: &App, engine: Option<Shared>) -> Arc<dyn OnboardingSource> {
    if let Some(o) = cx.try_global::<SourceOverride>() {
        return o.0.clone();
    }
    #[cfg(any(test, debug_assertions))]
    if let Some(f) = fixture::from_env() {
        return Arc::new(f);
    }
    Arc::new(EngineSource(engine))
}

#[cfg(any(test, debug_assertions))]
pub mod fixture {
    //! Made-up scans for tests and screenshots (never in release builds).

    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use serde::Deserialize;

    use pitwall_core::model::CodexHooksStatus;
    use pitwall_core::onboarding::scan::{
        Conversation, RulesInfo, RunningAgent, ScanProgress, ScanResult, ScannedAgent,
        ScannedProject,
    };
    use pitwall_core::permissions::{Access, PermissionsStatus};
    use pitwall_proto::ScannedPlace;

    use super::{OnboardingSource, Progress};

    /// The browser mock's scan (`mockOnboarding.ts`).
    pub const MOCK: &str = include_str!("fixture_mock.json");

    /// How long the pretend user takes in System Settings
    /// (`MOCK_GRANT_AFTER_MS`).
    pub const GRANT_AFTER: Duration = Duration::from_secs(4);

    #[derive(Deserialize, Default)]
    #[serde(rename_all = "camelCase", default)]
    pub struct Fixture {
        /// "granted" | "denied"; absent: folder access doesn't apply.
        pub fda: Option<String>,
        /// The fixture's own "now" (Unix ms): its times move by the
        /// difference to the real now, so "10 min ago" stays 10 min ago.
        pub now: Option<u64>,
        pub steps: Vec<Step>,
        pub result: FxResult,
        #[serde(skip)]
        granted_at: Mutex<Option<Instant>>,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct Step {
        pub step: String,
        pub status: String,
        pub summary: Option<String>,
        /// How long the step "runs" first.
        pub ms: u64,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct FxResult {
        pub agents: Vec<FxAgent>,
        pub projects: Vec<FxProject>,
        pub conversations: Vec<FxConversation>,
        pub running: Vec<FxRunning>,
        pub places: Vec<ScannedPlace>,
        pub codex_hooks: Option<FxHooks>,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct FxAgent {
        pub kind: String,
        pub name: String,
        pub installed: bool,
        pub path: Option<String>,
        pub version: Option<String>,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct FxRules {
        pub rulesync: bool,
        pub claude_md: bool,
        pub agents_md: bool,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct FxProject {
        pub path: String,
        pub display: String,
        pub is_git: bool,
        pub last_used: Option<u64>,
        pub sources: Vec<String>,
        pub agent_history: bool,
        pub added: bool,
        pub rules: FxRules,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct FxConversation {
        pub kind: String,
        pub kind_name: String,
        pub session_id: String,
        pub project_path: String,
        pub project_display: String,
        pub title: String,
        pub last_used: u64,
        pub in_pitwall: bool,
        pub outside_project: bool,
        pub display_project: Option<String>,
        pub running_elsewhere: bool,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct FxRunning {
        pub pid: u32,
        pub kind: String,
        pub kind_name: String,
        pub cwd: Option<String>,
        pub cwd_display: Option<String>,
        pub session_id: Option<String>,
        pub title: Option<String>,
        pub in_pitwall: bool,
        pub outside_project: bool,
        pub display_project: Option<String>,
    }

    #[derive(Deserialize, Default, Clone)]
    #[serde(rename_all = "camelCase", default)]
    pub struct FxHooks {
        pub installed: bool,
        pub path: String,
    }

    /// `PITWALL_ONBOARDING_FIXTURE`, read (panics on a broken fixture: it
    /// is a developer's typo, better loud than real data).
    pub fn from_env() -> Option<Fixture> {
        let v = std::env::var("PITWALL_ONBOARDING_FIXTURE").ok()?;
        Some(parse_spec(&v).unwrap_or_else(|e| panic!("PITWALL_ONBOARDING_FIXTURE: {e}")))
    }

    /// `mock`, `mock-fda`, inline JSON or a file path.
    pub fn parse_spec(spec: &str) -> Result<Fixture, String> {
        let spec = spec.trim();
        match spec {
            "mock" | "1" => parse(MOCK),
            "mock-fda" => {
                let mut f = parse(MOCK)?;
                f.fda = Some("denied".into());
                Ok(f)
            }
            s if s.starts_with('{') => parse(s),
            path => parse(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?),
        }
    }

    pub fn parse(json: &str) -> Result<Fixture, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// The checklist's step ids and statuses are `&'static str` in core.
    fn step_id(s: &str) -> Option<&'static str> {
        super::super::STEPS
            .iter()
            .map(|(id, _)| *id)
            .find(|id| *id == s)
    }

    fn status(s: &str) -> &'static str {
        match s {
            "running" => "running",
            "error" => "error",
            "skipped" => "skipped",
            _ => "done",
        }
    }

    fn source(s: &str) -> &'static str {
        match s {
            "claude" => "claude",
            "codex" => "codex",
            "vscode" => "vscode",
            "cursor" => "cursor",
            _ => "folder",
        }
    }

    impl Fixture {
        /// The scan result, its times moved to now.
        pub fn result(&self, now: u64) -> ScanResult {
            let shift = |t: u64| match self.now {
                Some(base) => t.saturating_add(now).saturating_sub(base),
                None => t,
            };
            let r = &self.result;
            ScanResult {
                agents: r
                    .agents
                    .iter()
                    .map(|a| ScannedAgent {
                        kind: a.kind.clone(),
                        name: a.name.clone(),
                        installed: a.installed,
                        path: a.path.clone(),
                        version: a.version.clone(),
                    })
                    .collect(),
                projects: r
                    .projects
                    .iter()
                    .map(|p| ScannedProject {
                        path: p.path.clone(),
                        display: p.display.clone(),
                        is_git: p.is_git,
                        last_used: p.last_used.map(shift),
                        sources: p.sources.iter().map(|s| source(s)).collect(),
                        agent_history: p.agent_history,
                        added: p.added,
                        rules: RulesInfo {
                            rulesync: p.rules.rulesync,
                            claude_md: p.rules.claude_md,
                            agents_md: p.rules.agents_md,
                        },
                    })
                    .collect(),
                conversations: r
                    .conversations
                    .iter()
                    .map(|c| Conversation {
                        kind: c.kind.clone(),
                        kind_name: c.kind_name.clone(),
                        session_id: c.session_id.clone(),
                        project_path: c.project_path.clone(),
                        project_display: c.project_display.clone(),
                        title: c.title.clone(),
                        last_used: shift(c.last_used),
                        in_pitwall: c.in_pitwall,
                        outside_project: c.outside_project,
                        display_project: c.display_project.clone(),
                        running_elsewhere: c.running_elsewhere,
                    })
                    .collect(),
                running: r
                    .running
                    .iter()
                    .map(|a| RunningAgent {
                        pid: a.pid,
                        kind: a.kind.clone(),
                        kind_name: a.kind_name.clone(),
                        cwd: a.cwd.clone(),
                        cwd_display: a.cwd_display.clone(),
                        session_id: a.session_id.clone(),
                        title: a.title.clone(),
                        in_pitwall: a.in_pitwall,
                        outside_project: a.outside_project,
                        display_project: a.display_project.clone(),
                    })
                    .collect(),
                places: r.places.clone(),
                codex_hooks: r.codex_hooks.as_ref().map(|h| CodexHooksStatus {
                    installed: h.installed,
                    path: h.path.clone(),
                }),
            }
        }

        /// The steps to report: the fixture's, else every step done.
        pub fn steps(&self) -> Vec<Step> {
            if !self.steps.is_empty() {
                return self.steps.clone();
            }
            super::super::STEPS
                .iter()
                .map(|(id, _)| Step {
                    step: id.to_string(),
                    status: "done".into(),
                    summary: None,
                    ms: 0,
                })
                .collect()
        }
    }

    impl OnboardingSource for Fixture {
        fn scan(&self, progress: Progress) -> Result<ScanResult, String> {
            for s in self.steps() {
                let Some(step) = step_id(&s.step) else {
                    continue;
                };
                progress(ScanProgress {
                    step,
                    status: "running",
                    summary: None,
                });
                if s.ms > 0 {
                    std::thread::sleep(Duration::from_millis(s.ms));
                }
                progress(ScanProgress {
                    step,
                    status: status(&s.status),
                    summary: s.summary.clone(),
                });
            }
            Ok(self.result(pitwall_core::clock::unix_ms()))
        }

        fn permissions(&self) -> PermissionsStatus {
            let fda = match self.fda.as_deref() {
                None => {
                    return PermissionsStatus {
                        applies: false,
                        full_disk_access: Access::Unknown,
                        desktop: Access::Unknown,
                        documents: Access::Unknown,
                        downloads: Access::Unknown,
                    }
                }
                Some("granted") => Access::Granted,
                Some(_) => {
                    let at = *self.granted_at.lock().unwrap_or_else(|e| e.into_inner());
                    if at.is_some_and(|t| Instant::now() >= t) {
                        Access::Granted
                    } else {
                        Access::Denied
                    }
                }
            };
            let folders = if fda == Access::Granted {
                Access::Granted
            } else {
                Access::Unknown
            };
            PermissionsStatus {
                applies: true,
                full_disk_access: fda,
                desktop: folders,
                documents: folders,
                downloads: folders,
            }
        }

        /// The pretend user turns the switch on a few seconds later.
        fn open_privacy(&self) -> Result<(), String> {
            let mut at = self.granted_at.lock().unwrap_or_else(|e| e.into_inner());
            if at.is_none() {
                *at = Some(Instant::now() + GRANT_AFTER);
            }
            Ok(())
        }

        fn is_fixture(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use pitwall_core::permissions::Access;

    use super::fixture::{parse, parse_spec, MOCK};
    use super::OnboardingSource;

    #[test]
    fn the_mock_is_the_browser_mocks_scan() {
        let f = parse(MOCK).unwrap();
        let r = f.result(1_000_000_000_000 + 5_000);
        assert_eq!(r.projects.len(), 7);
        assert_eq!(r.conversations.len(), 6);
        assert_eq!(r.running.len(), 3);
        assert_eq!(r.places[0].machines.as_ref().unwrap().len(), 2);
        assert_eq!(r.projects[0].sources, vec!["claude", "codex", "vscode"]);
        assert!(r.projects[0].rules.claude_md && !r.projects[0].rules.rulesync);
        // Times follow the real now: "10 min ago" stays 10 min ago.
        assert_eq!(r.projects[0].last_used, Some(999_999_400_000 + 5_000));
        assert!(r.codex_hooks.is_some_and(|h| !h.installed));
    }

    #[test]
    fn the_scan_reports_every_step_and_never_real_data() {
        let f = parse(r#"{"result":{"running":[{"pid":7,"kind":"codex","kindName":"Codex"}]}}"#)
            .unwrap();
        let seen = Mutex::new(vec![]);
        let r = f
            .scan(&|p| seen.lock().unwrap().push((p.step, p.status)))
            .unwrap();
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 12, "running then done, per step");
        assert_eq!(seen[1], ("agents", "done"));
        assert_eq!(r.running.len(), 1);
        assert_eq!(r.running[0].pid, 7);
        assert!(f.is_fixture());
        assert!(!f.permissions().applies, "no fda: the step is skipped");
    }

    #[test]
    fn folder_access_is_granted_a_while_after_open_settings() {
        let f = parse_spec("mock-fda").unwrap();
        assert_eq!(f.permissions().full_disk_access, Access::Denied);
        f.open_privacy().unwrap();
        assert_eq!(
            f.permissions().full_disk_access,
            Access::Denied,
            "not at once"
        );
        assert_eq!(
            parse_spec("mock").unwrap().permissions().full_disk_access,
            Access::Granted
        );
        assert!(parse_spec("{not json").is_err());
        assert!(parse_spec("/no/such/fixture.json").is_err());
    }

    #[test]
    fn sticky_follows_the_scroll_inside_its_column() {
        use super::super::sticky_shift;
        assert_eq!(sticky_shift(0., 300., 1200., 500.), 0.);
        assert_eq!(sticky_shift(400., 300., 1200., 500.), 120.);
        assert_eq!(sticky_shift(5000., 300., 1200., 500.), 700.);
        assert_eq!(sticky_shift(400., 300., 400., 500.), 0.);
    }
}
