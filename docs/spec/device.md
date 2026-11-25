# Device tiles

A phone's screen in a tile of a space's layout, next to the agent's
terminal: an Android emulator or a phone on a cable, or an iOS Simulator.
You watch the app the agent is building and tap and type into it, without
an emulator or Simulator window. Agents never see or drive the screen.

Status: built (GPUI, macOS). The iOS Simulator path runs end to end on
this machine; the Android path is built and tested only against a fake adb
(no device and no scrcpy-server were available, see "How it was
checked"). A device an agent starts opens beside it on its own, and agents
have `show_device`; both are built (see below), checked with fakes and one
real headless simulator boot, not yet seen on screen.

Settings: `general.devicePreviews` (on by default). Off: no device tile
opens, none is offered, nothing is listed. `general.deviceAutoOpen`
(`beside`, the default; `offer`; `off`) and `agents.showDevice` (on by
default) below.

## What the user sees
- **Open device…** in the command palette (⌘K): one row per device that can
  be shown now, "Open device: Pixel 8 (Android emulator)", "Open device:
  iPhone 17 (iOS 26.1) (iOS Simulator)". The list is made in the background
  each time the palette opens (`adb devices -l`, `xcrun simctl list devices
  -j`); with none there is a single "Open device…" row that says how to
  get one.
- The tile opens right of the focused tile (in it when that tile is empty),
  like a web tile. It has the tiles' header: grip (drag to move, or out of
  the window for a window of its own), the device's name, its state
  (connecting, live, view only, stopped: the reason in the tooltip), Back
  (Android) and Home, Connect again (when stopped), Move…, Maximise (⌘⏎),
  a window of its own, Close.
- The body is the screen, letterboxed in black. A click is a tap, a drag is
  a swipe, typing goes to the device while the tile has the keyboard
  (Pitwall's own ⌘/Ctrl shortcuts stay Pitwall's). Enter, Backspace,
  Delete, Tab, Esc, arrows and Space are keys; other characters are text.
- Moved, maximised, in a window of its own and back as the other tiles;
  after a restart the tile is where it was (`ui.json`: the pane's
  `"device": "android:<serial>"` / `"ios:<udid>"`; windows in
  `deviceWindows`) and connects again. A device that is gone says so in
  the tile ("The device went away"), with Connect again.
- Closing the tile stops the stream and undoes everything it set up (the
  adb forward and scrcpy-server on Android, the helper process on iOS).

## Architecture
```
 palette / ui.json ─► DeviceTile (main_screen/device_tile.rs, UI thread)
                          │  Session::start(backend, device, decoder)
                          ▼
            device::stream::Session (its own thread)
              Backend::connect ─► Connection { video, input, stop }
              video bytes ─► proto::device::video::Reader (framing)
                         ─► platform::video::Decoder (VideoToolbox)
                         ─► Event::Picture(Frame{CVPixelBuffer 420f})
                          │  futures channel, newest picture kept
                          ▼
            DeviceTile::render ─► gpui::surface(buffer)   (macOS only)
            mouse/keys ─► geometry::to_screen ─► Session::send ─► InputSink
```
- `crates/pitwall-proto/src/device/video.rs`: the stream's framing (codec
  id, session packets, media packets), shared by the Android reader and the
  iOS helper's writer; H.264 helpers (NAL split, AVCC ⇄ Annex B).
- `crates/pitwall-proto/src/device/sim.rs`: the helper's input records
  (24 bytes) and its status lines (JSON on stderr).
- `crates/pitwall-app/src/device/`: `list.rs` (adb and simctl parsers),
  `geometry.rs` (tile point → device), `scrcpy.rs` (server command, control
  messages, the pinned server file), `stream.rs` (Session, traits),
  `android.rs`, `ios.rs`, `tests.rs`.
- `crates/pitwall-app/src/platform/video.rs` + `video_macos.rs`: the
  decoder.
- `crates/pitwall-simhost`: the iOS helper, a separate macOS-only binary.
- Both backends sit behind `Backend`; adb behind `Adb`; the server file
  behind `ServerSource`; decoding behind `Decode`. Tests use fakes for all
  four.

## Android
Frames and input come from **scrcpy-server v5.0.1**, run on the device
over adb, exactly as the scrcpy client does it:
1. `adb -s S push <file> /data/local/tmp/scrcpy-server.jar`
2. `adb -s S forward tcp:P localabstract:scrcpy_<scid>` (scid: 31 random
   bits, `%08x`)
3. `adb -s S shell CLASSPATH=/data/local/tmp/scrcpy-server.jar app_process /
   com.genymobile.scrcpy.Server 5.0.1 scid=… log_level=warn
   tunnel_forward=true audio=false video_codec=h264 max_size=1600
   max_fps=60 video_bit_rate=8000000 control=true send_device_meta=false
   cleanup=true power_on=true`
4. Connect to `localhost:P` until the server's dummy byte (`0x00`) comes
   (every 100 ms, as scrcpy; 10 s at most). That is the video socket.
5. Connect again: the control socket. Device messages on it (clipboard,
   acks) are read and dropped.
6. Video: `u32` codec id (`h264`; `1` means the encoder failed), then
   12-byte session packets (`0x80000000|flags`, width, height) and media
   packets (`u64` pts with bit 63 = session, 62 = config, 61 = key frame;
   `u32` size; data). A rotation sends a new session packet and config.
7. Input: `INJECT_TOUCH_EVENT` (32 bytes, pointer id −2 "generic finger",
   position in the current video size, pressure u16 fixed point),
   `INJECT_KEYCODE` (14 bytes), `INJECT_TEXT` (u32 length + UTF-8, at most
   300 bytes), `BACK_OR_SCREEN_ON` (2 bytes).
8. Stop: kill the `adb shell`, `adb forward --remove tcp:P`. The server
   also cleans up after itself (`cleanup=true`).

adb is found in `$ANDROID_HOME`, `$ANDROID_SDK_ROOT`, the SDK's usual places
(`~/Library/Android/sdk`, `~/Android/Sdk`, `%LOCALAPPDATA%\Android\Sdk`),
then `PATH`. Device states map to plain words: "unauthorized" → "allow USB
debugging on the device", "no permissions" → the udev hint, "offline".

### Where scrcpy-server comes from (open: see "Decisions")
Pitwall does not ship or download it. `ServerSource` is the seam; the one
built (`UserFile`) uses `<data>/device/scrcpy-server-v5.0.1` only when its
SHA-256 is `764eb6f79811d5211fe9df341120882ba9994c7a61b897d7bf3fb662e53bc536`
(the published hash for v5.0.1), and otherwise says where to put it. The
server refuses any other version than the one in the command, so the
version and hash are pinned together (`device/scrcpy.rs`).

### Without scrcpy-server (designed, not built)
- **screenrecord + `input`**: `adb exec-out screenrecord
  --output-format=h264 -` is raw H.264 on stdout (Android 5+); input
  through `adb shell input tap|swipe|text|keyevent` (and `motionevent
  DOWN|MOVE|UP` from Android 10). Nothing to install. Costs: a 3-minute
  limit before Android 14 (`--time-limit 0` lifts it only from 14), so a
  restart every 3 minutes with a visible hitch; rotation is letterboxed
  into the first size; each `input` is a new process (~100–300 ms, so
  drags are coarse). A fair fallback, not the main path.
- **Emulator gRPC** (`-grpc`, token in `pid_*.ini`): `streamScreenshot`
  gives PNG/RGBA frames (no encoded video; that is its WebRTC path) and
  `sendTouch`/`sendKey` input. Emulators only.

## iOS Simulator
Neither `simctl` nor any shipping tool gives frames and touches for a
headless simulator: `simctl io screenshot` is one still (~0.27 s here),
`simctl io recordVideo` writes only a file, and there is no `simctl` input
at all. So, as the owner decided, Pitwall has its own helper:
**`pitwall-simhost`**, a separate macOS-only binary (not linked into the
app or core) that loads CoreSimulator and SimulatorKit **at run time from
the user's Xcode** (never bundled) and:
- reads the booted device's main display IOSurface (no Simulator.app
  window, no Screen Recording permission), encodes it with VideoToolbox
  H.264 (real time, no B-frames, key frame at least every 4 s) and writes
  the same framing as Android to stdout;
- reads 24-byte input records (touch, HID key, hardware button) on stdin
  and sends them as Indigo HID messages through SimulatorKit;
- says on stderr, as JSON lines, `{"event":"ready","width":…,"height":…,
  "input":true|false,"why":…,"xcode":"26.1.1"}` or
  `{"event":"stopped","why":…}`.

`pitwall-simhost stream <udid> [--set <dir>] [--fps n] [--seconds n]
[--input <file>] [--raw]`, `pitwall-simhost probe`. It ends when stdin
closes (the app closed the tile or quit), when the device shuts down, or on
an error. The app finds it next to its own binary (the bundle's
`Contents/MacOS`, added to `scripts/package-app.sh`'s sidecars) or at
`$PITWALL_SIMHOST`.

**Failing soft.** Without SimulatorKit, or without one of its symbols, the
helper still streams with `"input":false` and the reason; the tile says
"view only". Without CoreSimulator, or for an unknown device, it stops
with the reason; the tile shows it. Any crash (an Objective-C exception
from a private call aborts the process) ends only the helper; the tile
shows "stopped" with Connect again. The app never loads these frameworks.

### Private interfaces used
Verified on **Xcode 26.1.1 (17B100), CoreSimulator 1051.9.4, iOS 26.1
runtime**, macOS 26 (Darwin 25.5). No other Xcode was tried.

CoreSimulator (`/Library/Developer/PrivateFrameworks/CoreSimulator.framework`,
else Xcode's copy):
- `SimServiceContext +sharedServiceContextForDeveloperDir:error:`
- `-defaultDeviceSetWithError:`, `-deviceSetWithPath:error:`
- `SimDeviceSet -devices`
- `SimDevice -UDID`, `-name`, `-state` (3 = booted), `-io`
- `SimDeviceIOClient -ioPorts`; each port `-descriptor` (a `ROCKRemoteProxy`)
  → `-state`, `-displayClass` (u16, 0 = main display),
  `-framebufferSurface` (an IOSurface)

SimulatorKit (`<developer dir>/Library/PrivateFrameworks/SimulatorKit.framework`):
- class `SimulatorKit.SimDeviceLegacyHIDClient`: `-initWithDevice:error:`,
  `-sendWithMessage:freeWhenDone:completionQueue:completion:`
- `IndigoHIDMessageForKeyboardArbitrary(usage, down)` (HID page 7 usages)
- `IndigoHIDMessageForButton(button, down, target 0x33)` (Home 0, Lock 1,
  Siri 0x400002)
- Touch: the message `IndigoHIDMessageForMouseNSEvent` makes is
  rebuilt by hand, 0x140 bytes: inner size 0x90 at 0x18, event type 2 at
  0x1c, kind 0xB at 0x20, timestamp at 0x24, the touch (0x70 bytes) at
  0x30 with x and y ratios (f64) at 0x3c and 0x44, target 0x32, down 1 / up
  2; the payload repeated at 0xB0 with fields 1 and 2 at 0xC0 and 0xC4.

The calls through `ROCKRemoteProxy` and the struct-typed `sendWithMessage:`
go through raw `objc_msgSend` (objc2's debug encoding check can't see
proxied methods).

Which interfaces to call and the touch layout were learned by reading idb
(FBSimulatorControl, MIT, Meta); no code was copied (NOTICE).

### Risk: newer Xcodes
idb notes that from CoreSimulator 1155.4 (a later Xcode than this one)
Indigo keyboard and button messages are dropped and Indigo touch becomes
unreliable; their replacement is the DTUHID path. On such an Xcode the
helper's input may silently do nothing while it reports `input:true`. Not
verified (no such Xcode here). Plan: check the CoreSimulator version at
start and report "view only, input needs an update" above a version known
to work, until the DTUHID path is written and verified.

## Drawing
GPUI's `surface(CVPixelBuffer)` draws a decoded frame with no copy. It
exists on macOS only, and the Metal renderer accepts only
`kCVPixelFormatType_420YpCbCr8BiPlanarFullRange` (`420f`); the decoder asks
VideoToolbox for exactly that, IOSurface-backed and Metal-compatible. The
Linux (wgpu) and Windows (DirectX) renderers ignore surfaces, so elsewhere
frames would go through `RenderImage`/`img` with `cx.drop_image` for each
replaced frame (a CPU copy per frame).

A surface can't be rotated in GPUI. When the device turns, both backends
send a new size and the picture follows; a manual "turn the picture"
(`geometry::Turn`, mapped and tested) is left unused until there is a
rotated draw.

## Opening on its own
The owner: "when agent starts the simulator to open here". When an agent
starts an iOS Simulator or an Android emulator (or a phone comes online
while it installs), the device's tile opens right of that agent's
terminal, as a web tile does, without the keyboard and without bringing
Pitwall forward. `general.deviceAutoOpen`: `beside` (default), `offer`
(only the notice below) or `off` (the palette still lists it).

**Signals** (`crates/pitwall-core/src/devices.rs`), the tools' own, nothing
an agent has to do for Pitwall:
- *A launcher in the agent's process tree*: `xcrun simctl boot|bootstatus|
  install|launch|openurl`, `xcodebuild … Simulator…`, `xcrun devicectl`,
  `open -a Simulator`, `flutter run` (its script or the Dart snapshot),
  `expo run:ios|run:android` / `expo start --ios|--android`,
  `react-native run-ios|run-android`, `emulator` (and its `qemu-system-*
  -avd`), `adb install|install-multiple|wait-for-device`, gradle's
  `install<Variant>` tasks. Looking (`simctl list`, `adb devices`) is not
  starting. Found among the descendants of the agent's process in one
  `ps -axww` of the whole table (the same parser as the "Elsewhere" scan),
  read only when some agent's output changed since the last look or a
  launcher was running then.
- *A line its terminal printed* in the last two minutes, read by the
  terminal's existing line scanner (the one that finds dev-server
  addresses): the command as the agent's screen echoes it (`simctl boot
  "iPhone 17"`, `emulator -avd Pixel_8`, `adb -s emulator-5554 install`,
  `platform=iOS Simulator,name=…`, `npx expo run:ios`, `flutter run`),
  flutter's "Launching … on iPhone 17 in debug mode", Expo's "Opening on
  …", React Native's "Found booted …", gradle's "Installing APK … on '…'",
  an `emulator-NNNN` serial. History replayed when a terminal is attached
  again doesn't count.

**Looking cheaply** (`crates/pitwall-app/src/main_screen/device_auto.rs`,
every 1.5 s): a platform's devices are listed only while some agent gives
a signal for it, and for three minutes after the last one. iOS: `xcrun
simctl list devices -j` (each simulator's state and `lastBootedAt`).
Android: the adb server's own socket (`host:devices-l` on 127.0.0.1:5037,
or `ANDROID_ADB_SERVER_PORT`), so Pitwall never starts the server to look;
no server, no device online.

**What counts as a boot** (`crates/pitwall-app/src/device/auto.rs`): the
first listing of a watch is the baseline; a device that comes online after
it is a boot. A simulator that booted at most 30 s before the watch began
(a quick `simctl boot` that finished before the first listing) counts too,
by its `lastBootedAt`.

**Whose it is**, strongest first:
1. the agent asked (`show_device`, below): nothing more happens for that
   boot;
2. exactly one agent had a launcher for that system running from two
   minutes before the boot to 15 s after it;
3. several did: the one whose printed line names this device, else the
   only one of them that printed a device line;
4. none did: the only agent whose line names it, else the only agent that
   printed a device line.
A line naming another device doesn't count. Anything else is nobody's.

**What happens**: someone's boot opens beside that agent (`beside`); a
boot that is nobody's, any boot with `offer`, or one whose agent's
terminal isn't in an open window gets a notice in the main window: "iPhone
17 started", "<agent> started it. Open its screen in Pitwall…", with
**Open in Pitwall** (beside that agent, else in the space shown) and **Not
now**. A device whose tile is already open stays where it is (no second
tile). Each boot is acted on once: a tile the user closes doesn't come back
for the same boot (a later boot of the same device is a new one).

**Simulator.app**: `open -a Simulator`, Xcode, `flutter run` and `expo run`
usually open Simulator.app's window too; Pitwall leaves it alone (never
closes or hides it). The tile works alongside it. `xcrun simctl boot`
alone keeps the simulator headless: no window but the tile.

Limits: a boot by the user's own hand while an agent happens to run a
launcher for that system is taken as the agent's. A TUI that repaints an
old command line makes that line count again for two minutes. Signals come
only from agents on this computer. A launcher that lives less than the
1.5 s between looks (a plain `simctl boot`) is seen through its echoed
command line (the agent's screen shows it) or not at all; the boot is then
nobody's and only offered.

## Agents: `show_device`
The owner, again: "or when I ask show em on simulator to open on pitwall".
Beside `show_web` and `show_settings` in Pitwall's `pitwall` MCP server:
`show_device { device?: string }`, and `pitwall open --device [<name|udid|
serial>]` from a shell (`device.open`). The device is named as simctl or
adb lists it ("iPhone 17", "iPhone 17 (iOS 26.1)"), by UDID, adb serial or
key (`ios:<udid>`, `android:<serial>`), case aside; a part of a name works
when it fits one booted device. None named: the one booted device; several
booted: an error that lists them. A device still booting (or, none named,
none booted yet) is waited for, listing again each second, up to 60 s; a
name nothing has yet (an emulator not up), up to 20 s; then a plain error
that says what there is.

It opens that device's tile right of the agent's terminal (in the space
Pitwall shows when that terminal isn't in an open window, said so), or
brings the tile's space into view where it already is. It only shows: the
answer says which device and where (`device`, `name`, `shownIn`, `reused`,
`waited`), never pixels or state, and there is no input tool. Never an
extension's (as Settings).

When to use it is in the tool's description and the server's instructions
(the same place `show_web`'s are): when the user asks to see or run the
app on a simulator or emulator ("show me on the simulator", "run it on the
simulator"), boot or start one with the tools the agent normally uses if
none is running, then call `show_device`. Agents get no new ability: they
already boot devices with simctl, flutter, expo; the tool only shows.

`agents.showDevice` (needs `general.devicePreviews`) removes the tool and
makes `pitwall open --device` refuse. Agents that show files through the
built-in rule (Aider) get a line about `open --device`.

## A "Preview device" offer in the terminal's header (not built)
Designed before opening on its own was: the header offers "Preview
<device>" when the terminal prints a device line. The notice above covers
the same moment; a header chip would add a second place for it. Left out.

## Other systems (designed, not built)
- **Windows**: Media Foundation's H.264 decoder (part of Windows, no extra
  licence), into NV12/BGRA, drawn as a `RenderImage`.
- **Linux**: VA-API where there is a GPU decoder; else openh264 (BSD code;
  Cisco's binary covers the patent licence only if downloaded from Cisco
  at run time) or ffmpeg's decoder via the system's libavcodec. All need a
  CPU copy into a `RenderImage` per frame.
- iOS Simulators exist only on macOS. Android works the same everywhere
  once a decoder exists. Today `Decoder::new()` there says "Device screens
  need a video decoder Pitwall doesn't have on this system yet".

## How it was checked
Run on this machine (Xcode 26.1.1, a private device set so the user's
Simulator.app and devices were untouched; booted headless with `xcrun
simctl --set <dir> bootstatus -b`, shut down and deleted afterwards):
- `pitwall-simhost stream <udid> --set <dir> --seconds 3 > out.h264`:
  23 frames at 1206×2622; `ffprobe` decoded them.
- Input through `--input`: a tap at (0.85, 0.483) opened Settings; HID
  usages 0x17 0x08 0x16 0x17 typed "Test" in its search field; Home
  returned to the home screen (checked with `simctl io screenshot`).
- `PITWALL_TEST_SIM=<udid> PITWALL_TEST_SIM_SET=<dir> cargo test -p
  pitwall-app --lib device::tests::a_real_simulator -- --ignored`: the
  app's Session with the real helper and the real VideoToolbox decoder:
  connected with input, size 1206×2622, three pictures, no helper left
  after the session was dropped.
- `simctl io screenshot -` (stdout) and its ~0.27 s per still; `simctl io
  recordVideo` writing only to a file; no `simctl` input command.
- The decoder: `platform::video` test decodes an ffmpeg-made H.264 file to
  three 96×64 `420f` buffers.

Read in sources, not run:
- scrcpy v5.0.1 (tag sources: `Server.java`, `Options.java`,
  `DesktopConnection.java`, `Streamer.java`, `ControlMessageReader.java`,
  `DeviceMessageWriter.java`; client `server.c`, `demuxer.c`,
  `control_msg.c`): everything in "Android" steps 1–8. The SHA-256 from the
  release's `SHA256SUMS.txt` and `doc/build.md`; the file itself was not
  downloaded. Licence Apache-2.0, no NOTICE file in the repository.
- AOSP `screenrecord` (`--output-format=h264`, the time limit and its
  Android 14 change) and `input` (`motionevent` from Android 10).
- The emulator's `emulator_controller.proto` (`streamScreenshot`, image
  formats, `sendTouch`, `sendKey`). Whether gRPC is on by default in
  current emulators: **unverified**.
- GPUI (vendored `third_party/zed`): `surface()` macOS only, the `420f`
  check in the Metal renderer, wgpu/DirectX ignoring surfaces.
- ScreenCaptureKit (Apple's documentation): window capture needs the
  Screen Recording permission; `SCContentSharingPicker` (macOS 14) avoids
  the global one. The monthly re-confirmation in macOS 15 is reported by
  the press only: **unverified**.
- idb's sources, for the private interfaces above (then verified by
  running).

Opening on its own and `show_device`, run here:
- `PITWALL_TEST_SIM_SET=<dir> PITWALL_TEST_SIM=<udid> cargo test -p
  pitwall-app --lib device::auto::tests::a_real_boot -- --ignored`, with a
  device created in a private set (`xcrun simctl --set <dir> create …`):
  `xcrun simctl --set <dir> boot` run under the test process was found in
  the real process table as an iOS launcher under it, the real `simctl
  list` was the baseline, and the boot (by `lastBootedAt`) opened beside
  "the agent". Shut down and deleted afterwards; no window appeared.
- `xcrun simctl list -j devices` on Xcode 26.1.1 has `lastBootedAt`.

Tested only against fakes:
- Attribution (one agent, two agents told apart by their lines, an
  ambiguous boot only offered, lines alone, lines naming another device,
  old signals), Android boots as devices coming online, each boot once,
  the three settings, `show_device`'s resolution and wait (fake listings
  and a fake sleep), the adb server's socket protocol (a fake wire), the
  tile placed right of the agent without the keys, never twice, the
  notice's Open in Pitwall (GPUI test windows, not drawn on screen), the
  tool and `pitwall open --device` against a running server with a fake
  app.
- The whole Android path: listing (parser on captured `adb devices -l`
  text), the connect sequence and its order, touch/key/text/back/home
  bytes, the dummy byte, a missing or wrong server file (nothing runs),
  an encoder error, a non-H.264 stream, the device going away, and closing
  (server killed and forward removed, once). `adb devices -l` was run for
  real (no devices); no emulator was booted and no scrcpy-server used.

Not checked at all:
- Opening on its own in a running Pitwall with a real agent (Claude Code
  booting a simulator); the notice as drawn; the real outputs of flutter,
  Expo and React Native (their lines are from their sources and docs, not
  captured here); an Android emulator or the adb socket against a real
  server.
- The tile drawn in a window (tests may not open visible windows here):
  `gpui::surface` with these buffers, the letterboxing against
  `geometry::fit`, clicks landing where they should on screen.
- Simulator rotation; a phone over USB; Xcode versions other than 26.1.1.

## Risks
- **Private iOS interfaces** change with Xcode (above). Contained to the
  helper; view-only is the floor, a stopped tile the worst case.
- **scrcpy version lock**: server and protocol move together; a new
  scrcpy means a new pinned hash and possibly framing changes (v4 changed
  the framing).
- **adb side effects**: listing starts the adb server if it isn't running
  (as Android Studio does). Pushing a file to `/data/local/tmp` on the
  user's device.
- **Unverified drawing** (above). It is the next thing to check by hand.
- **Frame pacing**: the UI keeps only the newest picture; a slow UI drops
  frames instead of queuing them. The helper polls the IOSurface at the
  fps and encodes only when its seed changes (plus once a second).

## What I would cut
- The hand-written Indigo touch message and keyboard path if the next
  Xcode breaks them, until DTUHID is done: ship iOS view-only rather than
  half-working input.
- The screenrecord fallback and emulator gRPC: not needed if the owner
  accepts a user-provided scrcpy-server.
- The manual turn (`Turn`): devices send their own rotation.

## Decisions for the owner
1. **iOS input via private frameworks.** Built and working on Xcode 26.1.1.
   Recommend: ship it, with the CoreSimulator version gate above, and
   budget the DTUHID path before the next Xcode major.
2. **How scrcpy-server reaches the user.** (a) The user downloads it
   (today: put the file in `~/.pitwall/device/`, checked by hash);
   (b) Pitwall downloads the pinned release from GitHub on first use,
   checks the hash, with a one-line consent; (c) bundle it (Apache-2.0
   allows it: add its licence and NOTICE lines). Recommend (b): no binary
   in Pitwall's own release, still one click, hash-pinned.
3. **Windows and Linux decoders.** Recommend Media Foundation on Windows
   first (no new dependency or licence), then Linux via the system's
   libavcodec where present, with openh264 as a later option; both draw
   through `RenderImage`.
