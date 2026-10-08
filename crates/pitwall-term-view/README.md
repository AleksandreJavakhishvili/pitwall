# pitwall-term-view

A reusable GPUI terminal view for Pitwall: it draws an `alacritty_terminal`
grid and turns keyboard, IME, mouse, scroll, focus and clipboard input into
bytes. Pitwall's own code is built on it: interactive panes and the light,
read-only Wall tiles.

Clean room: written from the GPUI and alacritty_terminal APIs and docs, and
from xterm's control-sequence documentation. Nothing comes from Zed's
GPL-3.0 `terminal` / `terminal_view` crates, and nothing depends on them.

```sh
cargo run -p pitwall-term-view --example term            # $SHELL on a local PTY
cargo run -p pitwall-term-view --example term -- --light --no-blink -- /bin/bash --norc
cargo run --release -p pitwall-term-view --example wall_bench -- --tiles 20
cargo test -p pitwall-term-view
```

## GPUI source

**crates.io `gpui = "=0.2.2"`** (published 2025-10-22). It is the newest
crates.io release on 2026-10-08, so no Zed git rev is used. It is
Apache-2.0, and so are its `gpui_*` helper crates.

- Feature `runtime_shaders`: the Metal shaders are compiled at startup. This
  lets the crate build without Xcode's separate Metal toolchain (Xcode 26
  doesn't install it). A release build could drop the feature once CI has
  `xcodebuild -downloadComponent MetalToolchain`.
- On macOS gpui pins `core-foundation = "=0.10.0"`. That moves the whole
  workspace, Tauri included, from 0.10.1 to 0.10.0 in `Cargo.lock`
  (`toml` 0.8.2 → 0.8.23 and `proc-macro-crate` 2.0.2 → 2.0.0 move too). The
  workspace tests and clippy pass with it.
- On Linux, gpui needs system libraries that CI doesn't install yet: xkbcommon
  (+x11), wayland, xcb, fontconfig/freetype and vulkan headers. Only macOS was
  built here.

## Use

```rust
let pty = LocalPty::shell(TermSize::new(100, 30))?;            // or HolderStream::connect(socket, true)?
let terminal = Terminal::new(pty, TermSize::new(100, 30), TerminalConfig::default());
let pane = cx.new(|cx| TerminalView::new(terminal.clone(), ViewMode::Interactive, ViewSettings::default(), window, cx));
let tile = cx.new(|cx| TerminalView::new(terminal, ViewMode::Tile { min_scale: 0.3 }, ViewSettings::default(), window, cx));
cx.bind_keys(pitwall_term_view::default_key_bindings());       // or the app's own, in context "Terminal"
```

`TerminalView` emits `TermEvent`s: `Title`, `Bell`, `ClipboardStore` and
`Exited`.

## Design

**Model (`terminal.rs`, no GPUI).** A `Terminal` is one `Term` plus its
`vte` parser, inside alacritty's `FairMutex`. Any number of views can share
it: a pane and a Wall tile show the same terminal through one parser.

- **Bytes come in pushed.** `TermStream::attach(feed)` hands the stream a
  `Feed`, and the stream calls `feed.push(bytes)` from whatever thread it
  reads on. Pushes are parsed in 32 KB slices, and the lock is released
  between slices so a flood of output can't stall a frame.
- **Bytes go out the same way.** Keys, pastes, reports and the parser's own
  answers (DA, DSR, OSC 10/11 colour queries, CSI 14 t) go through
  `TermStream::write`, and resizes through `TermStream::resize`.
- **Adapters.** `LocalPty` (portable-pty, feature `pty`) is used by the
  example, tests and bench. `HolderStream` (feature `holder`) speaks
  pitwall-hold's attach protocol: `attach(replay)`, then `Output`/`Replay`
  frames in and `input`/`resize` frames out. `NullStream` is for views
  that only read.
- **Change signals.** Each view gets a coalesced `Changes` signal: one
  wake-up per batch of output, not one per chunk.
- **Synchronized updates.** DEC mode 2026 is honoured (`StdSyncHandler`). An
  update that never ends is applied at its deadline.

**Rendering (`runs.rs`, `element.rs`, `boxdraw.rs`).**

- **Damage → line generations.** Each frame folds alacritty's damage into
  per-line generations kept in the shared model. Several views can then use
  the same damage, which alacritty itself can't do.
- **Only changed rows are rebuilt.** A row is rebuilt only if its generation
  changed *and* its content hash changed. Shaped rows are cached by content,
  not by position, so scrolling reuses rows that only moved. A test checks
  that one changed cell reshapes exactly one row.
- **Grid → runs.** `runs::build_row` produces background spans, text runs,
  decorations and box cells.
  - Text runs group cells with the same font style and colour. Spaces stay
    inside runs, so runs are long, and each run is shaped once as a whole
    string (`shape_line`).
  - Each run records the column every character starts at. Glyphs are placed
    on their cells by that column, not by the shaper's advances. Glyphs from
    fallback fonts (Georgian, CJK, emoji) are centred in their 1 or 2 cells,
    and combining marks keep their offset from the base glyph. Nothing
    drifts off the grid. Ligatures are off.
- **Colours.** The theme comes first, then OSC 4/10/11/12 overrides, xterm's
  6×6×6 cube and grey ramp, bold-as-bright for colours 0–7, dim at 50 %
  alpha, inverse, and hidden. `TermTheme::pitwall_dark()` and
  `pitwall_light()` are the exact values from `src/terminal/registry.ts`.
- **Box drawing and blocks** (U+2500–257F except the diagonals, and
  U+2580–259F) are drawn as quads snapped to device pixels.
  - That covers light, heavy and double lines with proper joins, dashes,
    shades, quadrants and eighths.
  - Rounded corners are bordered quads with a corner radius, clipped to the
    cell.
  - Lines meet across cells and rows at any line height, so there are no
    gaps.
- **One paint layer per terminal.** This is the key performance fix.
  - Outside a layer, GPUI inserts every primitive into a bounds tree to
    compute its draw order. With about 50k glyphs a frame that alone cost
    25 ms.
  - Inside `paint_layer`, every primitive shares the layer's order, which
    brings it down to 0.6 ms.
  - Within a layer, quads draw before glyphs, which is exactly the order a
    terminal needs: backgrounds, then selection and search, then the block
    cursor, then text (the text under the cursor uses the accent colour).
- **Metrics follow xterm.js exactly**, so a pane looks like today's:
  - Cell height uses xterm's rule:
    `floor(ceil(round(ascent + descent) × dpr) × line_height) / dpr`. At
    13 px with line height 1.15 that is 19.5 px, as in the web UI.
  - Cell width is the font's advance width.
  - The baseline is centred in the row, like CSS half-leading.
  - The default padding is Pitwall's pane padding (6/4/4/10).
- **Fonts.** The family list is JetBrains Mono, then Menlo, SF Mono, and so
  on. Each candidate is checked with `resolve_font` and the choice is cached
  for the process. Enumerating all system fonts instead made CoreText load
  every font, about 6 MB per view.
  - The app should register **static** JetBrains Mono TTFs (Regular, Bold,
    Italic, Bold Italic; OFL-1.1) with `register_fonts` before the first
    frame. The web UI's fontsource package only has variable woff2 subsets.
    CoreText can't load woff2, and with an in-memory variable font, bold
    renders as regular.
- **Wall tiles** (`ViewMode::Tile`) show the terminal at its own size, scaled
  to the tile's width, with the bottom rows shown first. They never resize
  the PTY and never take input.
  - All tiles repaint together from one `TileClock`: at most 10 times a
    second, and only the tiles that changed. Per-tile timers had produced
    about 190 window frames a second.
  - Wrap tiles in `AnyView::cached(...)` so tiles that didn't change replay
    their last frame.

**Input (`keys.rs`, `mouse.rs`, `view.rs`).**

- **Keys.** xterm encodings for cursor keys (DECCKM), Home/End,
  PageUp/Down, Insert/Delete and F1–F20 with modifier parameters; Tab and
  Shift-Tab; Backspace and Ctrl/Alt-Backspace; C0 control bytes;
  Ctrl+Alt; and Option-as-Meta (on by default, like Pitwall's xterm).
  - Plain text, IME commits and Option-composed characters are not encoded
    here. They go through GPUI's input handler, so dead keys and input
    methods work.
  - Held keys repeat; the macOS press-and-hold accent menu is off.
  - ⌘ chords are left to the app's key bindings.
- **IME.** Composition text is drawn underlined at the cursor. Nothing is
  sent until the input method commits. The candidate window is placed at the
  cursor cell.
- **Paste.** Line endings become CR. When the program enables mode 2004 the
  paste is wrapped in brackets, and any end marker inside it is removed.
- **Mouse.**
  - Reporting: modes 1000, 1002 and 1003, encoded as default, UTF-8 (1005)
    or SGR (1006). Wheel events are reported too. Holding Shift bypasses
    reporting.
  - Selection: Simple, word (double click), line (triple click) and
    Shift-extend. Dragging past the top or bottom edge scrolls.
  - Links: ⌘-click on a URL (regex on the logical line) or an OSC 8
    hyperlink; the link is underlined while hovered.
- **Scrolling.** The wheel and trackpad scroll the scrollback, with pixel
  deltas accumulated. On the alternate screen with mode 1007 the wheel sends
  arrow keys instead.
  - The scrollbar thumb shows on hover or while scrolled back, and can be
    dragged. Its gutter is always reserved: when it depended on hover, the
    column count changed on hover and cleared the selection.
- **Focus.** Mode 1004 reports focus in and out, for both view focus and
  window activation. An unfocused view shows a hollow cursor. The cursor
  blinks (600 ms) and stops after 5 minutes without input.
- **Search** (⌘F). A literal find bar uses alacritty's regex search.
  Enter/⌘G finds the next match upwards, Shift-Enter/⌘⇧G downwards. Visible
  matches and the current one are highlighted, and the view scrolls to the
  match.
- **Other actions.** Copy, Paste, SelectAll, Clear (drops scrollback and
  sends ^L), scrolling by line, page and to the ends, and font size ⌘=/⌘-/⌘0.
- **Program requests.** OSC 52 copy is allowed and read is denied
  (`Osc52::OnlyCopy`). OSC 0/2 title and BEL become events.

## Side by side with the current terminal

I rendered the same bytes two ways: GPUI (`--font-file` with static
JetBrains Mono) and xterm.js 6 configured exactly like `registry.ts`
(options, theme, `macOptionIsMeta`, unicode11, pane padding, antialiased
smoothing). The xterm.js side ran in headless Chrome, not the app's
WKWebView, so the installed app wasn't touched.

What matches:

- The glyphs, weights (bold, dim, italic), row pitch, cell width, padding
  and block cursor.
- Underline, double underline and strikethrough positions.
- The colours: identical once colour-managed. GPUI's screenshots are
  tagged Display P3 and Chrome's are sRGB; converted to sRGB, every theme
  colour reads exactly, for example `#ff6b6b` and `#5fd38d`.

What differs:

- **Box drawing:** gap-free in GPUI. xterm's DOM renderer draws it with the
  font, so its lines break between rows at line height 1.15.
- **Shades ░▒▓:** flat alpha in GPUI, dithered font glyphs in xterm.
- **Emoji:** slightly larger in GPUI.
- **Curly underline:** smaller amplitude in GPUI.

## Benchmark: 20 Wall tiles

`examples/wall_bench.rs` runs N tiles. Each one is a local PTY running the
repo's synthetic busy agent `scripts/tui-agent.py` (about 10 redraws a
second of Claude-Code-style output, roughly 12 KB/s, print only). The bench
measures its own process only: the generators are excluded, as agents are
in perf.md.

- **Memory** is `phys_footprint`.
- **CPU** is user+sys time as a percentage of one core.
- **Frame CPU** is render, layout, prepaint and paint for one window frame.

Measured on an M-series Mac with a release build (LTO): 1600×1000 window at
2× scale, on screen, 100×30 terminals, 8 s settle then 10 s measured, three
runs.

| 20 busy tiles | GPUI tiles (this crate) | Current app, 20 tui agents + Wall (perf.md pass 2) |
|---|---|---|
| CPU | **2.9–3.2 %** (whole process: 20 parsers + UI) | tree 46–49 % (WebKit 35–36 %, app 17.6 %) |
| Memory | **193–214 MB** avg, GPU memory included (one transient peak of 383 MB) | WebContent 277–299 + GPU process 226–231 + app ~60 ≈ 560–590 MB |
| Frames | 9.4/s, frame CPU p50 1.5–1.6 ms, p95 2.4–2.5 ms, max ≤ 3.2 ms | not measured |
| Rows reshaped | ~275/s for 20 tiles (out of ~5 700 visible rows/s) | n/a |

Other runs:

| Run | CPU | Memory |
|---|---|---|
| 20 idle tiles | 0.7 % | 87 MB |
| GPUI with no tiles | 0.5 % | 80 MB |
| 20 busy tiles, view cache off | 4.9 % | 384 MB (more GPU memory) |

Fixes the bench drove:

| Fix | Before | After |
|---|---|---|
| Paint layer | 98 % CPU, 25 ms per frame | 0.6 ms per frame |
| Shared tile clock | ~190 frames/s | 9.4 frames/s |
| No font enumeration | 216 MB with 20 idle tiles | 87 MB |

These numbers are not like for like:

- The bench has no Pitwall core: no holder IPC, detection or git polling.
  Most of the current app process's 17.6 % was Tauri IPC plumbing, which an
  in-process GPUI app doesn't have.
- GPUI's GPU memory counts inside the process footprint, and it varies from
  run to run by up to about 150 MB. The WebKit GPU process is counted
  separately in the old numbers.
- "No blank tiles / smooth" was checked by eye on screenshots, not measured.

## Licences

| Dependency | Licence | Use |
|---|---|---|
| gpui 0.2.2 (+ gpui-macros, gpui_collections, gpui_util, gpui_sum_tree, gpui_refineable, gpui_media, gpui_http_client, gpui_semantic_version, gpui_util_macros) | Apache-2.0 | UI framework |
| alacritty_terminal 0.26 (+ vte 0.15) | Apache-2.0 (vte: Apache-2.0 OR MIT) | emulator (already in the workspace) |
| futures 0.3 | MIT OR Apache-2.0 | change signals |
| portable-pty 0.9 (optional, `pty`) | MIT | local PTY (already in the workspace) |
| pitwall-hold (optional, `holder`) | Apache-2.0 (this repo) | holder adapter |
| libc 0.2 (dev) | MIT OR Apache-2.0 | bench measurements |

The whole tree of this crate (516 crates on aarch64-apple-darwin) is
permissive: MIT and/or Apache-2.0, BSD-2/3, ISC, Zlib, Unicode-3.0,
CC0-1.0, Unlicense and BSL-1.0 options. It also has two MPL-2.0 crates:
`cbindgen`, a build-time code generator used by gpui's build script, and one
that was already in the workspace. No GPL, LGPL or AGPL. No Zed `terminal`
or `terminal_view`.

## Postponed (in-house work, not dropped)

- **Kitty keyboard protocol.** alacritty supports it; it is off here, as in
  xterm.js today. Turn on `Config::kitty_keyboard` and encode the keys.
- **Application keypad mode.** GPUI doesn't report keypad keys separately.
- **Meta+Shift on non-US layouts** uses a US shift table (Option-as-Meta
  only).
- **SGR 21.** xterm.js treats it as double underline; alacritty's parser
  treats it as "bold off". Double underline works through `4:2`.
- **Vi / keyboard selection mode** (alacritty has one), and
  keyboard-extended selection.
- **Search bar extras.** It is a minimal literal find: no regex or case
  toggles, no match count, no text cursor inside the query.
- **Selection auto-scroll** only moves on mouse movement; there's no timer
  while the pointer rests past the edge.
- **Links.**
  - The URL regex doesn't keep balanced parentheses.
  - There's no URI tooltip on hover.
  - Plain-click opening (without ⌘) is not available.
- **Bell.** Only an event; no visual bell.
- **Diagonals** ╱╲╳, Powerline (U+E0B0…) and legacy computing symbols are
  drawn by the font, not as shapes.
- **Bidirectional text** (Arabic, Hebrew) isn't reordered, as in xterm.js.
- **Images.** No sixel, kitty or iTerm image protocols.
- **Bundled font.** JetBrains Mono isn't bundled in the repo. The app
  should ship the static TTFs and call `register_fonts`; until then Menlo is
  used. Bold synthesis for fonts without a bold face isn't done either.
- **Emoji and curly underline** sizing: tune to match xterm (see above).
- **Accessibility.** GPUI 0.2.2 has no accessibility tree.
- **Linux primary selection** (middle-click paste) and copy-on-select by
  default (`ViewSettings::copy_on_select` exists, off by default).
- **Wall tile details.**
  - There's no per-tile hover or scroll.
  - Below `min_scale` a tile crops its right edge.
  - Pitwall's per-tile auto-shrink font isn't wired in; the view has font
    size actions.
- **Other platforms.** Only macOS was built and run here. Linux and Windows
  aren't built or tested yet, and Linux CI needs gpui's system libraries.
- **Release shaders.** Precompiled Metal shaders (drop `runtime_shaders`)
  need the Metal toolchain on the build machine.
