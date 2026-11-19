# Web tiles

A web page in a tile of a space's layout, next to the terminals and files
tiles: docs, a dashboard, a dev server, without leaving Pitwall. It is the
system's own web view, not a browser: one page per tile, no tabs, no search,
no extensions, and agents have no access to it.

Status: built (GPUI). Step 1 of "web in Pitwall".

## Two kinds, one tile
- **A page you open.** "Open web page…" in the command palette, or ⌘L
  (Ctrl+Shift+L on Windows/Linux): a web tile right of the focused tile (in
  it, when that tile is empty), its address field taking the keys. Owned by
  no agent.
- **An agent's Preview.** "Preview :<port>" in a terminal's header
  (terminals.md §4) opens the dev server the terminal printed in a web
  tile right of that terminal. One per agent: pressing Preview again closes
  it, and an address from the chip's menu goes to the open one. It follows
  its terminal (see below) and closes with its agent.

Both are the same tile kind (`ui.json` `webTiles`, a layout pane's `web`):
the same bar, dragging, drop zones, resizing and restore.

## The bar
32 px, the tile's drag handle: a grip, back, forward, reload (stop while
loading), the address field, the page's title (a Preview: "<agent>'s
preview"), for a Preview phone width (390 px, centred) and Web Inspector,
then "Open in default browser" and close. No favicon (an icon stands for
it). A dot in the field while the page loads. A line under the bar says
why something didn't happen (an address that isn't one, a declined
download).

## Addresses (`webview::address`)
- `http(s)://…` as typed.
- A host without a scheme gets one: `example.com/docs` → `https://`;
  local and LAN hosts (`localhost:3000`, `127.0.0.1`, `192.168.x.x`,
  `*.local`, `*.internal`, tailnet) → `http://`.
- Anything else is refused under the bar, nothing loads: words with spaces,
  a word without a dot ("there's no search here"), and other schemes
  (`file:`, `javascript:`, `about:`, custom app schemes).
- A Preview's address is editable too. Typed there it is followed as the
  Preview's address until the terminal prints a new one, which wins.
  A Preview stays on local addresses (`webview::allowed`); anything else
  opens in the browser.

## Layout
- Dragged by its bar (the grip or the bar's free space) like a files tile:
  beside any tile, along an edge of the space, onto a terminal to take its
  place (that terminal keeps running and stays in the sidebar), onto
  another files or web tile to swap. The drop zones say "Web page".
- Resized with the layout's dividers; presets keep it.
- Restored after a restart with its address (and a Preview with its agent;
  a Preview whose agent is gone is pruned). A tile moved to a space in
  another window gets a new view there (the page loads again).
- Old `ui.json` with the in-pane Preview's `previewBelow` / `previewLeft`
  loads; they are dropped.

## Layering
The web view is a native view over GPUI's drawing (`src/webview`), placed
after every frame where its element was drawn (`preview::placed`).
- Not drawn (another space, the Wall, Review, a folded tile): hidden.
- Covered by anything drawn later (menu, dialog, palette, toast, tooltip)
  or while anything is dragged over the window (a tile, an agent, files):
  swapped for a snapshot of itself drawn in its place, so drop zones and
  ghosts show (`preview::covering`, `Stage`). Liquid Glass pop-ups are
  windows of their own above the main window, so they cover it natively.

## Keyboard
A click into the page gives it the keys. Pitwall's shortcuts (⌘ on macOS,
Ctrl+Shift elsewhere) still work from there; the page keeps copy, paste,
cut, select all, undo, redo. ⌘L in a web tile selects its address. Esc
(after the page had it too), a click anywhere outside the page (the bar,
its address field, another tile), or GPUI's focus moving on gives the keys
back. The address field is a GPUI field in the main window: it types the
same in Flat and Liquid Glass.

## Lifetime and cost
- A page you opened stays loaded while its tile exists, hidden or not
  (another space, route or window state): no reload when you come back. A
  hidden page is the engine's to throttle (WebKit pauses timers and
  animation of hidden views).
- A Preview hidden for 10 minutes lets its view go and keeps its address;
  shown again, it loads again.
- Closing a tile frees its view.

## Data
- Pages you open: one persistent profile, `default`, in Pitwall's data
  folder (`<data>/webview/profile-default`, `preview::profile_dir`; a
  profile id is the folder's name so more can come later). macOS 14+:
  WebKit's own store keyed by that folder (`dataStoreForIdentifier`, the
  data under `~/Library/WebKit/<app>/WebsiteDataStore/<id>`): the app's,
  never Safari's. Before macOS 14: the app's default WebKit store (shared
  with Previews). Windows (WebView2) and Linux (WebKitGTK): in the folder.
- Previews: a store per project, as before (`<data>/webview/<project>-<hash>`).

## Untrusted content
- No script bridge: Pitwall injects no script and has no message handler.
- Only http(s) loads in a page tile; another scheme a page navigates to
  isn't opened (nor handed to the system: no `msteams:`, `file:`).
- `target=_blank` and `window.open`: the address loads in the same tile
  (a Preview: local addresses only, others open in the browser). No new
  windows.
- Downloads: macOS declines them (a response the engine can't show) and
  says so under the bar, with the browser one click away. Windows and
  Linux: the engine's default (WebView2 saves to Downloads with its own
  download bar; WebKitGTK to the Downloads folder).
- Camera, microphone: denied without a prompt on macOS (WKUIDelegate) and
  Windows (every `PermissionRequested`, also location and notifications).
  Location and notifications on macOS: WKWebView has no prompt of its own
  without app entitlements and Info.plist keys, which Pitwall doesn't
  have, so they fail. Linux: WebKitGTK's default (permission requests
  unhandled are denied).
- Agents: no MCP tool, no daemon method and no CLI command reads or drives
  a web tile.

## Setting
`general.webPages` (on by default; Settings → General → "Show web pages in
Pitwall"). Off: no web view is made; ⌘L says so, an open tile says so in
its place, and a Preview opens in the browser.

## Platforms
WKWebView on macOS, WebView2 on Windows, WebKitGTK (X11) on Linux, the
engines the Preview already used. Where none can be made (no WebView2
runtime, no WebKitGTK, Wayland), a page tile says why in its place and a
Preview opens the browser with a toast, once.

## Not yet
Opening a tile in a window of its own, tabs, favicons, search, find in
page, zoom, more profiles, and anything for agents.
