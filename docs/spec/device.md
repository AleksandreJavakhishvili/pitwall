# Device tiles

A phone's screen in a tile of a space's layout, next to the agent's
terminal: an Android emulator or a phone on a cable, or an iOS Simulator.
You watch the app the agent is building and tap and type into it, without
an emulator or Simulator window. Agents never see or drive the screen.

Status: first slice built (GPUI, macOS). The iOS Simulator path runs end to
end on this machine; the Android path is built and tested only against a
fake adb (no device and no scrcpy-server were available, see "How it was
checked"). `show_device` and the terminal's "Preview device" offer are
designed here but not built.

Setting: `general.devicePreviews` (on by default). Off: no device tile
opens, none is offered, nothing is listed.

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

## Agents: `show_device` (designed, not built)
Beside `show_web` and `show_settings` in Pitwall's `pitwall` MCP server:
`show_device { device?: "android:<serial>" | "ios:<udid>" }`. It opens
that device's tile next to the agent's terminal (or the only booted device
when none is named). It only shows: the agent gets back "shown" or why not,
never pixels, and has no input tool. A setting `agents.showDevice` (needs
`general.devicePreviews`) removes it. Not built in this slice.

## "Preview device" offer (designed, not built)
Like the terminal header's "Preview :port": when a terminal prints a line
that native tools print on start (`adb` "Installing APK", `flutter run`
"Launching … on <device>", Expo "Opening on iPhone…", `xcrun simctl
launch`), the header offers "Preview <device>" for that device. Matched on
the tools' own output, not on a mode we invent. Not built.

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

Tested only against fakes:
- The whole Android path: listing (parser on captured `adb devices -l`
  text), the connect sequence and its order, touch/key/text/back/home
  bytes, the dummy byte, a missing or wrong server file (nothing runs),
  an encoder error, a non-H.264 stream, the device going away, and closing
  (server killed and forward removed, once). `adb devices -l` was run for
  real (no devices); no emulator was booted and no scrcpy-server used.

Not checked at all:
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
