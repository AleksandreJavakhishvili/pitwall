# Web tiles

A web page in a tile of a space's layout, next to the terminals and files
tiles: docs, a dashboard, a dev server, without leaving Pitwall. It is the
system's own web view, not a browser: one page per tile, no tabs, no search,
no extensions. Agents can open a page for the user (`show_web`, below) but
never read or drive one.

Status: built (GPUI). Step 1 of "web in Pitwall"; step 2 adds pages in
windows of their own and agents' `show_web`.

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
- **A page an agent opened** (`show_web`, below): a page like one you
  opened (the same profile, no Preview rules), remembered as that agent's
  (`webTiles[id].by`). It stays when the agent goes.

Both are the same tile kind (`ui.json` `webTiles`, a layout pane's `web`):
the same bar, dragging, drop zones, resizing and restore.

## The bar
32 px, the tile's drag handle: a grip, back, forward, reload (stop while
loading), the address field, the page's title (a Preview: "<agent>'s
preview"), for a Preview phone width (390 px, centred) and Web Inspector,
then "Open in default browser", "Move…" (the places menu, as a files
tile's), "Open in a window of its own", and close. In a window of its own:
"Dock…" (the same menu) and the pin instead. No favicon (an icon stands for
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
  a Preview whose agent is gone is pruned).
- The page is one entity kept by tile id across the app
  (`crate::web::pages`), whoever draws it. A tile moved to a space in
  another window, or into a window of its own and back, is the same page:
  on macOS its WKWebView is moved into the new window's content view
  (`WebView::reparent`), keeping the page, its history and scroll. Windows
  and Linux make the view again there (the page loads again).

## In a window
- "Open in a window of its own" on the bar, "Window of its own" in Move…,
  `where: window` from `show_web`, or a tile dragged out of every Pitwall
  window (macOS). A normal window titled "<page title> — Web" (its host
  until the page has a title), with Pitwall's chrome and the same bar.
- Back into a layout: "Dock…" (beside a terminal, an edge, a tile of its
  own, in a terminal's place), or its grip dragged over a Pitwall window
  (macOS; the drop zones show there, as for a files window). The window
  closes and the page is in the tile.
- Pin keeps it above other apps (macOS, Windows), as a files window's.
- Its place and size are kept (`ui.json` `webWindow`) for the next one.
- Kept across restarts: unlike a files window (whose place is kept but
  which isn't reopened), a page left in a window (`webTiles[id].window`)
  opens in one again at the next start, behind the rest, without the
  keyboard. Closing the window closes the page; quitting doesn't.
- The placement after each frame (`Stage`: hidden, covered, snapshot) runs
  in the window as in a tile. A Preview can be in a window too, and still
  follows its terminal.

## Agents (`show_web`)
Pitwall's MCP server (the same one as `show_files`, docs/reference/
agents.md) has a third tool, `show_web`; `pitwall open --web [<url>]`
does the same from a shell (Aider's rule names it). Every kind that gets
the files tools gets it the same way; Claude Code and Qwen Code have it
pre-approved (`mcp__pitwall__show_web`).

- `url` (optional): http/https only, read as the address field reads it.
  Without it an empty page opens with its address field for the user to
  type in.
- `where`: right (default), left, above, below (of the agent's terminal,
  or of `of`'s), replace (that terminal's place; it keeps running), window.
  The tile lands in the Pitwall window showing that terminal; when none
  does, a window of its own, and the answer says so.
- One page per agent: its next call goes to the page it opened before,
  wherever the user moved it (navigated there when a url is given), and
  moves it only when `where` names a place it isn't in.
- Nothing comes forward and the keyboard stays where it is, also for an
  empty page (whose field waits for a click). The answer says what opened
  and where, and that the agent can't read or control the page.

The tool's description, as agents see it:

> Show a web page to the user in Pitwall, the app you are running in: it opens in a web tile next to your terminal in Pitwall's layout (or a window of its own), in Pitwall's own web view, where the user may be signed in to sites. This only shows a page to the user: you get no page content back, and you cannot click, type, scroll or run anything in it (nothing the user does there is sent to you). Use it when the user asks to open a page, a site, a local dev server or a browser in Pitwall ("open the docs on the left", "open a browser window"), not to read the web yourself. Without url, an empty page opens with its address field for the user to type in. A later call goes to the page you opened before, wherever the user moved it, unless `where` names a place. The result says what opened and where, or why not (not a web address, Pitwall not running, turned off).

## Safety (agents)
- **Show only.** `web.open` answers with where the page is, never with
  anything from it: no content, title, address it went on to, cookie or
  screenshot. No tool, daemon method or CLI command reads a page, runs
  script in it, clicks, types or scrolls. Nothing the user does in the
  page reaches the agent.
- **Addresses.** Only http and https (`pitwall_proto::web::address`, the
  same rules as the address field), checked in the MCP server, the daemon
  and the app: `javascript:`, `data:`, `file:`, `about:` and app schemes
  are refused and nothing opens.
- **The user's profile.** A page an agent opens is in the user's own web
  profile, where they may be signed in. The agent can make the user look
  at a page (as a link in its answer could), not act as them: it never
  sees the page. A page that tries something is the user's to read, as in
  their browser; downloads, camera and microphone are declined as above.
- **No focus taking.** The page never gets the keyboard or brings Pitwall
  forward, so an agent can't steer typing into a page.
- **Settings.** `agents.showWeb` (on by default) and `general.webPages`
  are as sensitive as `agents.showFiles` (Safe): `pitwall settings set`
  changes them without asking, as it does showing files. Off, the tool
  isn't listed (agents running then hear `tools/list_changed`) and
  `web.open` is refused.
- **Read-only hint.** `show_web` says `readOnlyHint: true`, as the files
  tools do, so agents that go by it (Codex) don't ask before each call.
  That is what it is for the agent: it shows the user something and
  changes nothing in the agent's environment (no file, process or
  setting), and nothing comes back. It is not nothing for the user: the
  page loads in their own signed-in profile, as a link they clicked
  would, which is why the hint also says `openWorldHint: true` and why
  the page never gets the keyboard.
- **Version skew.** A command-line tool newer than the running app
  (without the `web` capability) refuses with "quit Pitwall and open it
  again"; an older one doesn't list the tool. Pitwall not running: the
  tool is still listed and answers that.
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
  a web tile; `show_web` only opens one (above).

## Setting
`general.webPages` (on by default; Settings → General → "Show web pages in
Pitwall"). Off: no web view is made; ⌘L says so, an open tile says so in
its place, and a Preview opens in the browser; agents don't get
`show_web`. `agents.showWeb` (on by default; Settings → Agents → "Agents
can show you web pages") turns off only the agents' tool. Both are as
sensitive as `agents.showFiles`.

## Platforms
WKWebView on macOS, WebView2 on Windows, WebKitGTK (X11) on Linux, the
engines the Preview already used. Where none can be made (no WebView2
runtime, no WebKitGTK, Wayland), a page tile says why in its place and a
Preview opens the browser with a toast, once.

## Not yet
Tabs, favicons, search, find in page, zoom, more profiles. A terminal
folded for lack of room when a page tile opened doesn't unfold when it
closes (the layout's own rules). Agents reading or driving a page: not
planned.
