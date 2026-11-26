# Tile SDK: native building blocks

Part of the [Tile SDK and marketplace](README.md) design. No native plugin API; native building blocks; which built-in tiles move to the SDK. Sources (S1…) and *unverified* claims are listed in [sources.md](sources.md).

## 16. Native building blocks and native capabilities

Extensions get native speed and native looks through parts Pitwall
draws and runs, not through native code of their own. This section
says what those parts are, how an extension places and controls them,
and which of Pitwall's own tiles move onto the SDK.

### 16.1 The principle: no native plugin API
Pitwall never loads an extension's dynamic library (`.dylib`, `.so`,
`.dll`), in the app or in the daemon. Reasons:
- **A crash takes everything down.** Code loaded into the app shares
  its process: a bad pointer or a panic across the boundary ends every
  terminal, every agent's screen and every unsaved edit at once. A
  trapped component ends only its own tile ([2.2](runtime.md#22-crates-new)).
- **No permission boundary.** Native code can read any file, the
  keychain, other tiles' memory and the agents' screens. The grants in
  [section 5](permissions.md#5-permissions-and-security-model) would be advice, not limits.
- **No stable Rust ABI.** Rust doesn't promise a layout for types
  across compiler versions, so a plugin built against one Pitwall
  release can't be trusted with the next; every release would break
  every plugin or force a C ABI that loses Rust's types.
- **Per-platform builds and signing.** Each plugin would need a build
  per OS and CPU, and on macOS each must be signed in a way the app's
  hardened runtime accepts (the details are *unverified* here, as in
  [section 12](alternatives.md#12-alternatives-considered)). One `.wasm` file runs everywhere ([8.1](runtime.md#81-three-versions)).
- **GPUI changes often.** Pitwall vendors GPUI at a fixed Zed commit
  and moves it on its own schedule; a plugin drawing with GPUI directly
  would break on every move. Tiles describe a tree the host draws
  ([3.3](ui.md#33-the-ui-description)), so the host can change GPUI freely.

Zed made the same choice for its own reasons: its team wrote that
"we would never be able to prevent these kinds of crashes if we loaded
extensions directly as shared libraries", and chose WebAssembly
because "it's cross-platform, and it's designed for running untrusted
code safely" (S25, about tree-sitter grammars; S18 for extensions in
general). Zed's posts don't list the other reasons above; those are
this document's.

So: **anything that needs native speed or OS access is built by
Pitwall and offered through the SDK**, as a building block ([16.2](#162-native-building-blocks)) or a
platform service ([section 14](platform-services.md#14-platform-services-what-pitwall-gives-every-tile)). A native program an author must run
(a database driver, a language server) runs only as a managed service
in its own process ([section 13](services.md#13-tile-apps-with-a-server)), never inside Pitwall.

### 16.2 Native building blocks
A building block is a part Pitwall draws natively with its own code
(the same code its built-in tiles use), which an extension places in
its tile and controls. It differs from the plain elements of [3.3](ui.md#33-the-ui-description) in
two ways: it has state the host keeps (a buffer, a page, a stream), and
the extension talks to it through a handle, not by re-sending its
content every frame.

How it fits the tree: the extension creates a block through a host
interface and gets a resource; it places the block with an element
that names the resource (`block(handle-id)`); the host draws it there,
keeps its state across frames and sends its events to the tile's
`on-event` ([3.4](ui.md#34-events)) with the handle's id. Dropping the resource removes
the block. A block not placed in the current frame keeps its state
until dropped or the tile is suspended.

The `code` element of [3.3](ui.md#33-the-ui-description) stays for short read-only snippets (an
error, a request body). `code-view` is for files and anything the user
edits.

#### Code view
Pitwall's code view (`code_view` [today]): highlighting by language,
find, brackets, cursors, undo, the editor's limits.

```wit
interface code-view {
  resource code-view {
    constructor(language: string, read-only: bool);
    set-text: func(text: string);              // replaces; resets undo
    text: func() -> string;                    // editable views only
    revision: func() -> u64;                   // bumps on every edit
    set-language: func(language: string);
    set-read-only: func(read-only: bool);
    reveal: func(line: u32, column: u32);
    set-marks: func(marks: list<line-mark>);   // gutter: added, changed, error, note
    set-decorations: func(d: list<range-mark>); // underline, highlight; host colours
    selection: func() -> list<range>;
  }
}
// events (into 3.4's variant): code-changed(handle, revision),
// code-save(handle) on ⌘S, code-selection(handle, list<range>)
```

- **Permission:** none for text the extension supplies. Opening a
  project file into it goes through `files.read`, saving through
  `files.write` (the extension calls `files.write` itself after
  `code-save`; the view never writes a file).
- **Read back:** the text of its own editable views and the selection,
  nothing else. It can't read other tiles' editors or the user's open
  files.
- **Limits:** the editor's [today]: edits up to 2 MiB, read-only from 2
  to 10 MiB, highlighting up to 4 MiB, larger files refused with a
  message; no soft wrap yet ([editor.md](../editor.md)).
- **Minimum or on demand:** minimum (phase 2), because the review port
  ([16.3](#163-which-built-in-tiles-move-to-the-sdk)) needs it read-only and the files tree port needs it editable.

#### Diff view
Pitwall's diff drawing (`code_view::diff`, the Review screen [today]):
side by side or inline, line and character changes, folded unchanged
runs.

```wit
interface diff-view {
  resource diff-view {
    constructor(language: string, layout: diff-layout); // side-by-side | inline
    set-texts: func(old: string, new: string);
    set-layout: func(layout: diff-layout);
    reveal: func(side: side, line: u32);
    set-comments: func(c: list<line-comment>);  // drawn by the host under a line
  }
}
// events: diff-line-click(handle, side, line), diff-comment(handle, side, line)
```

- **Permission:** none for texts the extension supplies. The texts of
  a project's changes come from `git.read` (`git.diff` gives old and
  new per file).
- **Read back:** nothing: the extension supplied both texts already.
- **Limits:** the character-level diff stops above 40 000 characters
  per change and falls back to lines (`MAX_CHAR_DIFF` [today]); the
  code view's size limits apply to each side.
- **Minimum or on demand:** minimum (phase 2): the review port needs
  it, and the git client seed tile later.

#### Web view
A page inside the extension's tile, in the same native web view the
web tiles use ([web.md](../web.md) [today]), under the same rules.

```wit
interface web-view {
  resource web-view {
    constructor(url: string);                  // http(s) only
    navigate: func(url: string);
    reload: func();
  }
}
// events: web-loaded(handle, url-origin)   // origin only, never the page
```

- **Permission:** `web.view` with a hosts list, like `net`'s ([5.1](permissions.md#51-new-permissions-new)):
  the extension can only put pages from those hosts in its tile. The
  host draws the page's origin in a strip above it, so a tile can't
  pass off a login page as its own UI.
- **Read back:** none: no script bridge, no page text, no cookies, no
  screenshots; links the user follows stay in the view. Same reasons as
  the web tile's limits ([section 11](agent-tools.md#11-tools-for-agents) and [1](README.md#1-goals-non-goals-user-stories)).
- **Minimum or on demand:** on demand. No seed tile needs it; a tile
  that wants a whole page can already open a web tile (`web.open`,
  [14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)). Trigger: an extension that must show a provider's page next to
  its own controls (a dashboard, a preview).

#### Terminal view: later
**Decision: not in v0.x; on demand later, if ever.**
- A terminal shows agents' screens and scrollback, which carry secrets
  and are withheld from extensions at every grant ([14.4](platform-services.md#144-not-provided)). Even a
  show-only view lets a tile choose when and where an agent's screen
  is visible, and invites requests for "just the last lines".
- A view of a shell the extension starts would be a native program run
  by an extension, which only a managed service may do ([section 13](services.md#13-tile-apps-with-a-server)).
- Terminals have one owner view and own the keyboard while focused
  ([terminals.md](../terminals.md) [today]); sharing that with a tile is
  new work in the terminal code.
- No seed tile and no port needs it. What extensions get instead:
  `agents.focus` brings the agent's own terminal forward, and a tile
  can link to it.

If a real need appears, the smallest form is a show-only view of one
agent's terminal, granted per agent by the user, with no read-back;
that would be its own design.

#### Device screen
The device tile's stream (Android through scrcpy, the iOS Simulator
through `pitwall-simhost` [today]), placed in an extension's tile.

```wit
interface device-view {
  resource device-view {
    constructor(device-id: string);            // from devices.list
  }
}
// events: device-state(handle, connected | gone)
```

- **Permission:** `devices.view` (new): list devices and show one.
- **Read back:** none: no frames, no screenshots, no input events. The
  user drives the device in the view as in a device tile; the extension
  sees only that it is connected.
- **Minimum or on demand:** on demand. Trigger: an extension that pairs
  a device with its own controls (a test runner, a deep-link tool).

Every block follows the rules of [3.3](ui.md#33-the-ui-description): the host picks fonts and
colours, menus and dialogs stay inside the tile, the extension can't
draw over a block, and a block's own keys (⌘S, ⌘F, typing) are the
host's while it is focused.

### 16.3 Which built-in tiles move to the SDK
Built-in tiles that are mostly UI over data an extension could reach
are rebuilt on the SDK as first-party extensions, bundled with Pitwall
and updated like any other. Tiles that are the platform itself stay
native.

| Tile today | Decision | Why |
|---|---|---|
| Review and source control (`review/`, [review.md](../review.md)) | **Port**: the first-party proof tile | Large and used daily, so it finds gaps fastest. It is UI over git, agents and files, which the SDK offers. Its native parts become building blocks (code view, diff view) |
| Files tree (`explorer/`, files tiles) | **Port** | A tree, a search box, viewers: all SDK elements or blocks. Needs `files.read/write`, file watching and a native search service (below) |
| Inbox (`inbox_tile.rs`) | **Port** the tile; the inbox model and the floating control stay Pitwall's | The tile is a list and a thread over the `inbox` interface ([15.4](extension-model.md#154-from-adapter-to-component)). [15.6](extension-model.md#156-what-stays-different) keeps the inbox as a dedicated surface; that holds: the model, the number keys and the floating control stay native, and the ported tile is one view of them |
| Terminals | **Stays native** | The platform's core; contents withheld from extensions ([16.2](#162-native-building-blocks)) |
| Editor (`editor/`) | **Stays native** | It *is* the code view block; one editor, shared |
| Web tiles | **Stays native** | A native web view with no bridge; the web view block reuses it |
| Device tiles | **Stays native** | Native streaming and GPU surfaces; the device view block reuses it |
| Settings | **Stays native** | Holds the permission grants and kill switches for extensions; an extension must not be able to draw it |

**The review port as proof tile.** It sits next to the seed tiles
([10.1](README.md#101-seed-tiles-first-party-public-sdk-only)), built only on the public SDK, in the order: read-only lists and
diffs first, then comments to Next up, then commit and merge. What it
needs, and what that pulls forward:
- `agents.read`, `worktrees.read` (minimum, [today]).
- `git.read` and `git.write`, and `files.read` for Markdown previews:
  these move from "on demand" ([14.3](platform-services.md#143-later-on-demand)) into the plan when the port
  starts, instead of waiting for the git client seed tile.
- Code view and diff view ([16.2](#162-native-building-blocks)), so both are minimum.
- Gaps it is expected to show, each to be decided when met:
  - `git`'s fixed operations lack `merge`, `merge-base` and restoring
    one file ([3.8](runtime.md#38-host-apis)); Review uses all three [today].
  - Per-task diffs come from task boundaries the daemon records
    (`review.md`, "Per-task diffs" [today]); extensions have no read
    for them.
  - "Send comments" sends at once to an idle agent [today]; a tile's
    `queue.add` only adds to Next up, waiting for Send ([14.2](platform-services.md#142-the-v0x-minimum-built-in-phases-12)). Either
    the port accepts that step, or a "send now" grant is designed.
  - Review is a full-window mode (⌘R) today; a tile has no such mode.

**The files tree port** needs, beyond `files.read/write`: change
events for granted projects (a watch the host runs), and search as a
host service (`files.search`, run natively by Pitwall over the granted
roots, results only), since a guest walking a large tree through WASI
would be slow (*unverified* until measured). Image and PDF viewers
become blocks or stay as "open in a viewer" (`viewer.open`); decide
when the port starts.

**Order and risk.** Ports start after the matching blocks and services
exist (phase 2 onwards) and replace the native tile only when the port
matches it feature for feature; until then the native tile stays the
default. If a port can't match it, the native tile stays and the gap is
written down.

### 16.4 Decisions
1. **Terminal view in v0.x?** Decided (owner, 2026-11-26): no ([16.2](#162-native-building-blocks)); on demand later,
   show-only, per agent, if ever.
2. **Web view: minimum or on demand?** Decided (owner, 2026-11-26): on demand; `web.open`
   covers today's needs.
3. **Port the review screen before WIT 1.0?** Decided (owner, 2026-11-26): yes: start it
   in phase 2 as the proof tile, so its gaps reach WIT before 1.0
   freezes it.
4. **Review as a tile or keep a full-window mode?** Decided (owner, 2026-11-26): a tile
   first, maximised by ⌘R (layouts can already maximise a tile); a
   full-window mode for extensions only if the port shows it is needed.
5. **Send comments at once?** Decided (owner, 2026-11-26): keep Next up's Send step
   for the port at first (the same safety as every extension), and
   revisit if users find the extra step slows review.
6. **Drop the native tiles once ported?** Decided (owner, 2026-11-26): yes, one release
   after each port matches its native tile, so there is one code path.
