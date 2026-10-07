//! Other places agents run — agw VMs today — and the sessions there that can
//! be added to Pitwall: each provider that can adopt sessions
//! (`caps.attach_existing`) is asked for its machines and what it
//! `discover`s on them. Read-only; providers that aren't set up on this Mac
//! (`Unsupported`) are left out. Shown by the onboarding scan ("Running
//! now"), adopted with `lifecycle::adopt`.

use std::collections::HashSet;

use crate::error::ErrorCode;
use crate::kind::KindCatalog;
use crate::provider::{Discovered, Locator, NativeState, Provider, Providers};

pub use pitwall_proto::{ScannedMachine, ScannedPlace, ScannedSession};

/// Every adopting provider's machines and sessions. `owned`: the locators
/// of Pitwall's agents. Providers and machines are asked in parallel.
/// Blocking.
pub fn places(providers: &Providers, catalog: &KindCatalog, owned: &HashSet<Locator>) -> Vec<ScannedPlace> {
    let adopting: Vec<_> = providers.all().iter().filter(|p| p.caps().attach_existing).cloned().collect();
    std::thread::scope(|s| {
        let tasks: Vec<_> = adopting.iter().map(|p| s.spawn(move || place(&**p, catalog, owned))).collect();
        tasks.into_iter().filter_map(|t| t.join().ok().flatten()).collect()
    })
}

fn place(p: &dyn Provider, catalog: &KindCatalog, owned: &HashSet<Locator>) -> Option<ScannedPlace> {
    let (version, machines) = std::thread::scope(|s| {
        let version = s.spawn(|| p.version());
        let machines = p.machines();
        (version.join().ok().flatten(), machines)
    });
    let machines = match machines {
        // Not set up on this Mac (agw not installed): nothing to show.
        Err(e) if e.is(ErrorCode::Unsupported) => return None,
        Err(e) => {
            eprintln!("pitwall: {}: could not list machines: {e}", p.label());
            None
        }
        Ok(list) => std::thread::scope(|s| {
            let found: Vec<_> = list.iter().map(|m| s.spawn(move || p.discover(&m.id))).collect();
            list.iter()
                .zip(found)
                .map(|(m, f)| {
                    let found = f.join().ok().and_then(|r| r.map_err(|e| eprintln!("pitwall: {}: {e}", m.label)).ok());
                    let mut sessions: Vec<_> =
                        found.unwrap_or_default().into_iter().map(|d| session(p, d, catalog, owned)).collect();
                    sessions.sort_by(|a, b| (a.status != "running", &a.name).cmp(&(b.status != "running", &b.name)));
                    Some(ScannedMachine { id: m.id.to_string(), label: m.label.clone(), detail: m.detail.clone(), sessions })
                })
                .collect()
        }),
    };
    Some(ScannedPlace { provider: p.id().to_string(), label: p.label(), version, machines })
}

fn session(p: &dyn Provider, d: Discovered, catalog: &KindCatalog, owned: &HashSet<Locator>) -> ScannedSession {
    let (kind, kind_name) = match catalog.resolve(&d.kind) {
        Some(k) => (k.id, k.name),
        None => (d.kind.clone(), d.kind.clone()),
    };
    ScannedSession {
        provider: p.id().to_string(),
        machine: d.locator.machine.to_string(),
        native: d.locator.native.clone(),
        name: d.title.clone().unwrap_or_else(|| d.locator.native.clone()),
        kind,
        kind_name,
        program: d.kind,
        workspace: d.workspace,
        user: d.user,
        cwd: d.cwd,
        status: match d.state {
            Some(NativeState::Running) => "running",
            Some(_) => "stopped",
            None => "unknown",
        }
        .into(),
        in_pitwall: owned.contains(&d.locator),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExec;
    use crate::provider::ProviderCaps;
    use crate::testing::FakeProvider;
    use std::sync::Arc;

    #[test]
    fn adopting_providers_list_their_sessions() {
        let fake = FakeProvider::named("vmhost", "box", Arc::new(LocalExec));
        let catalog = KindCatalog::new("/nonexistent/pitwall-agents".into());
        let providers = Providers::new(vec![fake.clone()]);
        // A provider that can't adopt sessions isn't asked.
        fake.add_session("work", "claude-code", "/srv/work");
        assert!(places(&providers, &catalog, &HashSet::new()).is_empty());

        fake.set_caps(ProviderCaps { attach_existing: true, ..Default::default() });
        fake.add_session("zed", "grok", "/srv/z").exit(0);
        let mine = Locator::new(fake.id(), fake.machine_id(), "work");
        let got = places(&providers, &catalog, &[mine].into_iter().collect());
        assert_eq!(got.len(), 1);
        let m = &got[0].machines.as_ref().unwrap()[0];
        assert_eq!((got[0].label.as_str(), m.id.as_str()), ("vmhost", "box"));
        let names: Vec<_> = m.sessions.iter().map(|s| (s.native.as_str(), s.kind.as_str(), s.status.as_str(), s.in_pitwall)).collect();
        assert_eq!(names, [("work", "claude", "running", true), ("zed", "grok", "stopped", false)]);
        assert_eq!(m.sessions[0].kind_name, "Claude Code");
        assert_eq!(m.sessions[0].program, "claude-code");
    }

    /// "Add to Pitwall": tracked without changing the session; again → same
    /// agent; removed → the session keeps running.
    #[test]
    fn adopting_attaches_and_removing_only_forgets() {
        use crate::engine::{lifecycle, Deps, Engine};
        use crate::model::{AdoptSessionRequest, Status};
        use crate::testing::{FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
        let dir = TempDir::new("adopt");
        let local = FakeProvider::named("local", "this-mac", Arc::new(LocalExec));
        let vm = FakeProvider::named("vmhost", "box", Arc::new(LocalExec));
        vm.set_caps(ProviderCaps { start: true, attach_existing: true, survives_detach: true, ..Default::default() });
        vm.set_remote(true);
        let ctl = vm.add_session("work", "claude-code", "/srv/work");
        vm.add_session("idle", "grok", "/srv/idle").exit(0);
        let engine = Engine::open(Deps {
            paths: crate::paths::Paths::new(dir.path().join("pitwall")),
            events: RecordingSink::new(),
            clock: ManualClock::new(1),
            store: MemStore::with(vec![]),
            providers: vec![local.clone(), vm.clone()],
        });
        let req = |native: &str| AdoptSessionRequest {
            provider: "vmhost".into(),
            machine: "box".into(),
            native: native.into(),
            cols: Some(90),
            rows: Some(30),
        };
        let v = lifecycle::adopt(&engine, req("work")).unwrap();
        assert_eq!((v.name.as_str(), v.kind.as_str(), v.cwd.as_str()), ("work", "claude", "/srv/work"));
        assert!(v.running && v.caps.input && v.caps.stop && v.caps.restart && v.caps.remove_keeps_session);
        assert!(!v.caps.diff && !v.caps.hooks && !v.caps.resume && !v.machine.can_create);
        assert_eq!((v.machine.provider.as_str(), v.machine.label.as_str()), ("vmhost", "Fake box"));
        // Input goes through the attached stream.
        crate::engine::input::write_input(&engine, &v.id, "hi\r").unwrap();
        assert_eq!(ctl.input(), b"hi\r");
        assert_eq!(lifecycle::adopt(&engine, req("work")).unwrap().id, v.id, "adopting twice is one agent");
        let idle = lifecycle::adopt(&engine, req("idle")).unwrap();
        assert_eq!((idle.status, idle.running, idle.kind.as_str()), (Status::Stopped, false, "shell"));
        assert!(lifecycle::adopt(&engine, req("nope")).is_err());
        assert!(vm.launched().is_empty(), "nothing was started");
        // The scan now marks it as in Pitwall.
        let owned = engine.records().iter().map(|r| r.locator()).collect();
        let got = places(engine.providers(), engine.kinds(), &owned);
        assert!(got[0].machines.as_ref().unwrap()[0].sessions.iter().all(|s| s.in_pitwall));
        lifecycle::remove(&engine, &v.id, false).unwrap();
        assert!(ctl.running(), "removing an adopted session leaves it running");
        // A provider that can't adopt is refused.
        local.add_session("x", "shell", "/");
        let e = lifecycle::adopt(&engine, AdoptSessionRequest { provider: "local".into(), machine: "this-mac".into(), ..req("x") });
        assert!(e.unwrap_err().contains("can't be added"));
    }
}
