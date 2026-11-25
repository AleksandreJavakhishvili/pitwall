use std::io::{Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pitwall_proto::device::video::{codec, session, Packet, CODEC_H264};

use super::android::{self, Adb, Android, Child, Socket};
use super::geometry::{fit, to_pixel, to_screen, Rect, Turn};
use super::list::{parse_adb, parse_simctl};
use super::scrcpy::{self, Control, ServerOptions, ServerSource, TouchAction};
use super::stream::{Decode, Event, Input, Key, Phase, Session};
use super::*;

// --- listing -----------------------------------------------------------------

#[test]
fn adb_devices_are_read_with_their_state_and_model() {
    let out = "List of devices attached\n\
        emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\n\
        0A1B2C3D4E             unauthorized usb:1-1 transport_id:2\n\
        192.168.1.20:5555      offline transport_id:3\n\
        \n";
    let d = parse_adb(out);
    assert_eq!(d.len(), 3);
    assert_eq!(d[0].id, "emulator-5554");
    assert_eq!(d[0].name, "sdk gphone64 arm64");
    assert!(d[0].virtual_device);
    assert_eq!(d[0].state, State::Ready);
    assert_eq!(d[0].key(), "android:emulator-5554");
    assert_eq!(d[1].name, "0A1B2C3D4E");
    assert!(!d[1].virtual_device);
    assert_eq!(d[1].state, State::NotReady("allow USB debugging on the device".into()));
    assert_eq!(d[2].state, State::NotReady("offline".into()));
    // adb starting its server says so first; no devices is an empty list.
    assert!(parse_adb("* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\n\n").is_empty());
}

#[test]
fn simctl_lists_available_ios_simulators_booted_first() {
    let json = r#"{"devices":{
      "com.apple.CoreSimulator.SimRuntime.iOS-26-1":[
        {"udid":"AAAA","isAvailable":true,"state":"Shutdown","name":"iPhone 17"},
        {"udid":"BBBB","isAvailable":true,"state":"Booted","name":"iPhone Air"},
        {"udid":"CCCC","isAvailable":false,"state":"Shutdown","name":"Gone"}],
      "com.apple.CoreSimulator.SimRuntime.watchOS-26-0":[
        {"udid":"DDDD","isAvailable":true,"state":"Booted","name":"Watch"}]}}"#;
    let d = parse_simctl(json).unwrap();
    assert_eq!(d.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["BBBB", "AAAA"]);
    assert_eq!(d[0].name, "iPhone Air (iOS 26.1)");
    assert_eq!(d[0].state, State::Ready);
    assert_eq!(d[1].state, State::NotReady("shut down".into()));
    assert_eq!(d[0].key(), "ios:BBBB");
    assert!(parse_simctl("not json").is_err());
}

#[test]
fn simctl_says_when_each_simulator_last_booted() {
    let json = r#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-26-1":[
        {"udid":"AAAA","isAvailable":true,"state":"Booted","name":"iPhone 17","lastBootedAt":"2026-11-22T21:18:41Z"},
        {"udid":"BBBB","isAvailable":true,"state":"Shutdown","name":"iPad"}]}}"#;
    let d = super::list::parse_simctl_booted(json).unwrap();
    assert_eq!(d[0].1, Some(1_795_382_321_000));
    assert_eq!(d[1].1, None);
    assert_eq!(super::list::utc_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(super::list::utc_ms("2000-03-01T00:00:01.250Z"), Some(951_868_801_000));
    assert_eq!(super::list::utc_ms("2026-13-01T00:00:00Z"), None);
    assert_eq!(super::list::utc_ms("2026-11-22 21:18:41"), None, "UTC only, as simctl writes it");
}

/// The adb server is asked over its socket (never started): the request
/// as adb sends it, its answer read as `adb devices -l`'s.
#[test]
fn the_adb_server_is_asked_without_starting_it() {
    struct Wire {
        sent: Vec<u8>,
        answer: std::io::Cursor<Vec<u8>>,
    }
    impl Read for Wire {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.answer.read(b)
        }
    }
    impl Write for Wire {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.sent.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let list = "emulator-5554\tdevice product:sdk model:sdk_gphone64 device:emu transport_id:1\n";
    let mut w = Wire { sent: vec![], answer: std::io::Cursor::new(format!("OKAY{:04x}{list}", list.len()).into_bytes()) };
    let out = android::ask_server(&mut w).unwrap();
    assert_eq!(w.sent, b"000ehost:devices-l");
    let d = parse_adb(&out);
    assert_eq!((d[0].id.as_str(), d[0].state.clone()), ("emulator-5554", State::Ready));
    let mut w = Wire { sent: vec![], answer: std::io::Cursor::new(b"FAIL0004nope".to_vec()) };
    assert_eq!(android::ask_server(&mut w), Err("nope".into()));
}

#[test]
fn keys_name_a_platform_and_an_id() {
    assert_eq!(parse_key("android:emulator-5554"), Some((Platform::Android, "emulator-5554".into())));
    assert_eq!(parse_key("android:10.0.0.2:5555"), Some((Platform::Android, "10.0.0.2:5555".into())));
    assert_eq!(parse_key("ios:ABC"), Some((Platform::Ios, "ABC".into())));
    assert_eq!(parse_key("ios:"), None);
    assert_eq!(parse_key("web:x"), None);
}

// --- geometry ----------------------------------------------------------------

#[test]
fn the_picture_is_letterboxed_and_centred() {
    // A 1080×2400 phone in a 600×600 tile: 270 wide, centred.
    let r = fit((600., 600.), (1080, 2400), Turn::None).unwrap();
    assert_eq!(r, Rect { x: 165., y: 0., w: 270., h: 600. });
    // Turned sideways it fills the width instead.
    let r = fit((600., 600.), (1080, 2400), Turn::Right).unwrap();
    assert_eq!((r.w, r.h), (600., 270.));
    assert_eq!(fit((0., 10.), (10, 10), Turn::None), None);
    assert_eq!(fit((10., 10.), (0, 10), Turn::None), None);
}

#[test]
fn a_tile_point_maps_to_the_device_and_the_letterbox_to_nothing() {
    let tile = (600., 600.);
    let frame = (1080, 2400);
    assert_eq!(to_screen((300., 300.), tile, frame, Turn::None), Some((0.5, 0.5)));
    assert_eq!(to_screen((165., 0.), tile, frame, Turn::None), Some((0., 0.)));
    assert_eq!(to_screen((100., 300.), tile, frame, Turn::None), None);
    // The top right of the device's screen…
    let r = to_screen((434.9, 0.1), tile, frame, Turn::None).unwrap();
    assert!(r.0 > 0.99 && r.1 < 0.01);
    assert_eq!(to_pixel(r, frame), (1079, 0));
    // …turned a quarter clockwise, is at the bottom right of the picture.
    let r = fit(tile, frame, Turn::Right).unwrap();
    let p = to_screen((r.x + r.w - 0.1, r.y + r.h - 0.1), tile, frame, Turn::Right).unwrap();
    assert!(p.0 > 0.99 && p.1 < 0.01, "{p:?}");
    // Upside down: the top left of the picture is the device's bottom right.
    let r = fit(tile, frame, Turn::Half).unwrap();
    let p = to_screen((r.x, r.y), tile, frame, Turn::Half).unwrap();
    assert_eq!(p, (1., 1.));
    // A quarter anticlockwise: the picture's top left is the device's
    // top right.
    let r = fit(tile, frame, Turn::Left).unwrap();
    let p = to_screen((r.x, r.y), tile, frame, Turn::Left).unwrap();
    assert_eq!(p, (1., 0.));
    assert_eq!(Turn::Left.next(), Turn::None);
}

#[test]
fn pixels_stay_inside_the_frame() {
    assert_eq!(to_pixel((1., 1.), (1080, 2400)), (1079, 2399));
    assert_eq!(to_pixel((0., 0.), (1080, 2400)), (0, 0));
    assert_eq!(to_pixel((0.5, 0.25), (1080, 2400)), (540, 600));
}

// --- scrcpy ------------------------------------------------------------------

#[test]
fn scrcpy_control_messages_have_its_layout() {
    // INJECT_TOUCH_EVENT: 32 bytes.
    let b = Control::Touch { action: TouchAction::Down, pos: (540, 1200), size: (1080, 2400) }.encode();
    assert_eq!(b.len(), 32);
    assert_eq!(&b[0..2], &[2, 0]);
    assert_eq!(&b[2..10], &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe]);
    assert_eq!(&b[10..14], &540i32.to_be_bytes());
    assert_eq!(&b[14..18], &1200i32.to_be_bytes());
    assert_eq!(&b[18..22], &[0x04, 0x38, 0x09, 0x60]);
    assert_eq!(&b[22..24], &[0xff, 0xff]);
    let up = Control::Touch { action: TouchAction::Up, pos: (0, 0), size: (1, 1) }.encode();
    assert_eq!((up[1], &up[22..24]), (1, &[0u8, 0][..]));
    // INJECT_KEYCODE: 14 bytes; ACTION_UP is 1.
    let k = Control::Key { down: false, keycode: 66, meta: 0 }.encode();
    assert_eq!(k, [0, 1, 0, 0, 0, 66, 0, 0, 0, 0, 0, 0, 0, 0]);
    // INJECT_TEXT: a u32 length, cut at 300 bytes on a character.
    let t = Control::Text("hé".into()).encode();
    assert_eq!(t, [1, 0, 0, 0, 3, b'h', 0xc3, 0xa9]);
    let long = "é".repeat(200);
    let t = Control::Text(long).encode();
    assert_eq!(u32::from_be_bytes(t[1..5].try_into().unwrap()), 300);
    assert_eq!(Control::BackOrScreenOn { down: true }.encode(), [4, 0]);
}

#[test]
fn the_server_command_pins_the_version_and_asks_for_h264_without_audio() {
    let c = scrcpy::server_command(&ServerOptions { scid: 0x1234abcd, max_size: 1280, max_fps: 30, bit_rate: 8_000_000, control: true });
    assert!(c.starts_with("CLASSPATH=/data/local/tmp/scrcpy-server.jar app_process / com.genymobile.scrcpy.Server 5.0.1 "));
    for want in ["scid=1234abcd", "tunnel_forward=true", "audio=false", "video_codec=h264", "control=true", "max_size=1280", "max_fps=30", "send_device_meta=false"] {
        assert!(c.split(' ').any(|w| w == want), "{want} in {c}");
    }
    // A 31-bit id, as scrcpy's.
    assert_eq!(scrcpy::socket_name(0xffff_ffff), "scrcpy_7fffffff");
    // Nothing a shell would read twice (scrcpy refuses these characters).
    assert!(!c.chars().any(|ch| " ;'\"*$?&`#\\|<>[]{}()!~".contains(ch) && ch != ' '));
}

#[test]
fn device_messages_on_the_control_socket_are_measured_to_be_skipped() {
    assert_eq!(scrcpy::device_message_len(&[0, 0, 0, 0, 3, b'a']), Some(8));
    assert_eq!(scrcpy::device_message_len(&[0, 0]), None);
    assert_eq!(scrcpy::device_message_len(&[1]), Some(9));
    assert_eq!(scrcpy::device_message_len(&[2, 0, 1, 0, 4]), Some(9));
    assert_eq!(scrcpy::device_message_len(&[]), None);
}

#[test]
fn only_the_pinned_server_file_is_used() {
    let dir = std::env::temp_dir().join(format!("pw-device-{}", std::process::id()));
    let f = scrcpy::UserFile::in_data(&dir);
    let e = f.server().unwrap_err();
    assert!(e.contains("doesn't include") && e.contains("scrcpy-server-v5.0.1"), "{e}");
    std::fs::create_dir_all(f.path.parent().unwrap()).unwrap();
    std::fs::write(&f.path, b"not the server").unwrap();
    assert!(f.server().unwrap_err().contains("SHA-256"));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(scrcpy::sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

// --- input mapping -----------------------------------------------------------

#[test]
fn android_input_becomes_scrcpy_messages_in_frame_pixels() {
    let m = android::controls(&Input::Touch { phase: Phase::Move, at: (0.5, 0.5) }, (1080, 2400));
    assert_eq!(m, vec![Control::Touch { action: TouchAction::Move, pos: (540, 1200), size: (1080, 2400) }]);
    assert_eq!(android::controls(&Input::Key { down: true, key: Key::Enter }, (1, 1)), vec![Control::Key { down: true, keycode: 66, meta: 0 }]);
    assert_eq!(android::controls(&Input::Back, (1, 1)).len(), 2);
    assert_eq!(android::controls(&Input::Home, (1, 1))[0], Control::Key { down: true, keycode: 3, meta: 0 });
}

#[test]
fn ios_text_becomes_hid_keys_with_shift() {
    use pitwall_proto::device::sim::Input as S;
    let r = ios::records(&Input::Text("Hi!".into()));
    assert_eq!(
        r,
        vec![
            S::Key { down: true, usage: 0xe1 },
            S::Key { down: true, usage: 0x0b },
            S::Key { down: false, usage: 0x0b },
            S::Key { down: false, usage: 0xe1 },
            S::Key { down: true, usage: 0x0c },
            S::Key { down: false, usage: 0x0c },
            S::Key { down: true, usage: 0xe1 },
            S::Key { down: true, usage: 0x1e },
            S::Key { down: false, usage: 0x1e },
            S::Key { down: false, usage: 0xe1 },
        ]
    );
    // Characters without a key on a US layout are left out.
    assert!(ios::records(&Input::Text("é".into())).is_empty());
    assert!(ios::records(&Input::Back).is_empty());
    assert_eq!(ios::records(&Input::Key { down: true, key: Key::Backspace }), vec![S::Key { down: true, usage: 0x2a }]);
    let t = ios::records(&Input::Touch { phase: Phase::Down, at: (1.2, -0.1) });
    assert_eq!(t, vec![S::Touch { phase: pitwall_proto::device::sim::Phase::Down, x: 1.0, y: 0.0 }]);
}

// --- lifecycle (fakes) -------------------------------------------------------

/// A decoder that names each frame by its first payload byte.
struct FakeDecoder;

impl Decode for FakeDecoder {
    type Picture = u8;
    fn config(&mut self, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn decode(&mut self, data: &[u8], _: u64) -> Result<Option<u8>, String> {
        Ok(data.last().copied())
    }
}

fn stream_bytes(frames: &[u8]) -> Vec<u8> {
    let mut s = codec(CODEC_H264).to_vec();
    s.extend(session(1080, 2400));
    s.extend(Packet { config: true, key: false, pts: 0, data: vec![0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x68] }.encode());
    for (i, f) in frames.iter().enumerate() {
        s.extend(Packet { config: false, key: i == 0, pts: i as u64, data: vec![0, 0, 0, 1, 0x65, *f] }.encode());
    }
    s
}

/// Video that blocks until its sender goes (the server ended).
struct Blocking(mpsc::Receiver<Vec<u8>>, Vec<u8>);

impl Read for Blocking {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1.is_empty() {
            match self.0.recv() {
                Ok(b) => self.1 = b,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.1.len());
        buf[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}

#[derive(Clone, Default)]
struct Bytes(Arc<Mutex<Vec<u8>>>);

impl Write for Bytes {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// adb as the tests see it: what was run, a server whose video is a
/// channel (killing it closes the channel), a control socket that records.
struct FakeAdb {
    log: Arc<Mutex<Vec<String>>>,
    video: Mutex<Option<mpsc::Receiver<Vec<u8>>>>,
    /// The server's end of the video: dropped when the server is killed.
    server: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
    control: Bytes,
    connects: Mutex<u32>,
}

struct FakeChild(Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>, Arc<Mutex<Vec<String>>>);

impl Child for FakeChild {
    fn kill(&mut self) {
        self.1.lock().unwrap().push("kill server".into());
        *self.0.lock().unwrap() = None;
    }
}

impl FakeAdb {
    fn new() -> FakeAdb {
        let (tx, rx) = mpsc::channel();
        FakeAdb { log: Arc::default(), video: Mutex::new(Some(rx)), server: Arc::new(Mutex::new(Some(tx))), control: Bytes::default(), connects: Mutex::new(0) }
    }
    fn send(&self, b: Vec<u8>) {
        if let Some(tx) = self.server.lock().unwrap().as_ref() {
            let _ = tx.send(b);
        }
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Adb for FakeAdb {
    fn run(&self, serial: Option<&str>, args: &[&str]) -> Result<String, String> {
        self.log.lock().unwrap().push(format!("{} {}", serial.unwrap_or("-"), args.join(" ")));
        Ok(String::new())
    }
    fn shell(&self, serial: &str, command: &str) -> Result<Box<dyn Child>, String> {
        self.log.lock().unwrap().push(format!("{serial} shell {}", command.split(' ').nth(4).unwrap_or("")));
        Ok(Box::new(FakeChild(self.server.clone(), self.log.clone())))
    }
    fn free_port(&self) -> Result<u16, String> {
        Ok(27183)
    }
    fn connect(&self, _port: u16) -> std::io::Result<Socket> {
        let mut n = self.connects.lock().unwrap();
        *n += 1;
        if *n == 1 {
            // The video socket: the dummy byte, then the stream.
            let rx = self.video.lock().unwrap().take().unwrap();
            Ok((Box::new(Blocking(rx, vec![0])), Box::new(std::io::sink())))
        } else {
            Ok((Box::new(std::io::empty()), Box::new(self.control.clone())))
        }
    }
}

struct Checked;

impl ServerSource for Checked {
    fn server(&self) -> Result<std::path::PathBuf, String> {
        Ok("/tmp/scrcpy-server-v5.0.1".into())
    }
}

fn phone() -> Device {
    Device { platform: Platform::Android, id: "emulator-5554".into(), name: "Pixel".into(), virtual_device: true, state: State::Ready }
}

fn backend(adb: Arc<FakeAdb>) -> Arc<Android> {
    Arc::new(Android {
        adb,
        server: Arc::new(Checked),
        options: ServerOptions { scid: 7, max_size: 0, max_fps: 0, bit_rate: 8_000_000, control: true },
        patience: Duration::from_secs(1),
    })
}

type Seen = Arc<Mutex<Vec<String>>>;

fn start(adb: Arc<FakeAdb>) -> (Session, Seen, mpsc::Receiver<()>) {
    let seen: Seen = Arc::default();
    let (tx, rx) = mpsc::channel();
    let s2 = seen.clone();
    let tx = Mutex::new(tx);
    let s = Session::start(backend(adb), phone(), FakeDecoder, move |e| {
        s2.lock().unwrap().push(match e {
            Event::Connected { input } => format!("connected input={}", input.is_ok()),
            Event::Size(w, h) => format!("size {w}x{h}"),
            Event::Picture(p) => format!("picture {p}"),
            Event::Ended(why) => format!("ended {why}"),
        });
        let _ = tx.lock().unwrap().send(());
    });
    (s, seen, rx)
}

fn wait_for(seen: &Seen, rx: &mpsc::Receiver<()>, what: &str) {
    while !seen.lock().unwrap().iter().any(|s| s.starts_with(what)) {
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| panic!("no {what}: {:?}", seen.lock().unwrap()));
    }
}

#[test]
fn an_android_tile_connects_streams_and_sends_touches() {
    let adb = Arc::new(FakeAdb::new());
    let (s, seen, rx) = start(adb.clone());
    adb.send(stream_bytes(&[1, 2, 3]));
    wait_for(&seen, &rx, "picture 3");
    assert_eq!(&seen.lock().unwrap()[..5], ["connected input=true", "size 1080x2400", "picture 1", "picture 2", "picture 3"]);
    assert_eq!(
        adb.log(),
        [
            "emulator-5554 push /tmp/scrcpy-server-v5.0.1 /data/local/tmp/scrcpy-server.jar",
            "emulator-5554 forward tcp:27183 localabstract:scrcpy_00000007",
            "emulator-5554 shell 5.0.1",
        ]
    );
    // A touch in the middle reaches the control socket in frame pixels.
    s.send(&Input::Touch { phase: Phase::Down, at: (0.5, 0.5) }).unwrap();
    let sent = adb.control.0.lock().unwrap().clone();
    assert_eq!(sent, Control::Touch { action: TouchAction::Down, pos: (540, 1200), size: (1080, 2400) }.encode());
    drop(s);
}

#[test]
fn closing_the_tile_ends_the_server_and_removes_the_forward_once() {
    let adb = Arc::new(FakeAdb::new());
    let (s, seen, rx) = start(adb.clone());
    adb.send(stream_bytes(&[1]));
    wait_for(&seen, &rx, "picture 1");
    drop(s);
    let log = adb.log();
    assert_eq!(&log[3..], ["kill server", "emulator-5554 forward --remove tcp:27183"]);
    // The reader thread ends on its own; nothing more is reported and
    // nothing is undone twice.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(adb.log().len(), 5);
    assert!(!seen.lock().unwrap().iter().any(|s| s.starts_with("ended")));
}

#[test]
fn a_device_that_goes_away_ends_the_stream_and_cleans_up() {
    let adb = Arc::new(FakeAdb::new());
    let (s, seen, rx) = start(adb.clone());
    adb.send(stream_bytes(&[1]));
    wait_for(&seen, &rx, "picture 1");
    // The device is unplugged: its server's stream ends.
    *adb.server.lock().unwrap() = None;
    wait_for(&seen, &rx, "ended");
    assert_eq!(seen.lock().unwrap().last().unwrap(), "ended The device went away");
    assert_eq!(&adb.log()[3..], ["kill server", "emulator-5554 forward --remove tcp:27183"]);
    // Closing the tile afterwards undoes nothing twice.
    drop(s);
    assert_eq!(adb.log().len(), 5);
}

#[test]
fn a_missing_server_file_is_said_and_nothing_runs_on_the_device() {
    struct Missing;
    impl ServerSource for Missing {
        fn server(&self) -> Result<std::path::PathBuf, String> {
            Err("Android screens need scrcpy-server".into())
        }
    }
    let adb = Arc::new(FakeAdb::new());
    let b = Arc::new(Android { adb: adb.clone(), server: Arc::new(Missing), options: ServerOptions { scid: 1, max_size: 0, max_fps: 0, bit_rate: 1, control: true }, patience: Duration::from_millis(10) });
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let _s = Session::start(b, phone(), FakeDecoder, move |e| {
        if let Event::Ended(why) = e {
            let _ = tx.lock().unwrap().send(why);
        }
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "Android screens need scrcpy-server");
    assert!(adb.log().is_empty());
}

#[test]
fn a_broken_stream_ends_with_its_reason() {
    let adb = Arc::new(FakeAdb::new());
    let (_s, seen, rx) = start(adb.clone());
    adb.send(codec(u32::from_be_bytes(*b"h265")).to_vec());
    wait_for(&seen, &rx, "ended");
    assert!(seen.lock().unwrap().last().unwrap().contains("not H.264"));
    assert_eq!(adb.log().last().unwrap(), "emulator-5554 forward --remove tcp:27183");
}

/// Against a real simulator (not run by default): a booted simulator's
/// UDID in `PITWALL_TEST_SIM`, its device set in `PITWALL_TEST_SIM_SET`
/// (optional), and `pitwall-simhost` built. Streams for a moment through
/// the real helper and decoder and checks pictures came.
#[test]
#[ignore]
fn a_real_simulator_streams_into_pictures() {
    let Some(udid) = std::env::var_os("PITWALL_TEST_SIM") else { return };
    let helper = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/pitwall-simhost");
    let b = Arc::new(ios::Simulator { helper: Some(helper), set: std::env::var_os("PITWALL_TEST_SIM_SET").map(Into::into), patience: Duration::from_secs(20) });
    let d = Device { platform: Platform::Ios, id: udid.to_string_lossy().into(), name: "sim".into(), virtual_device: true, state: State::Ready };
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let s = Session::start(b, d, crate::platform::video::Decoder::new().unwrap(), move |e| {
        let _ = tx.lock().unwrap().send(match e {
            Event::Connected { input } => format!("connected input={}", input.is_ok()),
            Event::Size(w, h) => format!("size {w}x{h}"),
            Event::Picture(p) => format!("picture {}x{}", p.width, p.height),
            Event::Ended(why) => format!("ended {why}"),
        });
    });
    let mut got = Vec::new();
    while let Ok(e) = rx.recv_timeout(Duration::from_secs(20)) {
        got.push(e);
        if got.iter().filter(|e| e.starts_with("picture")).count() >= 3 {
            break;
        }
    }
    eprintln!("{got:?}");
    assert_eq!(got[0], "connected input=true");
    assert!(got[1].starts_with("size "));
    assert!(got.iter().filter(|e| e.starts_with("picture")).count() >= 3);
    drop(s);
}
