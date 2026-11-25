//! Agents starting simulators and emulators (docs/spec/device.md, "Opening
//! on its own"): what tells Pitwall that an agent is bringing up a device,
//! so the app can show the device's screen beside that agent when it boots.
//!
//! Two signals, both the tools' own (nothing an agent has to do for us):
//! - **a launcher in its process tree**: `xcrun simctl boot`, `xcodebuild`
//!   for a simulator, `open -a Simulator`, `flutter run`, `expo run:ios`,
//!   `react-native run-android`, `emulator`, `adb install`, gradle's
//!   `installDebug`… ([`launcher`]), found among the descendants of the
//!   agent's process ([`launchers_under`]), from one `ps` of the whole
//!   table;
//! - **a line its terminal printed**: flutter's "Launching … on iPhone 17",
//!   Expo's "Opening on …", gradle's "Installing APK … on '…'", an emulator
//!   serial, or the command itself as the agent's screen echoes it
//!   ([`hint`]; read by the terminal's line scanner, `term::preview`).
//!
//! Pure parsing here, and [`Engine::device_signals`] that gathers both for
//! the agents on this machine. Deciding which device an agent started and
//! what to show is the app's.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::engine::Engine;
use crate::procs::ProcRow;

/// Which kind of device a signal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Ios,
    Android,
}

/// What a printed line says about a device: for which system (`None`:
/// either, as `flutter run`), and which device when it names one (a name
/// as the tool prints it, a simulator's UDID or an adb serial).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub os: Option<Os>,
    pub device: Option<String>,
}

/// Where a [`Signal`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A launcher runs in the agent's process tree now.
    Process,
    /// Its terminal printed a line [`hint`] knows, `age_ms` ago.
    Output,
}

/// One agent bringing up a device, as seen now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    pub agent: String,
    pub source: Source,
    pub os: Option<Os>,
    pub device: Option<String>,
    /// How long ago (0 for a launcher running now).
    pub age_ms: u64,
}

fn base(word: &str) -> &str {
    let w = word.trim_matches(|c| c == '"' || c == '\'');
    w.rsplit('/').next().unwrap_or(w)
}

/// Whether a process (its `args`) is a tool that boots, starts or installs
/// on a device, and for which system (`Some(None)`: either, as `flutter
/// run` without a device named). Looking (`simctl list`, `adb devices`)
/// is not starting.
pub fn launcher(args: &str) -> Option<Option<Os>> {
    let t: Vec<&str> = args.split_whitespace().collect();
    let first = base(t.first()?);
    let lower = args.to_ascii_lowercase();
    // simctl, alone or through xcrun: the subcommand, past `--set <dir>`.
    if let Some(i) = t.iter().position(|w| base(w) == "simctl") {
        let mut rest = t[i + 1..].iter();
        while let Some(w) = rest.next() {
            if *w == "--set" {
                rest.next();
                continue;
            }
            if w.starts_with('-') {
                continue;
            }
            return ["boot", "bootstatus", "install", "launch", "openurl"].contains(w).then_some(Some(Os::Ios));
        }
        return None;
    }
    if first == "xcodebuild" || t.iter().any(|w| base(w) == "xcodebuild") {
        return lower.contains("simulator").then_some(Some(Os::Ios));
    }
    if t.iter().any(|w| base(w) == "devicectl") {
        return Some(Some(Os::Ios));
    }
    if first == "open" {
        let app = t.windows(2).any(|p| p[0] == "-a" && base(p[1]).trim_end_matches(".app").eq_ignore_ascii_case("simulator"));
        return (app || lower.contains("simulator.app")).then_some(Some(Os::Ios));
    }
    // flutter run (the `flutter` script, or the Dart snapshot it runs).
    if let Some(i) = t.iter().position(|w| base(w) == "flutter" || base(w) == "flutter_tools.snapshot") {
        if t[i + 1..].contains(&"run") {
            let named = t.windows(2).find(|p| p[0] == "-d" || p[0] == "--device-id").map(|p| p[1].to_ascii_lowercase());
            let os = match named {
                Some(d) if d.starts_with("emulator-") || d == "android" => Some(Os::Android),
                Some(d) if d.contains("iphone") || d.contains("ipad") || d == "ios" => Some(Os::Ios),
                _ => None,
            };
            return Some(os);
        }
        return None;
    }
    if t.iter().any(|w| matches!(base(w), "expo" | "expo-cli")) {
        let ios = t.iter().any(|w| *w == "run:ios" || *w == "--ios" || *w == "-i");
        let android = t.iter().any(|w| *w == "run:android" || *w == "--android" || *w == "-a");
        return match (ios, android) {
            (true, false) => Some(Some(Os::Ios)),
            (false, true) => Some(Some(Os::Android)),
            (true, true) => Some(None),
            _ => None,
        };
    }
    if lower.contains("react-native") {
        if t.contains(&"run-ios") {
            return Some(Some(Os::Ios));
        }
        if t.contains(&"run-android") {
            return Some(Some(Os::Android));
        }
        return None;
    }
    if first == "emulator" || (first.starts_with("qemu-system-") && t.contains(&"-avd")) {
        return Some(Some(Os::Android));
    }
    if first == "adb" {
        let starts = ["install", "install-multiple", "install-multi-package", "wait-for-device"];
        return t.iter().any(|w| starts.contains(w)).then_some(Some(Os::Android));
    }
    if lower.contains("gradle") && t.iter().any(|w| install_task(w)) {
        return Some(Some(Os::Android));
    }
    None
}

/// `installDebug`, `:app:installRelease`, `installFreeDebug`.
fn install_task(word: &str) -> bool {
    let task = word.rsplit(':').next().unwrap_or(word);
    task.strip_prefix("install").and_then(|r| r.chars().next()).is_some_and(|c| c.is_ascii_uppercase())
}

/// The launchers running under each agent's process (`roots`: an agent's
/// id and its process), from the whole process table. The agent's own
/// process doesn't count, only what it started.
pub fn launchers_under(rows: &[ProcRow], roots: &[(String, u32)]) -> Vec<(String, Option<Os>)> {
    let mut children: HashMap<u32, Vec<&ProcRow>> = HashMap::new();
    for r in rows {
        if r.pid != r.ppid {
            children.entry(r.ppid).or_default().push(r);
        }
    }
    let mut out = Vec::new();
    for (agent, root) in roots {
        let mut stack = vec![*root];
        let mut seen = 0usize;
        while let Some(pid) = stack.pop() {
            for c in children.get(&pid).into_iter().flatten() {
                seen += 1;
                // A runaway table (a cycle) stops here.
                if seen > 4096 {
                    break;
                }
                if let Some(os) = launcher(&c.args) {
                    if !out.iter().any(|(a, o)| a == agent && *o == os) {
                        out.push((agent.clone(), os));
                    }
                }
                stack.push(c.pid);
            }
        }
    }
    out
}

struct Pattern {
    re: Regex,
    os: Option<Os>,
}

fn patterns() -> &'static [Pattern] {
    static P: OnceLock<Vec<Pattern>> = OnceLock::new();
    P.get_or_init(|| {
        let p = |re: &str, os: Option<Os>| Pattern { re: Regex::new(re).expect("device hint pattern"), os };
        vec![
            // The commands, as an agent's screen echoes them.
            p(r#"(?i)\bsimctl\s+(?:--set\s+\S+\s+)?boot\s+(?:"([^"]+)"|'([^']+)'|(\S+))"#, Some(Os::Ios)),
            p(r#"(?i)\bopen\s+-a\s+"?Simulator\b()"#, Some(Os::Ios)),
            p(r#"(?i)\bplatform=iOS Simulator(?:,[^'"]*?\bname=([^,'"]+))?"#, Some(Os::Ios)),
            p(r"(?i)\bemulator\s+(?:\S+\s+)*?-avd\s+(\S+)", Some(Os::Android)),
            p(r"(?i)\bemulator\s+@(\S+)", Some(Os::Android)),
            p(r"(?i)\badb\s+(?:-s\s+(\S+)\s+)?(?:install|wait-for-device)\b", Some(Os::Android)),
            p(r"(?i)\breact-native\s+run-ios\b()", Some(Os::Ios)),
            p(r"(?i)\breact-native\s+run-android\b()", Some(Os::Android)),
            p(r"(?i)\bexpo\s+run:ios\b()", Some(Os::Ios)),
            p(r"(?i)\bexpo\s+run:android\b()", Some(Os::Android)),
            p(r"(?i)\bflutter\s+run\b(?:.*?\s-d\s+(\S+))?", None),
            p(r"(?i)\bgradlew?\b.*\binstall(?:Debug|Release)\b()", Some(Os::Android)),
            // What the tools print.
            p(r"(?i)\bLaunching\s+\S+\s+on\s+(.+?)\s+in\s+(?:debug|profile|release)\s+mode", None),
            p(r"(?i)\bOpening\s+(?:\S+://\S+\s+)?on\s+(.+?)\s*(?:\.\.\.|…)?\s*$", None),
            p(r"(?i)\bFound booted\s+(.+?)\s*$", Some(Os::Ios)),
            p(r"(?i)\bInstalling APK\s+'[^']*'\s+on\s+'([^']+)'", Some(Os::Android)),
            p(r"\b(emulator-\d{4,5})\b", Some(Os::Android)),
        ]
    })
}

/// What `line` (one finished line of output, escapes gone) says about a
/// device starting, if anything.
pub fn hint(line: &str) -> Option<Hint> {
    if line.len() < 6 {
        return None;
    }
    for p in patterns() {
        if let Some(c) = p.re.captures(line) {
            let device = c.iter().skip(1).flatten().map(|m| m.as_str().trim().trim_matches(|ch| ch == '"' || ch == '\'')).find(|s| !s.is_empty()).map(String::from);
            let os = match (p.os, device.as_deref()) {
                (None, Some(d)) if d.to_ascii_lowercase().starts_with("emulator-") => Some(Os::Android),
                (os, _) => os,
            };
            return Some(Hint { os, device });
        }
    }
    None
}

/// What [`Engine::device_signals`] remembers between looks, so it only
/// reads the process table when something may have changed.
#[derive(Debug, Default)]
pub struct Look {
    seqs: HashMap<String, u64>,
    found: Vec<(String, Option<Os>)>,
}

/// How long a printed line counts.
pub const HINT_FOR_MS: u64 = 120_000;

impl Engine {
    /// The agents on this machine bringing up a device now: launchers in
    /// their process trees, and lines their terminals printed in the last
    /// [`HINT_FOR_MS`]. The process table is read only when an agent's
    /// output changed since the last look or a launcher was running then
    /// (an idle Pitwall reads nothing).
    pub fn device_signals(&self, look: &mut Look) -> Vec<Signal> {
        let mut roots = Vec::new();
        let mut changed = false;
        let mut out = Vec::new();
        {
            let agents = self.agents();
            let mut seqs = HashMap::new();
            for a in agents.iter() {
                let (Some(pid), Some(host)) = (a.local_pid(), a.host.as_ref()) else { continue };
                if host.ended() {
                    continue;
                }
                let seq = host.output_seq.load(std::sync::atomic::Ordering::Relaxed);
                changed |= look.seqs.get(&a.rec.id) != Some(&seq);
                seqs.insert(a.rec.id.clone(), seq);
                roots.push((a.rec.id.clone(), pid));
                for (age_ms, h) in host.device_hints() {
                    if age_ms <= HINT_FOR_MS {
                        out.push(Signal { agent: a.rec.id.clone(), source: Source::Output, os: h.os, device: h.device, age_ms });
                    }
                }
            }
            look.seqs = seqs;
        }
        if roots.is_empty() {
            look.found.clear();
        } else if changed || !look.found.is_empty() {
            if let Some(table) = crate::platform::process_table() {
                look.found = launchers_under(&crate::procs::parse_table(&table), &roots);
            }
        }
        for (agent, os) in &look.found {
            out.push(Signal { agent: agent.clone(), source: Source::Process, os: *os, device: None, age_ms: 0 });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is(args: &str) -> Option<Option<Os>> {
        launcher(args)
    }

    #[test]
    fn launchers_are_the_tools_that_start_devices() {
        let ios = Some(Some(Os::Ios));
        let android = Some(Some(Os::Android));
        assert_eq!(is("xcrun simctl boot iPhone 17"), ios);
        assert_eq!(is("/Applications/Xcode.app/Contents/Developer/usr/bin/simctl boot 1F0C"), ios);
        assert_eq!(is("xcrun simctl --set /tmp/s bootstatus ABCD -b"), ios);
        assert_eq!(is("xcrun simctl install booted app.app"), ios);
        assert_eq!(is("xcrun simctl list devices -j"), None, "looking isn't starting");
        assert_eq!(is("xcrun simctl io booted screenshot -"), None);
        assert_eq!(is("xcodebuild test -scheme App -destination platform=iOS Simulator,name=iPhone 17"), ios);
        assert_eq!(is("xcodebuild -list"), None);
        assert_eq!(is("xcrun devicectl device install app"), ios);
        assert_eq!(is("open -a Simulator"), ios);
        assert_eq!(is("open /Applications/Xcode.app/Contents/Developer/Applications/Simulator.app"), ios);
        assert_eq!(is("open README.md"), None);
        assert_eq!(is("/bin/bash /Users/a/flutter/bin/flutter run"), Some(None));
        assert_eq!(is("/Users/a/flutter/bin/cache/dart-sdk/bin/dart --packages=x /Users/a/flutter/bin/cache/flutter_tools.snapshot run -d emulator-5554"), android);
        assert_eq!(is("flutter run -d iPhone"), ios);
        assert_eq!(is("flutter test"), None);
        assert_eq!(is("node /p/node_modules/.bin/expo run:ios"), ios);
        assert_eq!(is("node /p/node_modules/.bin/expo start --android"), android);
        assert_eq!(is("node /p/node_modules/.bin/expo start"), None, "a dev server only");
        assert_eq!(is("node /p/node_modules/react-native/cli.js run-android"), android);
        assert_eq!(is("node /p/node_modules/react-native/cli.js run-ios --simulator iPhone 17"), ios);
        assert_eq!(is("node /p/node_modules/react-native/cli.js start"), None);
        assert_eq!(is("/Users/a/Library/Android/sdk/emulator/emulator -avd Pixel_8"), android);
        assert_eq!(is("/sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64 -avd Pixel_8"), android);
        assert_eq!(is("adb -s emulator-5554 install app.apk"), android);
        assert_eq!(is("adb wait-for-device"), android);
        assert_eq!(is("adb devices -l"), None, "looking isn't starting");
        assert_eq!(is("adb -L tcp:5037 fork-server server --reply-fd 4"), None);
        assert_eq!(is("java -cp gradle-wrapper.jar org.gradle.wrapper.GradleWrapperMain :app:installDebug"), android);
        assert_eq!(is("./gradlew assembleDebug"), None);
        assert_eq!(is("claude --resume abc"), None);
        assert_eq!(is(""), None);
    }

    fn row(pid: u32, ppid: u32, args: &str) -> ProcRow {
        ProcRow { pid, ppid, args: args.into() }
    }

    #[test]
    fn launchers_count_under_the_agent_that_started_them() {
        let rows = vec![
            row(1, 0, "/sbin/launchd"),
            row(100, 1, "claude"),
            row(101, 100, "/bin/zsh -c xcrun simctl boot 'iPhone 17'"),
            row(102, 101, "xcrun simctl boot iPhone 17"),
            row(200, 1, "codex"),
            row(201, 200, "/bin/bash -c ./gradlew :app:installDebug"),
            row(202, 201, "java org.gradle.wrapper.GradleWrapperMain :app:installDebug"),
            row(300, 1, "claude"),
            row(301, 300, "cargo test"),
            // Outside every agent: the user's own.
            row(400, 1, "xcrun simctl boot iPad"),
        ];
        let roots = vec![("a".to_string(), 100), ("b".to_string(), 200), ("c".to_string(), 300)];
        let found = launchers_under(&rows, &roots);
        // The shell running `xcrun simctl boot` counts too, once.
        assert_eq!(found, vec![("a".to_string(), Some(Os::Ios)), ("b".to_string(), Some(Os::Android))]);
        // The agent itself never counts (a root that is a launcher).
        assert!(launchers_under(&[row(5, 1, "emulator -avd X")], &[("x".into(), 5)]).is_empty());
        // A cycle in a broken table ends.
        assert!(launchers_under(&[row(7, 8, "a"), row(8, 7, "b")], &[("x".into(), 7)]).is_empty());
    }

    /// The engine gathers the lines each local agent printed, by agent;
    /// an agent whose terminal ended says nothing.
    #[test]
    fn the_engine_reports_each_agents_device_lines() {
        use crate::provider::ProviderCaps;
        use crate::testing::Harness;
        let h = Harness::new(vec![]);
        h.provider.set_caps(ProviderCaps { local_process: true, ..crate::testing::FakeProvider::local_like() });
        let dir = h.dir.path().to_string_lossy().into_owned();
        let new = |name: &str| {
            let req = crate::model::CreateAgentRequest { name: name.into(), kind: "shell".into(), project_path: dir.clone(), ..Default::default() };
            crate::engine::lifecycle::create(&h.engine, req).unwrap().id
        };
        let (a, b) = (new("a"), new("b"));
        let mut look = Look::default();
        assert!(h.engine.device_signals(&mut look).is_empty());
        h.provider.ctl(&b).unwrap().output(b"$ npx expo run:android\r\n");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut got = vec![];
        while got.is_empty() && std::time::Instant::now() < deadline {
            got = h.engine.device_signals(&mut look);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got, vec![Signal { agent: b.clone(), source: Source::Output, os: Some(Os::Android), device: None, age_ms: 0 }]);
        // Old lines stop counting.
        h.clock.advance(HINT_FOR_MS + 1);
        assert!(h.engine.device_signals(&mut look).is_empty());
        let _ = a;
    }

    fn h(line: &str) -> Option<(Option<Os>, Option<String>)> {
        hint(line).map(|h| (h.os, h.device))
    }

    #[test]
    fn lines_that_say_a_device_is_starting() {
        let ios = Some(Os::Ios);
        let android = Some(Os::Android);
        // Commands as the agent's screen shows them.
        assert_eq!(h("⏺ Bash(xcrun simctl boot \"iPhone 17\")"), Some((ios, Some("iPhone 17".into()))));
        assert_eq!(h("$ xcrun simctl boot 1F0C2C2A-0000"), Some((ios, Some("1F0C2C2A-0000".into()))));
        assert_eq!(h("Bash(open -a Simulator)"), Some((ios, None)));
        assert_eq!(h("xcodebuild test -destination 'platform=iOS Simulator,name=iPhone 17 Pro'"), Some((ios, Some("iPhone 17 Pro".into()))));
        assert_eq!(h("emulator -no-snapshot -avd Pixel_8 &"), Some((android, Some("Pixel_8".into()))));
        assert_eq!(h("adb -s emulator-5556 install -r app.apk"), Some((android, Some("emulator-5556".into()))));
        assert_eq!(h("npx expo run:android"), Some((android, None)));
        assert_eq!(h("npx react-native run-ios"), Some((ios, None)));
        assert_eq!(h("flutter run -d emulator-5554"), Some((android, Some("emulator-5554".into()))));
        // What the tools print.
        assert_eq!(h("Launching lib/main.dart on iPhone 17 in debug mode..."), Some((None, Some("iPhone 17".into()))));
        assert_eq!(h("› Opening on iPhone 17 Pro"), Some((None, Some("iPhone 17 Pro".into()))));
        assert_eq!(h("› Opening exp://192.168.1.4:8081 on Pixel_8..."), Some((None, Some("Pixel_8".into()))));
        assert_eq!(h("info Found booted iPhone 17"), Some((ios, Some("iPhone 17".into()))));
        assert_eq!(h("Installing APK 'app-debug.apk' on 'Pixel_8(AVD) - 14' for :app:debug"), Some((android, Some("Pixel_8(AVD) - 14".into()))));
        assert_eq!(h("INFO | Boot completed in 9000 ms; emulator-5554 is online"), Some((android, Some("emulator-5554".into()))));
        // Not about a device.
        assert_eq!(h("Compiling pitwall v0.1"), None);
        assert_eq!(h("simctl list devices"), None);
        assert_eq!(h("open the file"), None);
    }
}
