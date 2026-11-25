//! The devices there are: `adb devices -l` (Android, checked against adb
//! 36.0.2) and `xcrun simctl list devices -j` (iOS Simulators, checked
//! against Xcode 26.1.1). Parsers only; running them is
//! [`super::android`] and [`super::ios`].

use super::{Device, Platform, State};

/// `adb devices -l`'s output.
///
/// ```text
/// List of devices attached
/// emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1
/// 0A1B2C3D4E             unauthorized usb:1-1 transport_id:2
/// ```
pub fn parse_adb(out: &str) -> Vec<Device> {
    let mut found = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(serial), Some(state)) = (words.next(), words.next()) else { continue };
        let props: Vec<(&str, &str)> = words.filter_map(|w| w.split_once(':')).collect();
        let prop = |k: &str| props.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        let virtual_device = serial.starts_with("emulator-");
        let name = prop("model").map(|m| m.replace('_', " ")).unwrap_or_else(|| serial.to_string());
        let state = match state {
            "device" => State::Ready,
            "unauthorized" => State::NotReady("allow USB debugging on the device".into()),
            "offline" => State::NotReady("offline".into()),
            "no" => State::NotReady("no permission to use this USB device".into()),
            other => State::NotReady(other.to_string()),
        };
        found.push(Device { platform: Platform::Android, id: serial.to_string(), name, virtual_device, state });
    }
    found
}

/// `xcrun simctl list devices -j`'s output: available iOS simulators,
/// booted ones first, then by name.
pub fn parse_simctl(json: &str) -> Result<Vec<Device>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("simctl's list isn't JSON: {e}"))?;
    let runtimes = v.get("devices").and_then(|d| d.as_object()).ok_or("simctl's list has no devices")?;
    let mut found = Vec::new();
    for (runtime, devices) in runtimes {
        // com.apple.CoreSimulator.SimRuntime.iOS-26-1 → "iOS 26.1".
        let Some(os) = runtime.rsplit('.').next() else { continue };
        if !os.starts_with("iOS-") {
            continue;
        }
        let os = os.replacen('-', " ", 1).replace('-', ".");
        for d in devices.as_array().into_iter().flatten() {
            if d.get("isAvailable").and_then(|a| a.as_bool()) == Some(false) {
                continue;
            }
            let (Some(udid), Some(name)) = (d.get("udid").and_then(|u| u.as_str()), d.get("name").and_then(|n| n.as_str())) else { continue };
            let state = match d.get("state").and_then(|s| s.as_str()) {
                Some("Booted") => State::Ready,
                Some("Shutdown") => State::NotReady("shut down".into()),
                Some(other) => State::NotReady(other.to_lowercase()),
                None => State::NotReady("unknown".into()),
            };
            found.push(Device { platform: Platform::Ios, id: udid.to_string(), name: format!("{name} ({os})"), virtual_device: true, state });
        }
    }
    found.sort_by(|a, b| (a.state != State::Ready, &a.name).cmp(&(b.state != State::Ready, &b.name)));
    Ok(found)
}
