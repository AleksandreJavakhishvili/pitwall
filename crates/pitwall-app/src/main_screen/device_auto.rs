//! A device's screen next to the agent that started it (docs/spec/device.md,
//! "Opening on its own"), and an agent's `show_device`.
//!
//! [`start`] looks every [`EVERY`] while device screens are on and
//! `general.deviceAutoOpen` isn't off: the agents' signals from the engine
//! (`pitwall_core::devices`), then, only for a platform an agent gives a
//! signal for, its devices (`simctl list`; the adb server over its socket,
//! never started). What to do is [`crate::device::auto::Watch`]'s: open the
//! tile right of the agent's terminal ([`show_beside`]), or say the device
//! started with "Open in Pitwall" (a toast in the main window). Nothing
//! comes forward and the keyboard stays where it is.
//!
//! [`show_for_agent`] is `show_device`'s end (after the wait, on the
//! connection's thread, `cli_spaces`): the same tile, beside the agent.

use std::time::Duration;

use gpui::App;
use pitwall_proto::{code, DeviceShown, ErrorBody};

use super::tree::{Content, Side, Surface};
use crate::device::auto::{Mode, Outcome, Seen, Watch};
use crate::device::{Device, Platform};
use crate::explorer::dock::FilesPlace;

/// `general.deviceAutoOpen`.
pub const AUTO_SETTING: &str = "general.deviceAutoOpen";
/// `agents.showDevice`.
pub const SHOW_SETTING: &str = "agents.showDevice";

/// How often it looks.
pub const EVERY: Duration = Duration::from_millis(1_500);

/// What Pitwall does when an agent starts a device.
pub fn mode(cx: &App) -> Mode {
    let v = pitwall_proto::settings::find(AUTO_SETTING).and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def));
    Mode::parse(v.as_ref().and_then(|v| v.as_str()).unwrap_or("beside"))
}

/// Agents may show devices (on unless turned off).
pub fn show_enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(SHOW_SETTING)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

#[derive(Default)]
struct Auto {
    watch: Watch,
}

impl gpui::Global for Auto {}

fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The devices of `p` here now, with when each booted.
fn list(p: Platform) -> Vec<Seen> {
    match p {
        Platform::Ios if crate::platform::video::supported() => crate::device::ios::list_in(None).unwrap_or_default().into_iter().map(|(device, booted_at)| Seen { device, booted_at }).collect(),
        Platform::Ios => vec![],
        Platform::Android => crate::device::android::server_devices().unwrap_or_default().into_iter().map(|device| Seen { device, booted_at: None }).collect(),
    }
}

/// Every device here now that `show_device` can name (simulators, and
/// what a running adb server knows).
pub fn devices_here() -> Vec<Device> {
    [Platform::Ios, Platform::Android].into_iter().flat_map(list).map(|s| s.device).collect()
}

/// Start looking (once, at startup). Nothing runs in tests.
pub fn start(engine: pitwall_core::engine::Shared, cx: &mut App) {
    if cfg!(test) {
        return;
    }
    cx.set_global(Auto::default());
    cx.spawn(async move |cx| {
        let mut look = pitwall_core::devices::Look::default();
        loop {
            cx.background_executor().timer(EVERY).await;
            let on = cx.update(|cx| super::device_tile::enabled(cx) && mode(cx) != Mode::Off);
            if !on {
                continue;
            }
            let e = engine.clone();
            let (signals, back) = cx
                .background_executor()
                .spawn(async move {
                    let s = e.device_signals(&mut look);
                    (s, look)
                })
                .await;
            look = back;
            let wants = cx.update(|cx| {
                let w = &mut cx.global_mut::<Auto>().watch;
                w.note(&signals, unix_ms());
                [Platform::Ios, Platform::Android].into_iter().filter(|p| w.wants(*p)).collect::<Vec<_>>()
            });
            if wants.is_empty() {
                continue;
            }
            let listings = cx.background_executor().spawn(async move { wants.into_iter().map(|p| (p, list(p))).collect::<Vec<_>>() }).await;
            cx.update(|cx| {
                let m = mode(cx);
                let now = unix_ms();
                let mut outcomes = vec![];
                for (p, seen) in &listings {
                    super::device_tile::learn(&seen.iter().map(|s| s.device.clone()).collect::<Vec<_>>(), cx);
                    outcomes.extend(cx.global_mut::<Auto>().watch.listed(*p, seen, now, m));
                }
                for o in outcomes {
                    act(o, cx);
                }
            });
        }
    })
    .detach();
}

fn act(o: Outcome, cx: &mut App) {
    match o {
        Outcome::Open { device, agent } => {
            if show_beside(&device.key(), &agent, false, cx).is_err() {
                // Its terminal isn't in any open window: said instead.
                offer(device, Some(agent), cx);
            }
        }
        Outcome::Offer { device, agent } => offer(device, agent, cx),
    }
}

/// "iPhone 17 started — Open in Pitwall", in the main window.
fn offer(device: Device, agent: Option<String>, cx: &mut App) {
    let name = device.name.clone();
    let key = device.key();
    let who = agent.as_deref().map(|a| crate::explorer::dock::agent_name(a, cx));
    let detail = match &who {
        Some(n) => format!("{n} started it. Open its screen in Pitwall to watch and use it."),
        None => "Open its screen in Pitwall to watch and use it.".to_string(),
    };
    crate::windows::device_offer(&key, format!("{name} started"), detail, agent, cx);
}

/// Device `key`'s tile right of `agent`'s terminal, or where it is when
/// it is open already. `explicit` (an agent asked): a tile already open
/// is brought into view in its window, and an agent whose terminal isn't
/// shown gets it in the space Pitwall shows; otherwise that is an error.
/// Never takes the keyboard or brings Pitwall forward. Where it is, in
/// words, and whether it was open already.
pub fn show_beside(key: &str, agent: &str, explicit: bool, cx: &mut App) -> Result<(String, bool, Option<String>), String> {
    let s = Surface::Device(key.to_string());
    let skey = s.key();
    let ui = crate::ui_state::get(cx);
    let in_window = crate::windows::inbox_window(&skey, cx).is_some();
    let reused = in_window || ui.tile_of(&s.content()).is_some();
    let mut note = None;
    if !reused {
        let spot = match crate::web::place::spot_near(&s.content(), FilesPlace::Next(Side::Right), Some(agent.to_string()), cx) {
            Ok(spot) => spot,
            Err(e) if !explicit => return Err(e),
            Err(_) => {
                note = Some("The agent's terminal isn't shown in any open Pitwall window, so the device opened in the space Pitwall shows.".to_string());
                crate::web::place::spot_near(&s.content(), FilesPlace::Own, None, cx)?
            }
        };
        let (label, space, spot) = spot;
        super::inbox_window::dock(&skey, &label, &space, &spot, cx)?;
    } else if explicit && !in_window {
        // Its space shown in its window (nothing comes forward).
        let ui = crate::ui_state::get(cx);
        if let Some((space, _)) = ui.tile_of(&Content::Device(key.to_string())) {
            if let Some(h) = crate::windows::main_window(ui.window_of_space(&space), cx) {
                let _ = h.update(cx, |view, _, cx| {
                    if let Some(sc) = view.screen() {
                        sc.update(cx, |sc, cx| {
                            if sc.current_space().map(|c| c.id) != Some(space.clone()) {
                                sc.show_space(&space, cx);
                            }
                        });
                    }
                });
            }
        }
    }
    Ok((super::inbox_window::described(&skey, cx), reused, note))
}

/// `show_device`'s end: device `d` (found, and up) beside `agent`.
pub fn show_for_agent(d: &Device, agent: &str, waited: bool, cx: &mut App) -> Result<DeviceShown, ErrorBody> {
    if !super::device_tile::enabled(cx) || !show_enabled(cx) {
        return Err(ErrorBody::new(code::OFF, OFF));
    }
    let key = d.key();
    super::device_tile::learn(std::slice::from_ref(d), cx);
    if cx.has_global::<Auto>() {
        cx.global_mut::<Auto>().watch.shown(&key);
    }
    let in_front = crate::platform::app_in_front(cx);
    let (at, reused, note) = show_beside(&key, agent, true, cx).map_err(|e| ErrorBody::new(code::OTHER, e))?;
    let skey = Surface::Device(key.clone()).key();
    let window = if crate::windows::inbox_window(&skey, cx).is_some() {
        super::inbox_window::title(&skey, cx)
    } else {
        crate::web::place::holder_of(&Content::Device(key.clone()), cx).map(|(l, _, _)| l).unwrap_or_default()
    };
    Ok(DeviceShown { window, in_background: !in_front, device: key, name: d.name.clone(), at, note, reused, waited })
}

/// What `show_device` says while it is off.
pub const OFF: &str = "Showing devices is turned off in Pitwall (Settings → Agents → “Agents can show you a device”, agents.showDevice; or device screens, general.devicePreviews). Nothing was opened.";
