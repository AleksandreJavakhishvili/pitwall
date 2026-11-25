//! Opening on its own and `show_device`'s device, with made-up listings
//! and signals (no simctl, no adb).

use std::time::Duration;

use pitwall_core::devices::{Os, Signal, Source};

use super::*;

const T0: u64 = 1_800_000_000_000;

fn sim(udid: &str, name: &str, ready: bool) -> Device {
    Device { platform: Platform::Ios, id: udid.into(), name: format!("{name} (iOS 26.1)"), virtual_device: true, state: if ready { State::Ready } else { State::NotReady("shut down".into()) } }
}

fn emu(serial: &str, name: &str, state: State) -> Device {
    Device { platform: Platform::Android, id: serial.into(), name: name.into(), virtual_device: true, state }
}

fn seen(d: Device, booted_at: Option<u64>) -> Seen {
    Seen { device: d, booted_at }
}

fn launcher(agent: &str, os: Option<Os>) -> Signal {
    Signal { agent: agent.into(), source: Source::Process, os, device: None, age_ms: 0 }
}

fn printed(agent: &str, os: Option<Os>, device: Option<&str>, age_ms: u64) -> Signal {
    Signal { agent: agent.into(), source: Source::Output, os, device: device.map(String::from), age_ms }
}

fn opened(o: &[Outcome]) -> Vec<(String, Option<String>, bool)> {
    o.iter()
        .map(|o| match o {
            Outcome::Open { device, agent } => (device.key(), Some(agent.clone()), true),
            Outcome::Offer { device, agent } => (device.key(), agent.clone(), false),
        })
        .collect()
}

/// An agent's `xcrun simctl boot`: the simulator that boots while it runs
/// opens beside it; nothing is listed before a signal.
#[test]
fn a_simulator_booted_under_an_agent_opens_beside_it() {
    let mut w = Watch::default();
    w.note(&[], T0);
    assert!(!w.wants(Platform::Ios) && !w.wants(Platform::Android), "nothing to watch");
    w.note(&[launcher("api", Some(Os::Ios))], T0);
    assert!(w.wants(Platform::Ios) && !w.wants(Platform::Android));
    let shut = [seen(sim("A", "iPhone 17", false), Some(T0 - 86_400_000)), seen(sim("B", "iPad Air", true), Some(T0 - 3_600_000))];
    // The baseline: an iPad booted an hour ago is nobody's news.
    assert!(w.listed(Platform::Ios, &shut, T0, Mode::Beside).is_empty());
    let up = [seen(sim("A", "iPhone 17", true), Some(T0 + 2_000)), shut[1].clone()];
    w.note(&[], T0 + 3_000);
    let o = w.listed(Platform::Ios, &up, T0 + 3_000, Mode::Beside);
    assert_eq!(opened(&o), [("ios:A".into(), Some("api".into()), true)]);
    // The same boot again (the tile closed, or still open): nothing.
    assert!(w.listed(Platform::Ios, &up, T0 + 4_500, Mode::Beside).is_empty());
    // Shut down and booted again: a new boot.
    w.note(&[launcher("api", Some(Os::Ios))], T0 + 60_000);
    w.listed(Platform::Ios, &shut, T0 + 60_000, Mode::Beside);
    let again = [seen(sim("A", "iPhone 17", true), Some(T0 + 61_000))];
    assert_eq!(opened(&w.listed(Platform::Ios, &again, T0 + 62_000, Mode::Beside)), [("ios:A".into(), Some("api".into()), true)]);
}

/// A quick `simctl boot` finished before the first listing: simctl's
/// boot time still makes it news.
#[test]
fn a_boot_just_before_the_first_listing_counts() {
    let mut w = Watch::default();
    w.note(&[printed("api", Some(Os::Ios), Some("iPhone 17"), 1_000)], T0);
    let o = w.listed(Platform::Ios, &[seen(sim("A", "iPhone 17", true), Some(T0 - 500))], T0, Mode::Beside);
    assert_eq!(opened(&o), [("ios:A".into(), Some("api".into()), true)]);
}

/// Two agents both running iOS launchers: the one that printed this
/// device's name gets it; when neither did, it is only offered.
#[test]
fn two_agents_are_told_apart_by_what_they_printed_or_not_at_all() {
    let mut w = Watch::default();
    w.note(&[launcher("api", Some(Os::Ios)), launcher("web", None)], T0);
    w.listed(Platform::Ios, &[], T0, Mode::Beside);
    w.note(&[printed("web", None, Some("iPhone 17"), 0), printed("api", Some(Os::Ios), Some("iPad Air"), 0)], T0 + 1_000);
    let o = w.listed(Platform::Ios, &[seen(sim("A", "iPhone 17", true), Some(T0 + 1_500))], T0 + 2_000, Mode::Beside);
    assert_eq!(opened(&o), [("ios:A".into(), Some("web".into()), true)]);
    // A second boot nobody named: ambiguous, so only an offer, nobody's.
    let o = w.listed(Platform::Ios, &[seen(sim("A", "iPhone 17", true), Some(T0 + 1_500)), seen(sim("C", "iPhone 16e", true), Some(T0 + 2_500))], T0 + 3_000, Mode::Beside);
    assert_eq!(opened(&o), [("ios:C".into(), None, false)]);
}

/// Only lines, no launcher seen (a short-lived command): one agent that
/// printed one gets it; two that both did, nobody.
#[test]
fn printed_lines_alone_decide_only_when_one_agent_printed() {
    let mut w = Watch::default();
    w.note(&[printed("api", Some(Os::Android), None, 500)], T0);
    assert!(w.wants(Platform::Android) && !w.wants(Platform::Ios));
    w.listed(Platform::Android, &[], T0, Mode::Beside);
    let up = [seen(emu("emulator-5554", "sdk gphone64 arm64", State::Ready), None)];
    assert_eq!(opened(&w.listed(Platform::Android, &up, T0 + 2_000, Mode::Beside)), [("android:emulator-5554".into(), Some("api".into()), true)]);

    let mut w = Watch::default();
    w.note(&[printed("api", Some(Os::Android), None, 0), printed("web", None, None, 0)], T0);
    w.listed(Platform::Android, &[], T0, Mode::Beside);
    assert_eq!(opened(&w.listed(Platform::Android, &up, T0 + 2_000, Mode::Beside)), [("android:emulator-5554".into(), None, false)]);
    // A line naming another device doesn't count for this one.
    let mut w = Watch::default();
    w.note(&[printed("api", None, Some("emulator-5556"), 0)], T0);
    w.listed(Platform::Android, &[], T0, Mode::Beside);
    assert_eq!(opened(&w.listed(Platform::Android, &up, T0 + 1_000, Mode::Beside)), [("android:emulator-5554".into(), None, false)]);
}

/// Android: a device already online at the first listing is the baseline;
/// one that goes offline and comes back is a new boot; one still
/// "offline" isn't up.
#[test]
fn android_boots_are_devices_coming_online() {
    let mut w = Watch::default();
    w.note(&[launcher("api", Some(Os::Android))], T0);
    let phone = seen(emu("0A1B", "Pixel 8", State::Ready), None);
    assert!(w.listed(Platform::Android, std::slice::from_ref(&phone), T0, Mode::Beside).is_empty(), "already there");
    let starting = seen(emu("emulator-5554", "sdk gphone64", State::NotReady("offline".into())), None);
    assert!(w.listed(Platform::Android, &[phone.clone(), starting], T0 + 1_000, Mode::Beside).is_empty());
    let up = seen(emu("emulator-5554", "sdk gphone64", State::Ready), None);
    let o = w.listed(Platform::Android, &[phone.clone(), up.clone()], T0 + 2_000, Mode::Beside);
    assert_eq!(opened(&o), [("android:emulator-5554".into(), Some("api".into()), true)]);
    assert!(w.listed(Platform::Android, &[phone.clone(), up.clone()], T0 + 3_000, Mode::Beside).is_empty(), "once");
    w.listed(Platform::Android, std::slice::from_ref(&phone), T0 + 4_000, Mode::Beside);
    assert_eq!(w.listed(Platform::Android, &[phone, up], T0 + 5_000, Mode::Beside).len(), 1, "back again: a new boot");
}

/// The setting: offer says so instead of opening, off does nothing (and
/// the boot isn't kept for later), an agent's own show_device isn't
/// repeated.
#[test]
fn the_setting_and_show_device_decide_what_happens() {
    let run = |mode: Mode, shown: bool| {
        let mut w = Watch::default();
        w.note(&[launcher("api", Some(Os::Ios))], T0);
        w.listed(Platform::Ios, &[], T0, mode);
        if shown {
            w.shown("ios:A");
        }
        let up = [seen(sim("A", "iPhone 17", true), Some(T0 + 1_000))];
        let first = opened(&w.listed(Platform::Ios, &up, T0 + 2_000, mode));
        let later = opened(&w.listed(Platform::Ios, &up, T0 + 3_000, Mode::Beside));
        (first, later)
    };
    assert_eq!(run(Mode::Beside, false).0, [("ios:A".into(), Some("api".into()), true)]);
    assert_eq!(run(Mode::Offer, false).0, [("ios:A".into(), Some("api".into()), false)]);
    assert_eq!(run(Mode::Off, false), (vec![], vec![]), "off: nothing, now or later");
    assert_eq!(run(Mode::Beside, true), (vec![], vec![]), "shown by the agent already");
    assert_eq!(Mode::parse("offer"), Mode::Offer);
    assert_eq!(Mode::parse("off"), Mode::Off);
    assert_eq!(Mode::parse("beside"), Mode::Beside);
    assert_eq!(Mode::parse("bogus"), Mode::Beside);
}

/// Signals age out: the platform stops being listed, and a boot long
/// after the agent's launcher is nobody's.
#[test]
fn old_signals_stop_the_watch_and_dont_claim_late_boots() {
    let mut w = Watch::default();
    w.note(&[launcher("api", Some(Os::Ios))], T0);
    w.listed(Platform::Ios, &[], T0, Mode::Beside);
    w.note(&[launcher("web", Some(Os::Android))], T0 + KEEP_MS - 1_000);
    let o = w.listed(Platform::Ios, &[seen(sim("A", "iPhone 17", true), Some(T0 + BEFORE_MS + 30_000))], T0 + KEEP_MS - 1_000, Mode::Beside);
    assert_eq!(opened(&o), [("ios:A".into(), None, false)], "api's launcher was long gone");
    w.note(&[], T0 + KEEP_MS + 1_000);
    assert!(!w.wants(Platform::Ios), "nothing for iOS any more");
    assert!(w.wants(Platform::Android));
    assert!(w.listed(Platform::Ios, &[seen(sim("B", "iPad", true), Some(T0 + KEEP_MS))], T0 + KEEP_MS + 1_000, Mode::Beside).is_empty());
}

#[test]
fn show_device_finds_the_device_named_or_the_one_booted() {
    let devices = vec![
        sim("AAAA-1", "iPhone 17", true),
        sim("BBBB-2", "iPhone 17 Pro", false),
        Device { name: "iPhone 17 (iOS 26.0)".into(), ..sim("CCCC-3", "x", false) },
        emu("emulator-5554", "sdk gphone64 arm64", State::Ready),
    ];
    let one = |q: Option<&str>| match resolve(q, &devices) {
        Found::One(d) => d.key(),
        other => format!("{other:?}"),
    };
    // By name (the booted one of two runtimes), name with its runtime,
    // UDID, key, serial; case doesn't matter.
    assert_eq!(one(Some("iPhone 17")), "ios:AAAA-1");
    assert_eq!(one(Some("iphone 17 (ios 26.1)")), "ios:AAAA-1");
    assert_eq!(one(Some("aaaa-1")), "ios:AAAA-1");
    assert_eq!(one(Some("ios:AAAA-1")), "ios:AAAA-1");
    assert_eq!(one(Some("emulator-5554")), "android:emulator-5554");
    assert_eq!(one(Some("sdk_gphone64_arm64")), "android:emulator-5554");
    // Known but not up: wait.
    assert_eq!(resolve(Some("iPhone 17 Pro"), &devices), Found::Booting("iPhone 17 Pro (iOS 26.1) is shut down".into()));
    // Unknown: wait a little (an emulator appears once it starts).
    let Found::Missing(why) = resolve(Some("Pixel 9"), &devices) else { panic!() };
    assert!(why.starts_with("no device \"Pixel 9\" (there are: iPhone 17 (iOS 26.1) (ios:AAAA-1)"), "{why}");
    // None named: two booted is a question back.
    let Found::No(why) = resolve(None, &devices) else { panic!() };
    assert!(why.starts_with("several devices are booted: iPhone 17 (iOS 26.1) (ios:AAAA-1), sdk gphone64 arm64 (android:emulator-5554)"), "{why}");
    assert_eq!(resolve(None, &devices[..1]), Found::One(devices[0].clone()));
    assert_eq!(resolve(None, &devices[1..3]), Found::Booting("no device is booted".into()));
    // A part of a name that fits several booted ones: which?
    let two = vec![sim("A", "iPhone 17", true), sim("B", "iPhone 17 Pro", true)];
    assert_eq!(resolve(Some("iPhone 17"), &two), Found::One(two[0].clone()), "the exact name wins");
    let Found::No(why) = resolve(Some("iPhone"), &two) else { panic!() };
    assert!(why.starts_with("\"iPhone\" could be"), "{why}");
}

/// A real headless boot, in a private device set (never the user's):
/// `simctl boot` run under this process is seen as a launcher in the real
/// process table, and the simulator coming up in the real `simctl list` is
/// a boot that opens beside "this process". Shut down afterwards; the
/// caller creates and deletes the device:
/// `PITWALL_TEST_SIM_SET=<dir> PITWALL_TEST_SIM=<udid> cargo test -p
/// pitwall-app --lib device::auto::tests::a_real_boot -- --ignored`.
#[test]
#[ignore = "boots a simulator: needs Xcode and PITWALL_TEST_SIM(_SET)"]
fn a_real_boot_under_an_agent_is_seen_and_opens_beside_it() {
    let set = std::path::PathBuf::from(std::env::var_os("PITWALL_TEST_SIM_SET").expect("PITWALL_TEST_SIM_SET"));
    let udid = std::env::var("PITWALL_TEST_SIM").expect("PITWALL_TEST_SIM");
    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let listed = || super::super::ios::list_in(Some(&set)).unwrap().into_iter().map(|(device, booted_at)| Seen { device, booted_at }).collect::<Vec<_>>();
    let before = listed();
    assert!(before.iter().any(|s| s.device.id == udid && s.device.state != State::Ready), "shut down to begin with");
    // "The agent": this process; its child runs the boot.
    let me = std::process::id();
    let mut boot = std::process::Command::new("/bin/sh").arg("-c").arg(format!("xcrun simctl --set '{}' boot {udid} && sleep 3", set.display())).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let out = std::process::Command::new("/bin/ps").args(["-axww", "-o", "pid=,ppid=,args="]).output().unwrap();
    let rows = pitwall_core::procs::parse_table(&String::from_utf8_lossy(&out.stdout));
    let found = pitwall_core::devices::launchers_under(&rows, &[("me".into(), me)]);
    assert_eq!(found, vec![("me".to_string(), Some(Os::Ios))], "the boot is seen under its agent");
    let mut w = Watch::default();
    w.note(&found.iter().map(|(a, os)| launcher(a, *os)).collect::<Vec<_>>(), now());
    assert!(w.listed(Platform::Ios, &before, now(), Mode::Beside).is_empty(), "the baseline");
    let _ = boot.wait();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut got = vec![];
    while got.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        got = w.listed(Platform::Ios, &listed(), now(), Mode::Beside);
    }
    let _ = std::process::Command::new("xcrun").arg("simctl").arg("--set").arg(&set).args(["shutdown", &udid]).status();
    assert_eq!(opened(&got), [(format!("ios:{udid}"), Some("me".into()), true)]);
}

/// Asked before the device is up: listed again each second until it is,
/// at most a minute (a name nothing has: 20 s).
#[test]
fn show_device_waits_for_a_device_still_booting() {
    let p = Patience::default();
    let mut looks = 0;
    let mut slept = Duration::ZERO;
    let r = find_waiting(
        Some("iPhone 17"),
        || {
            looks += 1;
            vec![sim("A", "iPhone 17", looks >= 4)]
        },
        |d| slept += d,
        p,
    );
    assert_eq!(r, Ok((sim("A", "iPhone 17", true), true)));
    assert_eq!((looks, slept), (4, Duration::from_secs(3)));
    // Up already: no wait.
    assert_eq!(find_waiting(None, || vec![sim("A", "iPhone 17", true)], |_| panic!("no sleep"), p), Ok((sim("A", "iPhone 17", true), false)));
    // Never up: gives up after the minute, says so.
    let mut slept = Duration::ZERO;
    let e = find_waiting(Some("iPhone 17"), || vec![sim("A", "iPhone 17", false)], |d| slept += d, p).unwrap_err();
    assert_eq!(slept, Duration::from_secs(60));
    assert!(e.starts_with("iPhone 17 (iOS 26.1) is shut down, after waiting 60 s. Boot or start it first"), "{e}");
    let mut slept = Duration::ZERO;
    let e = find_waiting(Some("Pixel"), Vec::new, |d| slept += d, p).unwrap_err();
    assert_eq!(slept, Duration::from_secs(20));
    assert!(e.starts_with("no device \"Pixel\" (there are none), after waiting 20 s"), "{e}");
    // Ambiguous: no wait at all.
    let e = find_waiting(None, || vec![sim("A", "iPhone 17", true), sim("B", "iPad", true)], |_| panic!("no sleep"), p).unwrap_err();
    assert!(e.starts_with("several devices are booted"), "{e}");
}
