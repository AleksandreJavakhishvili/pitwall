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
  opened (no Preview rules; in its project's browser profile, below),
  remembered as that agent's
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
- **The user's profile.** A page an agent opens is in the user's browser
  profile for its project (Browser profiles, below), where they may be
  signed in. The agent can make the user look
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
- Pages (not Previews): one persistent store per browser profile (below),
  each with a folder in Pitwall's data folder (`<data>/webview/profile-<id>`,
  Default's `profile-default`, `preview::profile_dir`; the folder's name is
  the profile's id). Windows (WebView2) and Linux (WebKitGTK): the store is
  in the folder. macOS: Default is the app's default WebKit store
  (`~/Library/WebKit/<app>/WebsiteData`), where every page has been so far;
  each other profile is WebKit's own store keyed by its folder (macOS 14+,
  `dataStoreForIdentifier`, the data under
  `~/Library/WebKit/<app>/WebsiteDataStore/<id>`). The app's, never
  Safari's.
- Previews: a store per project on Windows and Linux
  (`<data>/webview/<project>-<hash>`). On macOS they are in the app's
  default store, with Default's pages: the per-project identifier store
  was never used there (the check asked the class for an instance method;
  found while building profiles and left as it was, so nobody's sign-ins
  move).

## Browser profiles
Separate sign-ins for the pages (work and personal Teams, Slack, mail),
tied to Pitwall's accounts so the right project gets the right logins.
Status: built (`crate::web::profiles`).

- **A profile is an account** of kind `web` (`pitwall_core::accounts::WEB`,
  "Browser profile"): a name ("Work") and its folder, in `accounts.json`
  beside the tools' accounts and the git identities. Default is the store
  every page used before profiles (its folder `profile-default`; on macOS
  the app's default WebKit store), so no one loses their sign-ins.
- **Made, renamed, removed** in Settings → Accounts → Browser profiles, or
  made from a project's Accounts… (Browser profile → + New profile, which
  also makes it the project's). No login step: the user signs in to sites
  in a page opened in it. Names are unique among profiles; "Default" is
  taken. Its id is `web-<name>`, never one whose folder is still on disk.
- **Removed:** asked first ("its sign-ins, cookies and site data are
  deleted from this computer"); refused while any page is in it (in a
  layout or a window, shown or not): the user switches those pages or
  closes them first. Then its projects go back to Default, its store is
  deleted (macOS 14+: `removeDataStoreForIdentifier`; Windows, Linux: the
  folder), and its folder last; tried again a few times while the engine
  lets go of it. A folder left behind keeps the id from being reused.
- **A project picks one** in its Accounts… dialog (a "Browser profile"
  row of chips under the agents' logins and the git identity: Default,
  each profile, + New profile), kept in `accounts.json` `projects[<folder>]
  .web` like a tool's account. It doesn't move agents or restart anything.
- **Which profile a page opens in:**
  - an agent's (`show_web`, `pitwall open --web`): the profile of that
    agent's project, else Default. The agent can't pick or name one (no
    argument): the project decides. The answer names it ("It is in the Work
    browser profile …"), so the agent can tell the user.
  - a page the user opens (⌘L, "Open web page…"): the profile of the
    project of the agent whose tile has the focus in the active space; no
    focused agent (an empty tile, a files or web tile): Default.
  - Previews: their project's own store, as before; no profile.
- **The project's profile changed** while its agents' pages are open: they
  stay as they are (no reload under the user). The agent's next `show_web`
  moves its page into the project's profile now (loaded again there, on
  the new address or the one it shows) and the answer says so. The same
  holds for a page the user switched by hand: an agent's page is put back
  in its project's profile when the agent shows it again.
- **The bar** shows the page's profile as a small chip next to the address
  once there is a profile besides Default (or the page is in one); Default
  is subdued. Its menu lists Default and every profile, the page's ticked;
  picking another switches it: a page is bound to one store, so its view is
  let go and made again in the other on the address it shows (history and
  the page's state go). The same bar in a window of its own. Not on a
  Preview.
- **Kept and restored:** `ui.json` `webTiles[id].profile` (absent:
  Default; an older Pitwall ignores it). A page comes back in its profile;
  one whose profile is gone (removed while Pitwall wasn't running, or
  accounts.json edited) comes back in Default with a note under its bar,
  once (its entry forgets the profile).
- **Isolation is the point.** Two profiles never share cookies, local
  storage, caches, service workers or HTTP auth: each has a store of its
  own. macOS 14+: Default the app's default store; every other profile one
  `WKWebsiteDataStore` per identifier (`webview::Options::own_store`),
  derived from the profile's folder path (`webview::store_id`, stable
  across runs; another Pitwall data folder, such as a test instance's,
  gets other identifiers). Windows: one WebView2 user data folder, one environment,
  per profile (several coexist in one process; each folder gets its own
  browser process). Linux: one WebKitGTK data manager (data, cache, cookie
  file) per profile folder, several contexts in one process.
- **Before macOS 14** there are no separate persistent stores: only Default
  is used. A project's profile is ignored there, the bar's menu shows the
  others greyed ("needs macOS 14"), Settings says so, and a page kept in
  another profile opens in Default with a note. No page is ever made in a
  store it would share with another profile (the macOS backend refuses
  one, too).
- **Checked by hand** on macOS 26 (an isolated test instance, a local
  page setting a cookie and local storage): Work, Personal and Default
  never saw each other's; Work's came back after a restart; a removed
  profile's store and folder were gone, and one made again under the same
  name started empty. Windows and Linux: tests only.
- **What leaves the machine:** nothing. The list (names, ids, folders) is
  in `accounts.json`; site data stays in the engine's store. Agents still
  get nothing from a page, in any profile.
- **Older Pitwall** reading a newer `accounts.json` keeps the profiles (a
  kind it has no agent for, shown nowhere) and the tools' accounts. With
  the sidebar grouped by account it may file a project that has a browser
  profile but no tool account under that profile's name (it takes a
  project's first account for its terminals). Pages open in Default there.
- Browser profiles aren't sidebar sections: grouping by account stays per
  tool login; a profile never restarts agents.

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
Tabs, favicons, search, find in page, zoom. A terminal
folded for lack of room when a page tile opened doesn't unfold when it
closes (the layout's own rules). Agents reading or driving a page: not
planned.
