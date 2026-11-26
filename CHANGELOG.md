# Changelog

All notable changes to Pitwall. Generated with [git-cliff](https://git-cliff.org) from [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/); see [CONTRIBUTING.md](CONTRIBUTING.md#commit-messages).

## Unreleased

### Highlights

- Next up moved to the agent's tile, and the right Details panel is gone. While prompts wait for an agent, its tile's header shows "2 queued"; click it (or "Show Next up for <agent>" in the command palette) for a small popover with the numbered queue, Send now and Remove, items an extension queued marked as before, the Auto-send switch and the last prompt sent. Esc or a click outside closes it. You still queue with "name: prompt" in the palette, Review's "Add to Next up", `pitwall queue` or an extension, and Auto-send works as before; the prompt box that sat in the panel is gone. So are the panel's Changes list and Files tab (Review and the files viewer have both), its worktree list (the sidebar has it), the top bar's Details button and ⌘. (Ctrl+>). An extension's right-panel sections (`contributes.panels`) still install but aren't shown, and `pitwall extension check` says to make them views.
- A simulator an agent starts opens in Pitwall, beside that agent. When an agent boots an iOS Simulator or starts an Android emulator (`simctl boot`, `flutter run`, `expo run:ios`, `emulator -avd`, …), its screen opens in a device tile right of the agent's terminal, without taking the keyboard. A boot Pitwall can't tie to one agent is only offered ("iPhone 17 started", Open in Pitwall), and a tile you close stays closed. Ask an agent to "show me on the simulator" and it boots one if needed and opens it the same way (Pitwall's MCP tool `show_device`, or `pitwall open --device`); it only shows you the screen. `general.deviceAutoOpen` (beside, offer, off) and `agents.showDevice` turn each off.
- Agents can set up your extensions. Ask the Race Engineer (or any agent) to set one up and it reads where it stands with `pitwall extension status`, the same checklist as its card plus the next step, and moves it forward one step at a time. Each change (install, turn on, a setup action, confirming and starting its server, its first sign-in, a setting) shows you an approval in Pitwall naming the agent and listing exactly what will run; nothing runs until you approve, and a declined step isn't asked again. Keys never go through the agent: it opens Settings next to its terminal at the field, and you enter the key there. A sign-in link opens in your browser and the agent only hears that it did. The agent can't grant more permissions than turning on shows, delete an extension's data or let it send without asking. `pitwall extension list`, `status` and `log` read; `pitwall extension --help` lists the rest.
- Updating an extension no longer breaks running agents or its server. Update is now one ordered step, listed in full on the review with every command: Pitwall stops the extension's server and backs up its data, installs the new files, updates the server's program if the new version needs it, starts the server and checks that it and the inbox come back, and only then updates the plugin in your agents. If the server doesn't come up, Pitwall puts back the previous extension, its data and the previous server by itself, and says what failed. A setup that fails for one account keeps the update and offers Retry. Go back asks whether to put the data backup back too. Agents started before an update keep the setup they started with until restarted: their row and pane show *old setup*, the extension's card lists them with Restart now and Restart all idle, and `pitwall agent list` says so. Settings → Extensions → "After an update, restart agents that use the old setup" is Ask (default), When idle (only once an agent has waited for you for 30 seconds, one at a time, continuing its conversation; never one that is working or waiting on an approval) or Never, and each card can override it. Agention 1.3.0 backs up its database with `agention backup`. For authors: `service.backup`, the manifest's `update` block, and `docs/extensions/updates.md`.
- Agention from its releases. The operator API Pitwall's inbox uses shipped in Agention v0.5.0, so the extension (now 1.1.0; Settings → Extensions offers Update) pins Agention's plugin to v0.6.0 and no longer needs a pre-release server. A new button, "Install Agention's server here", installs that server on this computer with npm after you confirm. A server from a pre-release build can't be upgraded in place: start the released one with a new data folder.
- Agention 0.7.0. The extension (now 1.4.0) pins Agention's plugin and server to v0.7.0. Its plugin refuses an older server, so Update first installs the 0.7.0 server Pitwall runs for you, after a backup of its database, and only then sets up your agents again. Agents already running keep working with the new server until you restart them. If you run your own server, upgrade it to 0.7.0 before you set up your agents again.
- Extensions can keep a local server running. An extension whose agents need a server on this computer can declare it, and Pitwall installs it through the extension's own install button, checks its version, starts it, restarts it if it stops (after 1, 5 and 30 seconds, then it gives up and shows the server's last lines), and stops it with everything it started when you turn the extension off, update or remove it, or quit. Nothing runs until you confirm the exact command once, and an update that changes it asks again. The server gets only your PATH, home and language, plus what the card lists. Its output is in the extension's Log, and its data is in a private folder of its own that Remove keeps unless you say otherwise. A server you already run on the same address is left alone: the card says "running elsewhere" and uses it. A first sign-in button opens the one-time link the server prints in your browser and keeps it nowhere. Agention 1.2.0 uses this: turn it on, confirm, sign in. For authors: `pitwall extension new --service` and `docs/extensions/service.md`.
- Questions from your extensions' inboxes in the floating "needs you" control. While you're in another app, the open questions of an extension that is on (Agention, the demo inbox) are cards under your agents': who asks and in which thread, the suggestions to click, a field for your own answer, Decline and Open in inbox. It's the inbox's own question, so answering it in either place answers it in both. With no such extension on, the control is as before.
- The floating "needs you" control says who waits and what for. Instead of a tile with a count, it's a pill with the agent's logo and name and one plain line of what it wants ("Run: pnpm test", "Edit src/cart/total.ts", the question), with Accept and Reject right on it for a plain yes or no, and "+2" when others wait. Opened, it shows one prompt at a time, oldest first, with "1 of 3" and arrows: a summary made of lines of the prompt itself over the terminal rows that ask, and Show all. ⏎ accepts and esc rejects, both re-checked against the screen like the buttons; ←/→ go between prompts. After an answer it says "Sent to api-fix" for a moment and moves on; if the prompt changed, "The prompt changed: nothing was sent." Accept all appears only when every prompt is a yes or no.
- The floating control's size is yours. Resize the open card by any edge or corner; go back to the pill and a click brings it back at the same size. A double click on an edge goes back to the default, and the size is kept for the next start.
- Writing extensions. `pitwall extension new my-inbox` writes a working starter (a manifest and a small Node adapter showing one inbox thread), and `pitwall extension check` and `test` check a folder offline: its manifest, what it would run and set in plain words, and its adapter actually answering. In Settings → Extensions, "Load from folder…" uses your folder where it is, never copied, marked "development", with Reload; Log on any extension shows what Pitwall and its adapter said to each other, secrets taken out. Stopping an adapter now stops everything it started. The author's guide is in `docs/extensions/`.
- Extensions update from their own releases, without waiting for Pitwall. An extension whose manifest names a git `source` is checked at start, once a day and with "Check for updates" on its card; a newer release tag is fetched only when you press Review, which lists what changes on this computer (its program, the variables it sets on agents, its setup commands, settings and source), and installed only when you press Update. The card shows where it came from (tag and commit) and when it was last checked; "Go back" returns the copy from before the last update. A setup action whose commands changed is unticked and asks to be run again. "Add from Git…" under Available installs one from an https repository the same way. Pitwall never runs anything from a fetched release, refuses a tag that moved, and never asks for a password. Agention's extension now releases from this repository, tags `agention-v*`.
- Extensions from npm. An extension's manifest can name an npm package as its `source` instead of a git repository, and "Add from Git or npm…" under Available installs one by its package name (npm's registry, or another https one). Checks, Review, Update and Go back work as for git, and the card says "From npm: …" with the version. Pitwall reads the registry and downloads the package itself, with no sign-in: it checks the download against the registry's sha512 integrity, unpacks only plain files, refuses a version that changed after it was seen, and never runs npm, node or a package's scripts, nor installs its dependencies. For authors: "Publishing on npm" in `docs/extensions/manifest.md`.
- Extensions can read what Pitwall knows, with your permission. An extension written for the new manifest format 2 says which permissions it asks for (your agents, your projects, your worktrees, its own connections), each with its reason, and turning it on shows them first: grant or refuse each, and nothing runs until you press Turn on. Its card lists them with the date you answered and a switch; turning one off applies to its very next call. An update that asks for more says so before installing, and the new permission stays off until you allow it. A program the extension starts gets the same grants on Pitwall's socket, your own terminal is unchanged, and Activity, next to Log, lists every call it made and what was refused. Extensions also get text, choice, switch and number settings. Existing extensions work as before. For authors: `pitwall extension new --protocol 2`, and the new pages in `docs/extensions/`.
- Extensions can add view tiles, commands, badges and notifications. A view opens from the command palette ("Open …") as a tile next to your terminals, showing a list with the selected item's details, a page of fields and text, or a status. Like the inbox, you can move it, maximise it, open it in its own window, and it's back after a restart. An extension's commands appear in the palette under its name. It can put a count or a few words on an agent's or a project's row in the sidebar, and a click opens what it points to. With the new Notify permission it can send you notifications, at most three a minute, only while Pitwall isn't in front, each marked with the extension's name. Before you turn an extension on, Settings shows what it adds, and an update's review lists what changes. Pitwall draws all of this itself, and none of it lets an extension act on your agents. The demo inbox now has a "Demo runs" view to try. For authors: `pitwall extension new --view`, and `docs/extensions/views.md`.
- Extensions can act, each step behind a permission you grant. All four start off. "Show you files and pages" opens a file at a line or a page next to an agent, as the agents' own show tools do, without taking the keyboard or bringing Pitwall forward. "Focus an agent" moves the focus to an agent's tile, but only while Pitwall is in front. "Send text to your agents" puts text in an agent's Next up, drawn in its own colour with "From <extension>". It waits for you to press Send: Auto-send skips it unless you turn on "Send what <extension> queues without asking", which is off and comes with a warning. It's at most 4000 characters, one item every 10 seconds and three waiting per agent, and you can remove it like your own. "Start agents" asks you every time, showing the project, the tool and the first prompt in full. Nobody answering in time means no, and it's limited to three requests in ten minutes. The two that reach your agents explain the risk in plain words on the permission screen and in an update's review. None of them changes what an agent itself may do. Activity logs every call with what it asked for, refused or not. View buttons can use the same actions, and a button without its permission is shown disabled, saying which permission it needs.
- Settings can be a tile. "Open as a tile" in the Settings dialog's header (or "Open Settings as a tile" in the command palette) puts the same Settings, every page, in a tile of the layout at its full size, with the page list beside the page, or above it in a narrow tile. Like the other tiles it focuses with a click, maximises with ⌘⏎, moves by its bar, opens in a window of its own and back, and is there after a restart at the page you left. There's only ever one Settings: the dialog hands over what you were typing, and while the tile is open, the Settings shortcut goes to it.
- Agents can show you Settings. Ask an agent where a setting is and it opens Settings at that page in the Settings tile next to its terminal, or switches the tile you already have (Pitwall's MCP tool `show_settings`, or `pitwall open --settings <page>`; `--extension <id>` unfolds an extension's card). It only shows you Settings: the agent gets nothing back from it, can't change anything there, and the keyboard stays where it was. Off with Settings → Agents → "Agents can show you Settings" (`agents.showSettings`). Extensions can't use it.
- Extensions can add a web page that opens from the command palette in a web tile. The page has its own browser data, apart from your accounts, and can't reach Pitwall: https, or a local address on this computer only. For authors: `pitwall extension check` describing the new permissions, and `docs/extensions/actions.md`.
- Switch a files tile's or window's folder yourself. The folder in its bar opens a menu: your repository's worktrees with their branches (the one shown ticked, filter by typing), "Open folder…" with a path field (`~` works) or the folder picker, "Up to parent folder" and "Back to the agent's folder". A folder's right-click menu in the tree has "Open as root", and the command palette has "Files: open folder…" and "Files: switch worktree…". The new folder shows its own branch and git status, tabs that are there too stay open, unsaved edits are never dropped (the switch is refused until you save or discard them), and the folder is kept for the next start. Folders on this computer only; an agent on another machine's files keep its folder.
- A text editor. "New file" or "Open file in editor…" in the command palette, or "Open in editor" in the files viewer, opens an editor tile next to your terminals for any file on this computer, its files in tabs that show unsaved edits and git's status. It edits with several cursors (⌘-click, ⌘D for the next match, ⌘⇧L for every one, ⌥⌘↑/↓), comments lines out by the file's language (⌘/), types brackets and quotes in pairs, indents after an opening bracket, goes to a line (⌃G), and marks the lines changed since git's last commit in the margin. ⌘S saves, ⇧⌘S saves as, ⌥⌘S saves all; a new file is named on its first save, and closing a file, the editor, its window or Pitwall asks about unsaved edits. When an agent changes an open file, the editor shows it at once if you have nothing unsaved; otherwise it asks, as the viewer does: Keep mine, Take theirs or Show difference. Files up to 2 MiB are edited, up to 10 MiB shown read-only. Like the other tiles it moves, maximises, opens in a window of its own, and is back after a restart with its files (not their unsaved text). There's no autocomplete, language server or debugger.
- Start an agent and give it its task in one command. `pitwall agent new --kind claude --project ~/code/api --name api --prompt "fix the failing tests"` (or `--prompt -` for stdin, `--prompt-file task.md`) puts the task in the new agent's Next up as it is created, as `pitwall queue add` would, and it goes in when the agent is first free, as long as Auto-send is on. It works for any kind, terminals included, and with `--machine`, where the approval shows the task too. The JSON gains `queued` and the text output says "Queued its first instruction". An empty prompt creates nothing. If the agent is created but its task can't be queued, the command says so, prints the agent and exits 1 (`queue_failed`), so a script can retry with `queue add <id>`. Nothing else is asked of you beyond what `agent new` and `queue add` already ask. The `pitwall` skill now shows this for "start an agent to do X".

### Fixes

- An extension's setup now covers every account of a tool. Setting up an extension such as Agention ran its commands (`claude plugin install …`) only for the tool's own folder, so agents on your other Pitwall accounts of that tool didn't get its plugin, while Settings showed setup as done. A setup step for a tool now runs for the tool's default folder and for each of your accounts of it, each with that account's own folder, and the confirmation lists every command under the account it is for. The step is ticked only once every account is covered; otherwise it says which accounts still need it. Setup also keeps covering accounts you add later: Pitwall runs the commands you already confirmed for the new account by itself, while the extension is on, and never runs commands you haven't confirmed (after an update changes them, it asks again). If you have accounts and set up an extension before this version, Pitwall finishes setting it up for them at start, or the step asks you to run it again.
- Asking for a worktree now shows that worktree. An agent whose files were shown from a folder above its repository (its home folder, a projects folder) asked for a sibling worktree by its path, its folder name or its branch, in a tile or a window, and the viewer stayed on the home folder with the worktree a folder in it, or answered that no such worktree exists. A `root` is now always the root, with that worktree's branch and git status; a name is also looked up in the repositories inside the agent's folder; a folder in another worktree roots the viewer there; and the answer gives the root actually shown, with a note when it isn't plainly what was asked. Agents are told to pass `root` for a worktree (`pitwall open --root`, the `root` of `show_files` / `show_folder`).

## [0.8.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.8.0) - 2026-11-22

### Highlights

- Web pages in tiles. "Open web page…" (⌘L / Ctrl+Shift+L) opens a page in a tile of the layout next to your terminals, in the system's own web view: type an address (`example.com`, `localhost:3000`; there's no search), go back and forward, reload, open it in your browser. An agent's Preview of its dev server is such a tile too, right of its terminal, still following what the terminal prints. Web tiles move, resize, go into a window of their own and come back after a restart like files tiles do. Turn it off in Settings → General.
- Agents can show you web pages. Ask an agent to open a page, a local dev server or an empty browser in Pitwall and it opens in a web tile next to its terminal: Pitwall's MCP tool `show_web`, or `pitwall open --web` from a shell. It only shows you the page: the agent gets nothing back from it and can't click or type in it, and it never takes the keyboard or brings Pitwall forward. Turn it off in Settings → Agents (`agents.showWeb`).
- Browser accounts: separate sign-ins for the web pages in Pitwall, such as a work one for the work Teams, Slack and mail and a personal one for yours. Add one in Settings → Accounts or from a project's dialog. Pages the project's agents open are in it, and so is a page you open while one of its agents has the focus; everything else stays in Default, where your current sign-ins are. Each page's bar shows its account and can switch it. Each keeps its own cookies and site data on this computer, and removing one deletes them. Needs macOS 14 or later on a Mac (before that, Default only).
- Profiles. A profile ("Work", "Personal") bundles what a project's agents run with: the account of each tool, the git identity, the browser account and each extension's connection. Make them in Settings → Profiles; a project picks one from its right-click → Profile…, or by dropping it on a profile in the sidebar, which groups projects by profile once you have one. A project can still change a single pick for itself. Changing a pick moves the project's running agents right away, as changing an account did. With no profiles, everything looks and works as before. Also in the CLI (`pitwall profile`).
- Extensions, a first draft. An extension is a folder with a short manifest in the `extensions` folder of Pitwall's data folder; it can run a small adapter process that brings its data in, keep a server address and an API key (in the system's secure store), give agents Pitwall launches a few environment variables, and offer setup buttons that run commands only after you confirm them. Settings → Extensions lists the ones Pitwall ships under Available: Install, turn on (nothing of an extension runs before that), then a numbered setup checklist that ticks as you go and shows a setup action's steps live. Update and Remove are there too.
- An inbox for your agents' questions. An extension's inbox opens as a tile of your layout (⌘I, or "Open … inbox" in the command palette): questions first, each a card you answer with a click or a number key. An agent with an open question gets a badge on its sidebar row, and a new question notifies you as an agent that needs you does. The tile moves like a web tile, into a window of its own and back. Agention is the first extension, with a demo inbox beside it.
- Several connections per extension, such as a work and a personal server. Each has its own settings and secrets, a profile or a project picks one, and the inbox shows all of them or one. What you had set up before becomes the default connection, with the same saved secrets.
- Files, web and inbox tiles are focused and maximised like terminals. A click in one makes it the focused tile, with a border, and gives it the keyboard, so ⌘⏎ (Ctrl+Shift+Enter on Windows and Linux) maximises and restores it with what it had open. ⌘↵ in a text box that sends with it, such as the Next up queue box and review comments, sends instead of maximising.
- Move around a zoomed picture with the mouse: drag an image, an SVG's picture or a PDF's pages, or scroll with two fingers. Pinch on a trackpad or ⌘-scroll (Ctrl+scroll on Windows and Linux) zooms about the pointer; a double click zooms about the point clicked.
- Files windows come back. A file viewer window open when you quit Pitwall opens again at the next start with the same folder, tabs, place and size. Unsaved edits aren't kept (quitting asks about them first).
- Checked by hand on macOS. The Windows and Linux builds pass their tests but have not been run by hand, and Agention was tried against a pre-release build of its server.

### Features

- **website:** Show agents opening files in a files tile ([`f536d24`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f536d24c418d6d5f9d6a60a4b36490d9ba4fb671))
- **app:** Drag, pinch and ⌘-scroll images and PDFs in the viewer ([`ed807fb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ed807fbd0298e8fd2958b1de990d8231469a0ae7))
- **app:** Open web pages in tiles; Preview becomes one ([`bd02f20`](https://github.com/AleksandreJavakhishvili/pitwall/commit/bd02f20341c3bda4b7da496baaa54469f879a432))
- **website:** Show features as a carousel, with an accounts slide ([`ef6053c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ef6053c2591b347bcb44a373ed053cd1c8ec1076))
- Add the show_web tool and web.open for agents ([`7f7fb14`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7f7fb1419ea16fec73d7540b52c3491a81b1c243))
- **app:** Open agents' web pages, and web pages in their own windows ([`71bfb72`](https://github.com/AleksandreJavakhishvili/pitwall/commit/71bfb725362d11ce9203a84dedc07ecf972d3a98))
- **app:** Open files windows again after a restart ([`2681a46`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2681a46e73070d58e33166494a93c8a16a8e8e45))
- **core:** Browser profiles as accounts of kind web ([`083bf85`](https://github.com/AleksandreJavakhishvili/pitwall/commit/083bf8548f3703341b32c01230adcd3c8ae1c0d4))
- **app:** Web pages open in their project's browser profile ([`47e3523`](https://github.com/AleksandreJavakhishvili/pitwall/commit/47e352315afa67ae99a63e86d1c94b0bdc6f69ea))
- **app:** Manage browser profiles in Settings and a project's Accounts ([`659153b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/659153b8e6cd3d4cf6081a62536b58ef3cecdbd6))
- **core:** Find extensions and check their manifests ([`2222028`](https://github.com/AleksandreJavakhishvili/pitwall/commit/22220289e995c9880f1968a1d9ae20d4cb149fa5))
- **app:** Turn extensions on in Settings and show them in the sidebar ([`8ec712e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8ec712e8c509dd590343996400b11477a4825a3f))
- **core:** Spec the extension adapter protocol and richer manifests ([`8c01591`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8c015915a74e0078dcffbbbb2cecf587567e307b))
- Add the Agention adapter, its API client and a fake server ([`321019b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/321019b9a027a58358dafaa0a248e18e76a01c90))
- **core:** Carry counts, blocking asks and answers in the protocol ([`9899903`](https://github.com/AleksandreJavakhishvili/pitwall/commit/989990326a11745cd1a6b382203861af5234ec06))
- **app:** Run extension adapters and keep their secrets ([`337498d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/337498d6aa5a8c14a351fa0df5f057e72bdb5e3d))
- **core:** Set extensions' variables on agents Pitwall launches ([`9b6a4fb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9b6a4fba315de9d10ddeb0cf09a2111c63af9ced))
- **app:** Run an extension's setup actions after confirming them ([`dcc2333`](https://github.com/AleksandreJavakhishvili/pitwall/commit/dcc2333135a469effd171f164b03ee9d4134690f))
- **app:** Open an extension's inbox as a layout tile ([`187b31c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/187b31c26cf5aab17948098f62e6af1bb15b1050))
- **app:** Polish the inbox tile after looking at it ([`6f54ebd`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6f54ebd30797032e1fb40a9fade88f44108077cd))
- **app:** Grow the inbox reply box with its text ([`d0fed4d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d0fed4dedcbdf64531651f7485ed9dde96ad7d88))
- **core:** Install, update and remove shipped extensions ([`8f844d3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8f844d34617b63fc0fac5cbe46fd23a64eb5e686))
- **app:** Give extensions a Settings tab with guided setup ([`1335e6b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/1335e6b7f4c91ed72283f91516c7faa089be1241))
- **core:** Resolve every pick through profiles and connections ([`d8ec0d9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d8ec0d95cdaf87b363963299842b6862d989187d))
- **app:** Run one extension adapter per connection ([`ebb4f46`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ebb4f46ad107f07b29489c3d3cbcc4f365d5db56))
- Call browser profiles browser accounts in tool results ([`0bb1f06`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0bb1f06a47d6c6d4eab1b015d0ea1fb6554080f3))
- **cli:** Manage accounts, profiles and project picks ([`741d597`](https://github.com/AleksandreJavakhishvili/pitwall/commit/741d597eb8ac59b9c85453aaf7314bf88164728c))
- Say browser account wherever the user reads it ([`c42fca0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c42fca08da713bd3ae03f6a8cf71a5a7fe812756))
- **app:** Set up each extension connection on its card ([`1c44b5f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/1c44b5fa7b823eedc62ef712a037e5526646a1a8))
- **app:** Fill profiles in Settings and pick them per project ([`05f7b99`](https://github.com/AleksandreJavakhishvili/pitwall/commit/05f7b9938ab54ef00d44c6c6cd0f0448b8018b9a))
- **app:** Show profiles only when there is a choice to bundle ([`aee151f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/aee151f847c7a4ba981d0344547e1cd289ef24ae))
- **app:** Filter the inbox by connection and say which is offline ([`e831e48`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e831e489ba85aa06d7de332c2e8bdad162814f85))
- **app:** Lead with profiles in Settings and the project dialog ([`4aaedb4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4aaedb4f32cd4bbf24a36591da2ed11ef69fed1c))
- **app:** Group the sidebar by profile once one exists ([`4af6406`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4af6406fc2015fdee908677379f9ad18f661d60b))
- **website:** Your agents' workstation, and a web pages slide ([`c1d9b48`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c1d9b4831a13cf98f630a3c6689b2e143915368f))
- **website:** Show browser profiles, add web pages to How it works ([`6b2ee1e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6b2ee1ecb63662e77263b4b4a7caecd7c6283519))

### Fixes

- **app:** Pass a typed nil block when web views pause media ([`f2f9156`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f2f91567197d9891cb266a2bef625662b70a0b93))
- **app:** The web tile's address field takes the bar's room first ([`fda3e76`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fda3e763baf8e050070fbb79b23c6def940ed435))
- **app:** Web tiles identify as Safari on macOS ([`9b77b92`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9b77b923d3c23f6012093f23b32dd0be838a44b6))
- Say a moved page is the agent's own, keep bench web windows away ([`cf373eb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/cf373eb63285f38566a98d5cfff3ba869796bcf8))
- Mark show_web read-only, as the files tools ([`98e6e77`](https://github.com/AleksandreJavakhishvili/pitwall/commit/98e6e772b8ff9ff862e6022dc7cb275e91f69e36))
- Give agents.showWeb and webPages the sensitivity of showFiles ([`22fad6f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/22fad6f021bf38a1398cf6add358cba8c528745f))
- **app:** Give browser profiles their own WebKit stores on macOS ([`daddb17`](https://github.com/AleksandreJavakhishvili/pitwall/commit/daddb17134be4d409ab4ebcbaeb3138ceef0a64f))
- **app:** Draw a space's tiles when there are no agents ([`9058166`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9058166107be222f6cbbde1427c818c8047ea934))
- Run only the Agention adapter's test files with npm test ([`9b0e269`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9b0e269be2f461462106851ae487a1f0bba05848))
- **app:** Keep typing in the inbox's fields, and move it as a tile ([`49e5d61`](https://github.com/AleksandreJavakhishvili/pitwall/commit/49e5d612624ff9ccb51595826a3b1386c9de61a4))
- **app:** Let text fields keep Cmd-Enter over the tile's Maximise ([`fab65c3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fab65c31eac0017f4419e9bd8de5d56a30e193dd))
- **app:** Focus and maximise files and web tiles like terminals ([`e55fc70`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e55fc70f01f34ebd53422699e46ead8dc341b456))
- **app:** Keep the project dialog within the window ([`be27f95`](https://github.com/AleksandreJavakhishvili/pitwall/commit/be27f95138a77b81c7be1f0fda90c1fbaef3da54))

### Refactors

- **app:** Drop the extensions' sidebar sections ([`033f262`](https://github.com/AleksandreJavakhishvili/pitwall/commit/033f26245caa0cd72eb6949065307f81b9630a6f))

### Documentation

- **website:** Explain files agents show you in How it works ([`9b05013`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9b050136144b3c8a76cc4ad366ad4abdaff91241))
- Describe moving and zooming pictures in the viewer ([`6d164e6`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6d164e6e880857e12b314250ad5f53bd58259b08))
- Spec web tiles and the Preview as a tile ([`f702c75`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f702c757b9a9a180c5740c6dd156a1afca61f697))
- Spec agents' web pages and pages in windows ([`680bf16`](https://github.com/AleksandreJavakhishvili/pitwall/commit/680bf16afdcc73fa41fd551801cdb6113ba62462))
- Spec browser profiles ([`4cd6d2a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4cd6d2abb6d54ecaa35afc41ec04abd202fd8596))
- Say where each page's store is on macOS ([`dae32f4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/dae32f4c454d533abbf0b65ee718569afc80238c))
- Spec extensions as a draft with panels left open ([`f20edd0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f20edd0bd8a6d01da2943f2a0f790a87949aa3bc))
- Say when an extension's code runs in the changelog ([`fd4243d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fd4243d73ad21188eb5a24027c0ea206702f8f4f))
- Describe installing extensions and the Extensions tab ([`5575e98`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5575e98a66d765f6d421d06dbb14c9223028f5af))
- Say how the inbox moves, takes keys and maximises ([`520e887`](https://github.com/AleksandreJavakhishvili/pitwall/commit/520e887b1eedf99646c40bff19e51196898dc581))
- Say that files and web tiles focus and maximise like terminals ([`b28556d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b28556d525f0f248de0d0dbb473ae0811603b7ce))
- Describe extension connections and profiles ([`29d7f73`](https://github.com/AleksandreJavakhishvili/pitwall/commit/29d7f7355546f6d16dc7fceb96fe1e13c97b526d))
- Say your agents' workstation in the README tagline ([`7c3bcc4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7c3bcc4a3d04b15d63bda73174dd1f800e666074))
- **website:** Say browser accounts, and what a profile bundles ([`11b4c01`](https://github.com/AleksandreJavakhishvili/pitwall/commit/11b4c011cd28041d4a622b04651076d2efc5b6d7))
- Highlights for 0.8.0 ([`fe24577`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fe245772fcbb23164f93ed214119a8df3991ad27))

### Tests

- **app:** A browser profile is no project's sidebar account ([`fa0fa94`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fa0fa9432d18074b71ee9db8c4e194c5f4e58aab))
- Cover the Agention adapter's wait loop, model and stdio ([`2955dbb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2955dbbeb1381f9e801fd42de4161c0060d82345))
- Check every extension folder from its own fixtures ([`36692ee`](https://github.com/AleksandreJavakhishvili/pitwall/commit/36692ee2d8fd5811f71256fedafcc56bd870aeab))
- Keep extensions' own tests out of the vitest run ([`4c6c00f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4c6c00fa7107f2c44a86f58a9c6f6e4e91542ff0))
- **app:** Press the platform's Maximise and send keys in inbox tests ([`1b61ec5`](https://github.com/AleksandreJavakhishvili/pitwall/commit/1b61ec506bd71f6ccdb52548d785197b3d0164a8))

### CI/Build

- Ship the repo's extensions with the app ([`a4a2425`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a4a2425e9396ffdbd6a4251109b897c26dc7fffa))


## [0.7.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.7.0) - 2026-11-19

### Highlights

- Agents can show you files. Ask an agent Pitwall started to open a file (at a line, which is marked), a folder, a worktree or an image, and it opens in a files tile: a slot of its own in the layout of the window that shows the agent, right of its terminal, or left, above or below it, in place of a terminal, or in a window of its own when you ask for it separately. Neither takes the keyboard or brings Pitwall forward. Turn it off in Settings → Agents.
- Files tiles move like terminals do. Drag one by its bar anywhere in a Pitwall window's layout (beside any tile, along an edge, or onto a terminal to take its place: that terminal keeps running and stays in the sidebar), out into a window, and back. "Move…", "Dock…" and "Open in window" do the same without dragging. Tiles come back after a restart with what they had open, and the window can be pinned above other apps.
- It works with most agents. Claude Code, Codex, Copilot CLI, Qwen Code, Amp and opencode get Pitwall's own MCP server (`show_files`, `show_folder`) at each launch, without a change to your config files. Gemini CLI and Cursor Agent only read their own MCP files, so Settings → Agents offers to add Pitwall's entry there. Aider gets a short rule to run `pitwall open`, which also works from any shell. Agents started before an update pick up the new tools without a restart.
- Files come from where the agent works now: a worktree it entered mid-session or a folder its terminal `cd`-ed to, not only where it started. The tree, branch and git status are that worktree's.
- Edit in the viewer. Change a text file where it is shown and save with ⌘S / Ctrl+S: undo, Tab in the file's own indentation, Replace and Replace All, input methods, line endings kept, atomic save. If the agent changes the file meanwhile, you choose Keep mine, Take theirs or Show difference; nothing is overwritten without asking. Files of agents on this computer only. Turn it off with `agents.editFiles`.
- The viewer shows more. Images (PNG, JPEG, GIF, WebP, SVG) fit the pane, at 100 % or zoomed. An SVG switches between its picture and its source (⌘⇧V / Ctrl+Shift+Alt+V), and the source can be edited. PDFs are drawn by the system's PDF engine with zoom, the current page and Find (⌘F). PDFs are macOS only for now: on Windows and Linux a PDF says so and offers "Open in default app".
- A design pass over the viewer: one bar with the agent, its folder and branch; tabs that scroll and mark unsaved files; a file tree that hides (⌘⇧E / Ctrl+Shift+Alt+E) and resizes by its edge, as does Review's list of changes. The light theme's code colours were redone so every one reads on the background (WCAG AA).
- Resize more with the mouse: the sidebar by its right edge, and the floating "needs you" control's opened cards by their edges and corner. Sizes are kept across restarts, and a double click on an edge goes back to the default.
- Real Liquid Glass for everything that floats, on macOS 26 with the Glass look: menus, dropdowns, the command palette, quick open, dialogs, Settings, toasts, tooltips and the floating control. Flat, older macOS, Windows and Linux look and behave as before.
- Showing files was checked by hand with Claude Code on macOS. The routes for the other agents are tested in code but not yet run against the real CLIs, and the Windows and Linux builds pass their tests but have not been run by hand.

### Features

- **website:** Show answering agents from the floating control ([`22f2490`](https://github.com/AleksandreJavakhishvili/pitwall/commit/22f2490dd2cc8c975b2772f3b469c0448de089b7))
- **app:** The floating control is Liquid Glass on macOS 26 ([`96e1612`](https://github.com/AleksandreJavakhishvili/pitwall/commit/96e161263bdd7c86a99712ddb7133549b37f9a5a))
- **app:** A host for floating surfaces as glass pop-up windows ([`203b3f1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/203b3f13adf81b1d6cf456cd7ba1a44d1c6b3ead))
- **app:** Context menus are Liquid Glass pop-ups on macOS 26 ([`8a4e0d2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8a4e0d276d27581db465d49d3888270ed51057b1))
- **app:** Dropdowns are Liquid Glass pop-ups on macOS 26 ([`da62835`](https://github.com/AleksandreJavakhishvili/pitwall/commit/da62835a8d496b33e19a59660282e531d2d8cbc5))
- **app:** The command palette is a Liquid Glass pop-up on macOS 26 ([`17d97a1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/17d97a19d1bd5bcaa481c1762070033f9f5da17c))
- **app:** Quick open is a Liquid Glass pop-up on macOS 26 ([`3bfe685`](https://github.com/AleksandreJavakhishvili/pitwall/commit/3bfe685c205122e3d80d0996a3de938638d97b12))
- **app:** The task dropdown in Review is a Liquid Glass pop-up ([`f096ad2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f096ad29b35a9906d2359336462533b263a4a35e))
- **app:** The menu of the Preview is a Liquid Glass pop-up ([`6baa702`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6baa7029c72c820b487cbdbb0320fa02428c91f4))
- **app:** The code view's menu is a Liquid Glass pop-up ([`511a57d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/511a57d59c3e50917da3d492cda661d72e5c9803))
- **app:** Passive glass pop-ups for surfaces that only show ([`ef27a53`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ef27a5359b9c90d74c0e4b397db7a7183c56de31))
- **app:** Toasts are Liquid Glass pop-ups on macOS 26 ([`43bb9ec`](https://github.com/AleksandreJavakhishvili/pitwall/commit/43bb9ec9f4783d9b71f725ef3607eff414719cc6))
- **app:** The explorer's notice is a Liquid Glass pop-up on macOS 26 ([`682c42d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/682c42d4353bd373137abc82c6dede1bb141e105))
- **app:** Tooltips are Liquid Glass pop-ups on macOS 26 ([`3b1ef98`](https://github.com/AleksandreJavakhishvili/pitwall/commit/3b1ef988a04deb8bed46f56ff08a6b9c570288f5))
- **app:** The code view's hovers and menu are glass pop-ups ([`a65251a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a65251a28291780efe5ac4c151113b407ffe6899))
- **app:** Glass pop-ups nest, and a dialog is one that stays ([`8a06946`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8a0694610960fabf54addb2457c343e5b0db94ad))
- **app:** Dialogs are Liquid Glass pop-ups on macOS 26 ([`44c1608`](https://github.com/AleksandreJavakhishvili/pitwall/commit/44c16087a6dc1d0becbaf50ff29055ae8941829c))
- **app:** Review dialogs are Liquid Glass pop-ups on macOS 26 ([`f95f39e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f95f39e61f00bb1fdf73f2cec9174c4b735ad187))
- **app:** Settings and about are Liquid Glass pop-ups on macOS 26 ([`fa6ad79`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fa6ad79b7719344055558aa71200ab8d66c7b44f))
- **app:** Resize the floating control with the mouse ([`fad5e6c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fad5e6c7c7397c689595073dabf595b1134fa2cb))
- **core:** Images are an explorer content kind ([`0c3aab1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0c3aab1061a9b28cf7e394fb20365845cd92ef11))
- **core:** Agents get Pitwall's MCP server at launch ([`8b686d1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8b686d1030dbb6be89939873491ee05d650dbdce))
- **cli:** Pitwall open and the pitwall MCP server ([`7eaa64f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7eaa64f5b7bd6d4c4a7135071e9d6d8d2fcfe213))
- **app:** Agents show files in the viewer; images drawn ([`192d11f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/192d11faa3aeb508c26d50092599aa938f97f5d0))
- **core:** The explorer reads any folder on this computer ([`a1b304c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a1b304c9f179252eddbfdb1a5cac8fc35c043871))
- **app:** Files beside the agent's terminal, or in a window of its own ([`2cf015c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2cf015c9d12945347549061b68fa48c2f88289c9))
- **core:** More agents get Pitwall's MCP server at launch ([`39705a0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/39705a0ee7fba24bdb9b53b08875875fc95bdcfa))
- **core:** Agents without MCP are told to run pitwall-cli open ([`01885b4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/01885b438704e762a28bf508556cf69b6624ce8a))
- **app:** Show in Settings how each agent kind shows files ([`066fe0f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/066fe0f6b5e146bf402c32d46d61af65e90e0e2d))
- **core:** Save a file edited in the viewer, on this computer only ([`827d4e3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/827d4e3bdf5909a1b6e5456d15223de4b36345de))
- **app:** The code view can edit a single file ([`f58574d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f58574d3e80a8db155f22746f804e9bd8d95d7e1))
- **app:** Edit and save files in the file viewer ([`d869c0e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d869c0e4981bf208ab5621b162c3edaf8dd04715))
- **app:** Files open in a window of their own by default ([`ef88206`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ef8820613d132384f926150e17438b176fa7dff1))
- **app:** Files take a slot of their own in the layout ([`2ed5caf`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2ed5caffde3a97d29e96a5c31e968eba12079c93))
- **app:** Files come from the worktree the agent works in now ([`9dc2b9c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9dc2b9c5b02da5d76cdbf2b11bd537271d0872ab))
- **app:** A kit side column that hides and is resized ([`16d5eca`](https://github.com/AleksandreJavakhishvili/pitwall/commit/16d5eca3c2c8cd7a9e476afd4927f881e56f487d))
- **app:** The file tree and Review's list hide and resize ([`5bb8d69`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5bb8d698d379351ab2c0c7f06002b07faf8a07d7))
- **app:** The viewer's new look in files tiles and windows ([`fb466f2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fb466f2ebfc40a7a42fca616ecad4de8ec89dc1a))
- **app:** Go to file and search in files in a files window ([`d289dbb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d289dbb18b06cb81226a0b9ad06451c6f97e41d9))
- **app:** Resize the sidebar by dragging its edge ([`2fd086a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2fd086a03a97db05321f59fcd1595b710139c9cb))
- **app:** Files dropped on a terminal take its place ([`bb03502`](https://github.com/AleksandreJavakhishvili/pitwall/commit/bb03502200fa806dd80682ecd1d8e06f75e2a78a))
- **core:** Send an SVG's text and read PDFs' bytes for the viewer ([`3142c52`](https://github.com/AleksandreJavakhishvili/pitwall/commit/3142c52e23493e176a7ae51e3744dff5d90dfb28))
- **app:** Draw PDF pages and find text with PDFKit on macOS ([`cafb573`](https://github.com/AleksandreJavakhishvili/pitwall/commit/cafb573b4a4958174176bc852affef8a4a3b3b69))
- **app:** Show PDFs and an SVG's source in the files viewer ([`935c13b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/935c13b934d45d40116000eafa7b85385a362830))

### Fixes

- **website:** The control is for anywhere outside Pitwall ([`22a18af`](https://github.com/AleksandreJavakhishvili/pitwall/commit/22a18af48860dd01a753987bd9cd60d28d879e8e))
- **app:** A glass pop-up leaves room for its soft shadow ([`cbe4441`](https://github.com/AleksandreJavakhishvili/pitwall/commit/cbe4441bcee54d0b61acf3be10120953c24272b7))
- **app:** A glass pop-up's window stays the main one ([`3441310`](https://github.com/AleksandreJavakhishvili/pitwall/commit/3441310f5371658a0aea5e2db472155bc151f49d))
- **core:** Keep --mcp-config's keys in one order ([`eca852c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/eca852ca6768b8443aa306a409deb35b1248478f))
- **app:** Agents' MCP server runs a stable copy of the CLI ([`4a4183a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4a4183a874ad9122fd8c8b9de576b346315a1be4))
- **app:** A line given without a column is marked ([`9940200`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9940200450e27bf6774ba2fa794eab65e70dc60f))
- **core:** The show-files rule keeps its commands as code ([`63c4062`](https://github.com/AleksandreJavakhishvili/pitwall/commit/63c406220be126e4cff5d4ee7020f9011e419c85))
- **cli:** The show tools no longer call the viewer read-only ([`8c684d2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8c684d20ca87903c30050967a468c37aaa335a63))
- **core:** The save test sets permissions through platform ([`fe2fb73`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fe2fb737b345ecfb4f37d8192a73a6535ed95c32))
- **app:** Say where a files tile is from the laid-out tiles ([`a6bab8c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a6bab8c85956f6ddece00cd298c6a093c3b71332))
- **app:** A removed agent's files close, or stay for unsaved edits ([`88044fc`](https://github.com/AleksandreJavakhishvili/pitwall/commit/88044fcb813946fbae91635d28867d9c64f2fd21))
- **app:** Light code colours that read on their background ([`bec8409`](https://github.com/AleksandreJavakhishvili/pitwall/commit/bec8409a04aa83371cbc0253b6670f9feb8e5789))
- **app:** Typing reaches fields in glass pop-ups again ([`23e204a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/23e204a388fff558bc3f22a96973b51d3ad34896))
- **app:** Ticked boxes show their mark on glass and dark ([`03f47ab`](https://github.com/AleksandreJavakhishvili/pitwall/commit/03f47abae6359eceaffc1627904278450bdd7b48))
- **app:** Drag files by their whole bar, not only the grip ([`352d6f4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/352d6f41591f03b7713b247421f9811b2e4b43fa))
- **cli:** Agents' tool servers take their tools from the running pitwall ([`9c28e8b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9c28e8bc552cab9980cd35644659901a7bd14b3f))
- **app:** Windows test builds don't use the files window's pin ([`0145cd7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0145cd768046e472409262e27c88c200dcca07a6))

### Documentation

- Roadmap up to 0.6.0 ([`21c279b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/21c279b55a727a0073866c3b05decf9ce544e1fb))
- Highlights for glass surfaces ([`e5c9617`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e5c9617ef332032f1b2926a2affd112d87a61129))
- Changelog for agents showing files ([`96a74ec`](https://github.com/AleksandreJavakhishvili/pitwall/commit/96a74ec80f5cc7dab6f06e62c410e9416af21f21))
- Changelog for files beside the terminal or in a window ([`63369b1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/63369b17345ddd48dfe63457a19aeacc0a0dde9b))
- Changelog for more agents showing files ([`91da5cc`](https://github.com/AleksandreJavakhishvili/pitwall/commit/91da5ccb8c452ebf2c4f3418257ab8ef68c97c8d))
- Editing in the file viewer ([`b174fed`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b174fede68ee97390ad0da9fa458b63e7271dd48))
- Files tiles in the layout and moving them ([`caa3580`](https://github.com/AleksandreJavakhishvili/pitwall/commit/caa3580f4cf7f826cec8b0dff0bee5ecc62a1e81))
- The hiding file tree and the viewer's new look in the changelog ([`6a4593c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6a4593c309be3a05a58e341dda4637cedc8f8c26))
- Describe the viewer's PDFs (macOS only) and SVG source ([`8360215`](https://github.com/AleksandreJavakhishvili/pitwall/commit/83602153b5934ee5f907d69b7879e28a5ea44970))
- Highlights for 0.7.0 ([`4618eef`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4618eef5fb3a4a08cfa6fdcdb4615768a978a4a7))

### Chores

- **app:** Debug commands for an ordinary window ([`c5d08af`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c5d08af39acb49fdbe98bdd3abee3b3dc365eae9))
- **app:** Debug commands to drive editing in the file viewer ([`7c88994`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7c88994e8fcdf7790adc6787bff496df6fffdf55))
- **app:** Debug verbs to answer approvals and show a files tile's tree ([`372e785`](https://github.com/AleksandreJavakhishvili/pitwall/commit/372e7854f80f39823eb997e25348af34a57a3766))
- **app:** Bring in main's sidebar resize and glass fixes ([`da341c1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/da341c11c857555ded43e765bb4e2386d1729cf7))


## [0.6.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.6.0) - 2026-11-12

### Highlights

- Answer agents without going back to Pitwall. While you're in another app and an agent asks something, a small Pitwall tile floats over your desktop, on every Space and over full-screen apps. Click it and each asking agent gets a card showing the live part of its terminal that holds the prompt, with Accept and Reject for plain yes/no prompts, Accept all when several ask, and Open for anything else.
- Or answer with the keyboard: once the cards are open, arrows, Enter, Esc and text go to the first card's agent as they would in its pane (click another card's terminal to type there), and Pitwall still doesn't come forward. The wheel scrolls its screen.
- Nothing is typed blind: Pitwall reads the agent's screen again before it sends a key, and sends nothing if the prompt is no longer the one you saw.
- The tile can be dragged anywhere and stays there. Settings → General → "Show a floating control when an agent needs you" turns it off.
- Accept and Reject keys were checked against Claude Code and Codex; the ones for Gemini, Qwen, Cursor, Copilot, OpenCode, Amp and aider are written from their prompts and not yet checked against the real CLIs. Tested on macOS; Linux and Windows builds have the control but it has not been run there.

### Features

- **detect:** The keys that answer a yes/no prompt ([`b8c9cc9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b8c9cc9293379076c05870352306af9acfb065b7))
- **core:** What a blocked agent asks, and answering it ([`80d81d9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/80d81d9e007f0b05cf77133a90867126d71adb61))
- **app:** Answer agents from a floating control ([`079384b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/079384ba6f33224ac9c743fe4ab62c977ac3d98d))
- **app:** The opened control has the keyboard right away ([`5a12795`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5a127953a00665a3631105ed2fe7e487387a4865))

### Documentation

- Highlights for 0.6.0 ([`41a3114`](https://github.com/AleksandreJavakhishvili/pitwall/commit/41a311468911a3687185c5e23fa0a7dbcfc0046c))


## [0.5.2](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.5.2) - 2026-11-10

### Highlights

- Much less CPU with many worktrees: Pitwall checks every worktree for changes about once a minute, and each git it started cost 50–100 ms of kernel time. Programs are now started by their full path, which the OS launches directly (about 30× less kernel time while git runs).
- Review's file list scrolls smoothly with hundreds of changed files: only the rows on screen are built, and scrolling the list no longer redraws the diff.
- Settings scrolls smoothly too, and the main screen is no longer redrawn behind Settings, About or the palette while they are open.

### Fixes

- **app:** Don't rediscover peers on every redraw after a failure ([`78778b0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/78778b0a06a983a4fc81325f53294a4b9d1ce477))
- **gpui:** A cached view replays only the frame before ([`b39f65c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b39f65c039bdd15b31f28b88d3e0f15a5310aef0))

### Performance

- **app:** Let a cached view keep the caches inside it ([`288583e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/288583ed6d6b4d717cfaaf3a341fc4576066adef))
- **app:** Replay the main screen under an overlay ([`b0d0791`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b0d07913e493ab5bc34e7877435a20eaee124922))
- **app:** Build only the settings parts on screen when scrolling ([`5dc0d54`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5dc0d54d38302bf42f4d1a92143419bc3a389619))
- **core:** Start programs by their full path ([`22814dc`](https://github.com/AleksandreJavakhishvili/pitwall/commit/22814dc2e493ced2676d1a6aa2d65c36602426ea))
- **app:** Draw only the review list's rows on screen ([`4d3219c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4d3219c13238b4f430ccec9c93fa63aab05912e0))

### Documentation

- Highlights for 0.5.2 ([`6479c60`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6479c60306257651c400c902b9a63c6a16686133))


## [0.5.1](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.5.1) - 2026-11-10

### Highlights

- Smoother scrolling on macOS 26: Pitwall now runs on Zed's current GPUI, which paces frames with the display (`CAMetalDisplayLink`) instead of waiting up to a frame and a half for each drawable.
- The sidebar draws only the rows on screen, so scrolling a long list of agents and worktrees no longer rebuilds every row on each frame (about 3× less work per frame).

### Fixes

- **app:** The windows preview's key names and window proc on new gpui ([`219cc36`](https://github.com/AleksandreJavakhishvili/pitwall/commit/219cc3693afeedd92c0fb9f199e214bca73504c6))
- **app:** New clippy lints in the windows and linux code ([`ee0f42a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ee0f42a50721554768b8c549ceb3d1a292044131))

### Performance

- **app:** Derive worktrees by agent when the list changes ([`f6dea2c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f6dea2c7a58e61c61a7cfa09cef80b82a9b670a1))
- **app:** Draw only the sidebar rows on screen ([`05663ae`](https://github.com/AleksandreJavakhishvili/pitwall/commit/05663aeef592b246738228de3a11b0d609e417ae))

### Refactors

- **app:** Port the app and terminal view to the new gpui ([`d08f7d7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d08f7d7a76c20ca6abfad3e58da0210b8376abec))

### Documentation

- Gpui vendored from zed, its licences and patches ([`de28d38`](https://github.com/AleksandreJavakhishvili/pitwall/commit/de28d385955fb3bc113fbe6b0b1e615f253da04f))
- Highlights for 0.5.1 ([`d702236`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d7022367feff40afd20cc0ebe8361e2dc3e51dbf))

### Chores

- **deps:** Rust 1.98.1 ([`f7028c7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f7028c719610fc2ccce61472a3cb67b8300ef8b4))
- **gpui:** Vendor gpui from zed's repository ([`73b451d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/73b451dfa425168f68b30ea0d067882ba155acb7))


## [0.5.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.5.0) - 2026-11-08

### Highlights

- Settings → Accounts: every agent login in one place, with its plan's limits and the tokens it used, read from each tool's own logs (Claude Code, Codex), next to your git identities. Gemini CLI and Copilot CLI can have more than one account too.
- Agents can tell Pitwall which worktrees they work in: `pitwall agent track <path>` (and `untrack`, `tracked`). Inside an agent Pitwall started, `$PITWALL_AGENT_ID` names it, so any agent that can run a command can say so, and its word beats Pitwall's guesses.
- Worktrees belong to the agent that worked there recently, by its hooks (this week's work outweighs last month's), not by where the folder lives. The sidebar and Review fold old, empty and deleted worktrees the same way, newest first, and offer `git worktree prune` for deleted ones.
- Review shows only the agent's own work: a branch that merged `develop` in no longer lists all of develop's history, merged-in branches are named by their merge, and Review says when other agents share the folder.
- Smoother sidebar scrolling: with the Flat look the window is drawn opaque, so macOS no longer blends the whole window on every frame; under Liquid Glass, scrolling the sidebar redraws only the sidebar.

### Features

- **core:** Read each account's usage from its tool's logs ([`9cb8c8a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9cb8c8a4693d75813ee97f234d8de9058e433e3c))
- **core:** Accounts for gemini cli and copilot cli ([`272ebcc`](https://github.com/AleksandreJavakhishvili/pitwall/commit/272ebcc98bee2eb2a57b9df48907d2304e3a5930))
- **app:** A settings page for accounts, with their usage ([`058bc35`](https://github.com/AleksandreJavakhishvili/pitwall/commit/058bc354c0883bef0e60881e4dca13bce15509d0))
- **app:** Review folds away worktrees without changes ([`3f798a0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/3f798a046e7eabd9d7eec666c35f96254e8c572b))
- **core:** Recent work decides a worktree's owner, and when it was used ([`92053b2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/92053b2738cdd49246adc60f3d4b5b221f67acd9))
- **app:** Fold old, empty and deleted worktrees the same everywhere ([`c994281`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c9942810701ef68782a17e112968c8cfd440523d))
- **core:** Let agents say which worktrees they work in ([`0d5be22`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0d5be225679b3851701cae418d78543d609315d2))

### Fixes

- **core:** A worktree is the agent's that worked there, by its hooks ([`50c1a87`](https://github.com/AleksandreJavakhishvili/pitwall/commit/50c1a87e88dca21fa45e856f87838d2160b1df50))
- **core:** Review leaves out history merged in from other branches ([`97b0596`](https://github.com/AleksandreJavakhishvili/pitwall/commit/97b0596b96e9d4e206597c31a18f88fb0abef639))
- **core:** Name merged-in branches by their merge, not the agent's own ([`2b591f3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2b591f32751105e494bb297adb2c9d8abf381d77))

### Performance

- **app:** Keep the cached parts under liquid glass ([`9954cc2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9954cc28fd7364911a066b6e8b7e7e40e4be639f))
- **gpui:** An opaque window draws to an opaque layer ([`d21f8c0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d21f8c0fdff7929f17c313aacf0fbd50953173c3))

### Refactors

- **app:** Drop an unused worktree row selector ([`d336b1d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d336b1dec1d2a0d61b728d4726d8d6fb9efb1f0f))
- **core:** Drop worktree_dirs, where a worktree lives never decides ([`257d1db`](https://github.com/AleksandreJavakhishvili/pitwall/commit/257d1db43996806136c51290d81df8ea2ac835a0))

### Documentation

- Settings → accounts, usage, gemini and copilot logins ([`0a395b8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0a395b8dfbc945375bd8a2368493ba796d702a7f))
- Highlights for 0.5.0 ([`7550201`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7550201950c2599ea5c3cbd9042c06985b374c50))


## [0.4.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.4.0) - 2026-10-31

### Highlights

- Watch another machine's agent live: open one of your other Mac's agents and its terminal streams in, colours, scrollback and all, following the theme and your tile font. View only: nothing you type reaches it. Works for every agent kind (Claude Code, Codex, any terminal), since it is the terminal itself that streams. Both Macs need this version.
- The sidebar groups by machine, then account and project, then agent, once agents run on more than one machine. Each machine has a badge with how it is doing (this Mac, an agw machine, or another Mac: connected, offline · last seen…, view only), needs-you and agent counts, and folds. Another Mac's agents sit under its own accounts, by their names, with an eye on each row. The icon rail on narrow windows groups the same way. With only this Mac, the sidebar looks as before.
- Settings → Peers lists all your machines in one place, this Mac included, with Pair, Pair again and Unpair, and a Refresh button to look again.
- Fixed: Pitwall opened from the Dock couldn't reach the Tailscale CLI ("Not listening"), so it never found your other machines.

### Features

- **app:** One list of your machines in settings, with refresh ([`768437f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/768437f01e6d8117e31f64c2b09967875989b7ca))
- **core:** Name each agent's account for other machines ([`d4917a4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d4917a416343575557e760858d97361bc654b8ec))
- **app:** Group the sidebar by machine, then account and project ([`f55732e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f55732e9d0421f7e5d9bbde67114f9a79cc6c3c0))
- **core:** Watch a terminal's output without nudging it ([`64ea5e2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/64ea5e2f3140fabdde20f2c3d74748a6439a854b))
- **core:** Stream a peer agent's terminal view-only over the tailnet ([`c00e49e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c00e49ec5994dca04f2d00861f3a425133b7095b))
- **gpui:** A view-only mirror mode for terminals ([`f434ee4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f434ee4895c3678df08c4af3443599b998847142))
- **app:** Show another machine's agent's terminal live, view-only ([`d1bd31f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d1bd31fccc1769aedc889303032f01cbc117c8c6))
- **app:** A peer's terminal follows the theme and the tile font ([`df2bcc7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/df2bcc7f1166446f5c45275ab689e736385f8e8b))
- **app:** Machine badges, account initials and view-only marks ([`7e85a22`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7e85a2222d5811fbd24060f9964626ad741f99c7))

### Fixes

- **core:** Reach the tailscale cli from an app opened from the dock ([`71532bc`](https://github.com/AleksandreJavakhishvili/pitwall/commit/71532bc6c55e3e559c53449d99a551ca1adcd1f4))
- **core:** Stop a peer connection's writers as soon as it ends ([`9a18c31`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9a18c31c6f9b073a58868782fd63c1e5a7a3a4df))

### Documentation

- The sidebar groups by machine, then account and project ([`4c982bd`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4c982bd9446f66887fdbfab225c60df5d6ab49ee))
- Live peer terminals, view only ([`a0b1160`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a0b116081bc8a71de083302a075e53aada92591d))
- Machine badges, account initials and the rail by machine ([`b5102fa`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b5102faf1cac252268a23303220d532ec17ae4bb))
- Highlights for 0.4.0 ([`d1e9aa5`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d1e9aa599dcd586c42d20d2723e816d4f6d37ca3))


## [0.3.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.3.0) - 2026-10-30

### Highlights

- Run Claude Code and Codex on more than one login (say work and personal): add accounts in Settings → Agents → Accounts, which shows who each is logged in as. Pick one in New agent, set one per project from its right-click → Accounts…, or move a running agent to another account: its conversation comes along and it resumes there. Settings → General → Group agents by → Account groups the sidebar by account.
- Git identities per project: the name and email on commits, a GitHub login (gh, also used for HTTPS pushes), Azure DevOps too, and an optional SSH key, set up in Settings → Agents → Accounts and picked in a project's Accounts…. Assigning one restarts the project's agents one at a time so they take it; busy ones wait until they finish.
- Arrange the sidebar by dragging: accounts, projects, and agents within a project keep the order you give them, and ⌘1–9 follow it. An agent that needs you turns amber where it is instead of jumping to the top. Grouped by account, drop an agent or a project on another account to move it there: its agents restart on that login at once, conversations included.
- See your other machines' agents: with Tailscale, Pitwall finds your own other computers running Pitwall and offers to show their agents in the sidebar under the machine's name, with live status and needs-you notifications. Read-only, and the other machine has to allow it first: Settings → Peers, or `pitwall peers`.
- Preview a dev server beside its terminal: when an agent prints a local address, Preview in the pane header opens the page next to the terminal (on macOS, Windows and Linux), following the server as it restarts or changes port. Markdown files show as formatted text in Review and the file viewer.
- The Changes list shows staged and unstaged changes apart, like VS Code's.
- Review compares an agent's branch with the branch it was created from (read from git's own records), not with a local `main` that may be months old: a branch made from `develop` shows only its own work. Of a branch and its `origin/` copy, the one that has moved on further is used; with no record, the main branch. The header says "Compared with <branch>", and its menu chooses another local or origin branch (remembered for that repository and branch) or Automatic. `pitwall review changes --base <branch>` does the same once.
- See what rules each agent gets: Settings → Rules & skills → Project defaults → Agents shows a project's default set, each agent's own set, the rules they add up to for that agent's kind, its files, and whether they are applied. `pitwall rules show <project>` (or `--agent <agent>`) prints the same.
- Gemini CLI and Aider get rules too, though rulesync has no target for them: Pitwall writes the rules into `GEMINI.md` (or a file of its own when the project has one) and `.aider.pitwall-rules.md` (passed with `--read`), kept out of git like the other generated files.
- The Race Engineer (and scripts) can manage rules from the CLI: `pitwall rules set create|add|remove|rename|delete` and `pitwall rules import project|file|git`. Deleting a set and importing a file or a repository ask you first.
- Skills come first: the library holds rulesync skills (`~/.pitwall/rules/skills/<name>/`) next to rules, and Settings (now Rules & skills) lists them first, with a New skill form. Assign skills to a project like rules (copied into `.rulesync/skills/`, with updates and diffs per file); a project with only skills keeps its own CLAUDE.md / AGENTS.md, as Pitwall then generates skills only. Agents can have personal skills too, kept out of git. `pitwall rules skill new` and `pitwall rules new` write new ones (replacing one asks), and importing a project, or a git repository with only `.claude/skills`, also brings in those skills. In New agent, the sets are grouped into rule sets and skills.
- Rules now end up in the project: assign a set or single rules from the library and Pitwall copies them into `.rulesync/rules/` (recording where each came from in `.rulesync/.pitwall.json`) and generates CLAUDE.md, AGENTS.md… for you to commit. Library updates are opt-in, with a diff; rules changed in the project are never overwritten, and conflicts need your choice. A hand-written CLAUDE.md is imported into `.rulesync/` first, and the old project default set can be moved into the project. Settings → Rules & skills → a project's Details, or `pitwall rules project …`.
- Agents take their name from their conversation: Claude Code's own title (yours from `/rename`, else the one it generates) or Codex's thread name, else the first prompt, read from the agent's own session files when a prompt is sent or a turn ends. A name you give (in New agent, or by renaming) is kept; right-click → Use session title, or `pitwall agent rename <agent> --auto`, follows the title again. Agents named before keep their names.
- Rename an agent in the app: right-click it in the sidebar → Rename…, or double-click its row. Enter saves, Esc cancels, and a bad name says why in place. Only the name changes: its worktree, branch and conversation stay (`pitwall agent rename <agent> <name>` does the same).
- After rules change, Settings → Rules & skills offers "Re-apply & restart all" for every affected agent, next to each pane's own button; `pitwall rules apply --stale [--project <p>] [--restart]` does the same from a terminal.

### Features

- **app:** Drop a space tab onto another window to move it there ([`42167b9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/42167b983812b4b69079add72bce1bf994c18778))
- **app:** File links outside the agent's folder open in the viewer ([`03187bf`](https://github.com/AleksandreJavakhishvili/pitwall/commit/03187bf08b1d1a3e3cb5e1afbe99544dd97d00f2))
- **core:** Changes in repositories inside the agent's folder ([`0b8734b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0b8734b2ffa6d9a03b08c618c749dab2c0ab93cc))
- **app:** A searchable project picker in New agent, More options, ⌘↵ ([`766a0f8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/766a0f84e677ec313065d5f324de1fa4517aa305))
- **core:** Repositories linked into the agent's folder count too ([`7b8edb2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7b8edb2222d665f682fb400cc017bb88e9ac8cf1))
- **app:** Accounts for agents; staged and unstaged changes ([`e11c903`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e11c903bfb815d3226185135d2ad1f92cc923ffe))
- **app:** Group the sidebar by account; projects move their agents ([`5427b8f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5427b8ffd7a2d7c30ff369bb3d00bb9df7117761))
- **ui:** The web demo shows accounts, staged changes and the new dialog ([`8b666c2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8b666c2a1d4155b2f4b169c6315e21df7e4307b9))
- **ui:** Show agent logos next to each agent kind ([`68964f5`](https://github.com/AleksandreJavakhishvili/pitwall/commit/68964f520aa301bc6e6b2731071a7bb43b22a28f))
- **core:** Keep agent tools up to date from pitwall, one at a time ([`f0e3c9f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f0e3c9ff10677958b47f2fe296cd0a04e23ee960))
- **core:** Git identities per project for agents and terminals ([`dd5df49`](https://github.com/AleksandreJavakhishvili/pitwall/commit/dd5df4934bd981afed4fa0f6a499c48c0b35b5c6))
- **app:** Bench opens a settings page or a project's accounts ([`8eaea3d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8eaea3dfa0472d265cebf887910b23ae02384775))
- **core:** Git identities log in to azure devops too ([`4e5ee4f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4e5ee4ff3cfc759d7b3dfdad0fd370a516fea504))
- **app:** Assigning a git identity restarts the project's agents ([`dc28cd7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/dc28cd70bf748b5328ebb8f07a580764b12d74cd))
- **core:** Show a project's and an agent's effective rules ([`e36a232`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e36a232593149b6aee1ab5bdd8cf3a9036ac2767))
- **core:** Rules for Gemini CLI and Aider without rulesync ([`56f74f0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/56f74f02cbc755b467714edeeab96e3ded774452))
- **cli:** Manage rule sets and import rules from the CLI ([`33178e5`](https://github.com/AleksandreJavakhishvili/pitwall/commit/33178e536f65dd2fd47087780d5a5b9c184724f7))
- **app:** Re-apply and restart every agent whose rules changed ([`8bbdc0a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8bbdc0a57eb1c1e58a05e140b125922afaa64d30))
- **core:** Project rules copied into the repository's .rulesync ([`613f9ac`](https://github.com/AleksandreJavakhishvili/pitwall/commit/613f9accebbbbe349edc490ec81b87040a44ed6d))
- **cli:** Project rules on the socket and the command line ([`a80196d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a80196d8e4d03fc4ca39893ed5bc8988730ac3f8))
- **app:** Project rules in Settings → Rules ([`084c036`](https://github.com/AleksandreJavakhishvili/pitwall/commit/084c03656578b171d1daf12b624739edeafbeff4))
- **core:** Skills in the library, in projects and for agents ([`eae2f92`](https://github.com/AleksandreJavakhishvili/pitwall/commit/eae2f9236c0f43a2e765a9c2f03151c2d480942c))
- **cli:** Skills on the command line ([`9996df8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9996df80bf5e3ce53acd5d5000e46c775a0fcaec))
- **app:** Skills first in Settings → Rules ([`299cece`](https://github.com/AleksandreJavakhishvili/pitwall/commit/299cece2ff2fc5c7bf1ec427da429f8c3b5033a2))
- **core:** Import a git repository's bare .claude/skills ([`f69085f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f69085f7f49fe8016da767ea0f7b1b96c3bb70f8))
- **core:** New library entries from text ([`b01ec8c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b01ec8cf154a233dbac33aaa79597707077f3a75))
- **app:** New skills from Settings, grouped sets in New agent ([`2bc21bf`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2bc21bf15c86edd23aa15764983a87dd405adbd4))
- **app:** The Settings page is Rules & skills ([`4236fe2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4236fe24eb3e85150c35a0d4686ac0858c12e910))
- **core:** Check an agent's name before renaming it ([`56b9c7e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/56b9c7ed74f403402b86938ee53784f485a37210))
- **app:** Rename an agent from the sidebar ([`a82a230`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a82a230621437a54aa30782f7019c3d17833c6fd))
- **core:** Read a conversation's title from the agent's own files ([`4af6269`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4af6269169ffe48140c0e93a37038b9dd79d373a))
- **core:** Name agents after their conversation's title ([`2889288`](https://github.com/AleksandreJavakhishvili/pitwall/commit/288928824930ac88ed4ac1ab89a4d4f4837160b5))
- **daemon:** Let agent.rename follow the session title again ([`6c927e8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6c927e89748dd58bdaad4075fb8518c8069ee3c1))
- **cli:** Add pitwall agent rename --auto ([`902b762`](https://github.com/AleksandreJavakhishvili/pitwall/commit/902b762348a39ca2a53043dfddd859dd3d41a367))
- **app:** Use an agent's session title from the sidebar ([`24e03e8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/24e03e8ceff2193b0e3402395d0bea0c6aad9f04))
- **app:** Review shows what it compares with and lets you choose ([`2658c61`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2658c61bf3735b6f0fc65b081d06f71c97ec5fb7))
- **daemon:** Review.changes says what it compares with ([`9d25278`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9d252780140cc9e6ec3ad82e89759659b7eef620))
- **cli:** Review changes --base compares with another branch ([`9c01588`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9c01588bd983a10b09fb7f50ca227cc9cf0ac73e))
- **app:** Review lists the focused agent alone ([`d771187`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d771187e2a8e63e16a8e7b0565fd8550ec6b7ab8))
- **core:** Agents report local dev-server addresses they print ([`18eda7e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/18eda7e1126e0f3f6f4521738629e686de4aa160))
- **app:** Pane header opens an agent's dev server in the browser ([`e2cb1c6`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e2cb1c61554abe2a0b9dcc8ab841492f22f7c5a7))
- **daemon:** Send agent events to subscribed socket clients ([`06f49a4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/06f49a48ae64d9e58da951ede0c558128cafe422))
- **cli:** Follow agents with pitwall events ([`b111e55`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b111e5506ec912e7989dd733f73115f64add3a14))
- **app:** Send the engine's events over the cli socket ([`04af321`](https://github.com/AleksandreJavakhishvili/pitwall/commit/04af321557835afdc0e8f87a742f767866ff48cf))
- **app:** Preview markdown files in review and the file viewer ([`78dfd46`](https://github.com/AleksandreJavakhishvili/pitwall/commit/78dfd46e7f12efa2554c96edb69d185882ea1c48))
- **daemon:** Add peer wire shapes and the methods peers may call ([`eb02a80`](https://github.com/AleksandreJavakhishvili/pitwall/commit/eb02a809440570b02f2df479d7c43b668e791e0e))
- **core:** See the agents of your other machines over tailscale ([`5a60236`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5a60236b7e018edd58ca81c447fdf3725575ac1a))
- **cli:** Add pitwall peers to pair with your other machines ([`36decb8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/36decb8178846323b79e7260571a9ed7b19ed612))
- **app:** Liquid glass laid out as macOS does it ([`57d207b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/57d207bfe3d7726a6ca73814c984d840dfcb423f))
- **app:** An agent's name is optional in New agent ([`2a2f611`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2a2f61122e43ddee9467c3db9c9af6a17b43bcee))
- **app:** A web view interface for the in-app preview ([`f0ffc1f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f0ffc1fec05284d1da14bfb27ff12e34b1530606))
- **core:** Count each time a terminal prints its dev-server address ([`764b01f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/764b01f58b870b39472b0feebaa0f2405b00816c))
- **gpui:** Let native views follow each drawn frame ([`c484c5a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c484c5a3c38d593c3029ff790b7977ca8c8622df))
- **app:** Open a pane's dev server beside its terminal ([`3bd69d1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/3bd69d150f0b833b2d159527dc2e175aaf3d27dd))
- **app:** Show the in-app preview with WKWebView on macOS ([`95d5b24`](https://github.com/AleksandreJavakhishvili/pitwall/commit/95d5b2487820b1333ae91a7b22abb9ef53b1c08c))
- **app:** The in-app preview on Windows, with WebView2 ([`a66ee4d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a66ee4d4ceef4c0b6c477077ce84c135ca1b3ad4))
- **app:** The in-app preview on Linux, with WebKitGTK ([`4d0ba8f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4d0ba8f827f92ea03ce90227e3a71d21b5cd368a))
- **app:** Put the in-app preview on the terminal's left too ([`ffd94d0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ffd94d05cc675f86632ec044e8326511616c94b8))
- **core:** Peers reload what another process wrote to peers/ ([`90299a6`](https://github.com/AleksandreJavakhishvili/pitwall/commit/90299a63baac9165eecf8a9675a561f6d11f54e6))
- **core:** A peers page in settings ([`34fe62d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/34fe62db3e3f5a16ad6fbdd9c801e1fc9feee878))
- **app:** Host tailnet peers and a settings page for them ([`f4bdacb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f4bdacbf33f6d50f452ca53a1aa77031f4349680))
- **app:** Other machines' agents in the sidebar, read-only ([`70cbd95`](https://github.com/AleksandreJavakhishvili/pitwall/commit/70cbd95265bafa074b858c8af63163ca2cac3238))
- **app:** Drag accounts, projects and agents in the sidebar ([`b751918`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b751918c544306945c88ba1854ed3674e722f1ee))

### Fixes

- **app:** "Choose folder…" opens the folder picker and keeps the dialog ([`09e8eeb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/09e8eeb68c0487ab8435483cf4eff92762dd4755))
- **detect:** Read your own rules from pitwall's data folder ([`ff96b0e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ff96b0e64e982b403eba69b0bf1e5c667f715d42))
- **app:** Long account emails end in an ellipsis, not under buttons ([`398994f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/398994fc0c40c2f0dd11f03c8f839fb880a4aeae))
- **ui:** Terminals and empty projects sit under their project's account ([`d072d7d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d072d7dfeeaeca86853414f7ea5c18cc70f1f6f1))
- **core:** Review compares with the agent's branch, not its start ([`3fca1b1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/3fca1b1323ba978884d984d6a9dab5143de70d94))
- **app:** Run terminals from a holder copy updates never replace ([`d12647a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d12647ae8a13e27db1b45517a08af65e1b1befcf))
- **core:** Git config shows a git identity's name and email ([`d180e82`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d180e82f1075fcfc64a581e0b80892cf2d0bd3fd))
- **app:** Restart a project's agents one at a time on a new git identity ([`df58cdc`](https://github.com/AleksandreJavakhishvili/pitwall/commit/df58cdc1962cda3675a6e8417e3240c16b1b4785))
- **app:** No rulesync warning for agents Pitwall writes rules for ([`f3d58c7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f3d58c784e505f8eeabcefbac98a8511e4e7cb52))
- **core:** Imported CLAUDE.md and AGENTS.md stay apart when generated ([`b4765a2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b4765a2c94bfbdf1f222c2821d72e0a60481df53))
- **core:** List generated rule files in a fixed order ([`9e66fc7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9e66fc72d0b458eb25d9f0018e26e5d080f2ddb2))
- **core:** Personal skills' scripts stay executable in agents' folders ([`de70974`](https://github.com/AleksandreJavakhishvili/pitwall/commit/de709742dc3f26b2f5cd45e4b5d97b9f16d67976))
- **core:** Review compares a branch with the one it was created from ([`c1f2f12`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c1f2f12c88f935b973d8a44d090ee02d46d1109b))
- **app:** Wrap text in a squeezed box without looping ([`89c70b9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/89c70b9b7030f3113d31b849de315e6e25477d51))
- **app:** Scroll the whole markdown preview and select its text ([`c326c43`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c326c437b5195c80235e3fe0d19333585d4f27b4))
- **app:** Draw html in the markdown preview as formatting ([`88543c9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/88543c91b6c63751e784f29b3fbf1131704f5fcb))
- **app:** Markdown preview fills the pane, wide code scrolls ([`7d64f0a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7d64f0a960bf0720c5f46ad2061e2de31dcf6c04))
- **app:** Agents named from their folder follow the session title ([`c6c11d8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c6c11d820248a119bc0a262cbbfcb5e58169e32c))
- **app:** Light glass reads as frosted white, not grey ([`0f7ed19`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0f7ed19e5215556916b3e179758c1ce0530afa4f))
- **gpui:** Give X11 windows a raw window handle ([`9eeb4ae`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9eeb4ae8328c6383018bc95a48452bc45ca37f94))
- **app:** Hand keys back on Windows and Linux as on macOS ([`e65bdb8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e65bdb8515c4d315553c38aefd74cc522f894fbb))
- **app:** Keep the preview's web inspector out of the pane on macOS ([`7068c19`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7068c194e7a55aaf73b79a7459e0db99c9152f54))
- **app:** Read peers off the ui thread ([`03cd37c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/03cd37c6722472505b918dc1f307d4f040094585))

### Performance

- **core:** Run git on its own only for agents whose changes are shown ([`97bf3e1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/97bf3e18ccd16ce89cd5ceb566e439e14fb08b56))

### Refactors

- **core:** Name compare_with as what review compares with ([`f302573`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f302573ead118926206e7cc3f7015ef7ecc52aef))

### Documentation

- Accounts, linked repositories and staged changes ([`ed4b17d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ed4b17d57624157ef4c602bfff06f03137067e80))
- A docs section with cli, settings, agent and detection references ([`f72c12b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f72c12bdbf91d1d93ceb4c37cf5ab0831c78e38c))
- Project rules in the spec, skill, Race Engineer and changelog ([`8f810d4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8f810d4f79846061ffd6e8ef7af37dc7002d0991))
- Skills in the rules spec, skill, Race Engineer and changelog ([`e71f560`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e71f5607d73459f1ea4c8c046336e461d7191519))
- Rules & skills page, new skill form, git skills import ([`cc91a01`](https://github.com/AleksandreJavakhishvili/pitwall/commit/cc91a01e9c6f875a5982b5b13b21860fb40530c0))
- Renaming agents in the app ([`00bbe8b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/00bbe8b4bf7caa270ce949c8a6e30cbc6ffd957f))
- Agents named after their session title ([`cd70b29`](https://github.com/AleksandreJavakhishvili/pitwall/commit/cd70b29e9d1fafc2dd396f5ec2763fc372d993c4))
- What review compares with ([`fc86cf4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/fc86cf416c000fd9742e7376710a3b2660fff6f2))
- Open preview chip for printed dev-server addresses ([`391c6f2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/391c6f2d3706859975f80e6b0b71247c450c8303))
- Socket events in the api, architecture and peers specs ([`0fee50e`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0fee50e185782f481e2d58ff0525a332bdb5a6c4))
- Selecting and copying the markdown preview, its html ([`29fdfd3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/29fdfd3668bf7ec89328c99967a30c826f379a94))
- Mark the tailnet peers phase 1 core as built ([`42ccd09`](https://github.com/AleksandreJavakhishvili/pitwall/commit/42ccd09618ed73704acfe336f796f7b6e36b118d))
- Describe the in-app preview ([`c95453c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c95453c3c78889d62770e39688d3b21938e198e9))
- Tailnet peers in the app as built ([`4e4f18f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/4e4f18f1ba667da09035488b4136f535ef69f204))
- The running app applies pitwall peers changes at once ([`5cb48d1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5cb48d15de7f833362d212d53f393d27f7c05d14))
- Highlights for 0.3.0 ([`1601fc4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/1601fc4758872743570dcc5c77756f3e7f0a17b6))

### Tests

- **daemon:** Drop the time bound from the slow-client events test ([`54843a3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/54843a34c4811008836986064fa4136e7a93c1bb))
- **app:** Copy from the explorer's markdown preview ([`da4fb51`](https://github.com/AleksandreJavakhishvili/pitwall/commit/da4fb5137a022ab67f6ea07f96c110c056b34a60))
- **app:** A laptop shows a studio's agents live on a fake tailnet ([`b27dd3f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b27dd3f5a3bddc0d615fd33fee716878813dd9a2))
- **app:** Settings -> peers in a test window ([`03fd166`](https://github.com/AleksandreJavakhishvili/pitwall/commit/03fd166f8cb50abac0dee2040fb6a1c147e21a2a))
- **app:** The host makes peers with both switches off ([`8b6f559`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8b6f5592459db277119863631d2e1ac09adb23df))

### CI/Build

- **deps:** Add pulldown-cmark for the markdown preview ([`c8ef7e9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c8ef7e9b55ef5fc4bae578805103ba7ad4661dc9))
- **deps:** Add WebKit bindings and image for the in-app preview ([`001dcdc`](https://github.com/AleksandreJavakhishvili/pitwall/commit/001dcdcb101e0e8e9cbd9a79e9117b84152aaa8c))
- **deps:** Add webview2-com for the Windows preview ([`9bcbd79`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9bcbd793c6973d4767a0192a56ba817ba056c7ac))


## [0.2.1](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.2.1) - 2026-10-10

### Highlights

- Drag a space tab out of the tab bar to open it in its own window, where you let go.
- With more than one window, ⌘P (quick open) and Review open in the window you're in; before, the file could open in another window and leave this one blank. ⌘P also works again after Esc closes a file.
- Liquid Glass: no more flicker of the whole window while agents work or a diff opens, and the floating sidebar and right panel have clean rounded corners.
- With Reduce motion on, dialogs fade in instead of flashing dark, and a click outside a dialog only closes it (it no longer reopens what was under it).
- The website's favicon is the Apex P, like the app icon.

### Features

- **app:** Drag a space tab out to open it in its own window ([`461ce70`](https://github.com/AleksandreJavakhishvili/pitwall/commit/461ce7099cf893c3c711ad794b236bcca47e0bca))

### Fixes

- **website:** The favicon is the Apex P like the app icon ([`7cedd0f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7cedd0f6f9071e937fac02489ced51f95d7ce2b8))
- **app:** Glass panels' background follows their rounded corners ([`a32087c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a32087cb4d6f3f937beab2a36db28d8b1440fbf5))
- **app:** Quick open shows the file in its own window ([`c8d7b59`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c8d7b59849ac631477ec35d5ed1cd54999159f6a))
- **app:** Glass pieces stay put while the screen replays a frame ([`8ad8025`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8ad8025bf8623bf938b688c5f6adeebb144d4234))
- **app:** Dialogs fade in with Reduce motion instead of snapping ([`dd3512b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/dd3512b766002c07843c23efdb0b0a4d1175792e))
- **app:** A dialog's backdrop takes the click instead of passing it on ([`257184a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/257184ae7395a0004abdba9ecfb6930850d30c14))

### Documentation

- Highlights for 0.2.1 ([`5ff36cd`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5ff36cdd4205290649d5fbb32823c8cc8fa01046))

### Tests

- **app:** Quick open tests use the shortcut of the OS they run on ([`e6bbbd3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/e6bbbd3ca26e22e9ea8c715ac9dcae10db7e271c))
- **app:** Tests see default OS accessibility options ([`aa16cd9`](https://github.com/AleksandreJavakhishvili/pitwall/commit/aa16cd9ec41c0fff6d1de42e0396b7b0023069f7))


## [0.2.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.2.0) - 2026-10-09

### Highlights

- Pitwall is now a native app. The web-view (Tauri) app is replaced by one drawn with GPUI in Rust: faster, lighter on memory, with native menus and keyboard handling on macOS, Windows and Linux.
- Same design and same data: it opens your existing agents, spaces, windows and settings from `~/.pitwall`, and agents left running by the previous version are picked up where they are. Install it over the old one: it keeps the name Pitwall and the same app identity.
- The Glass look uses the system material: Liquid Glass on recent macOS, vibrancy on older macOS, Mica on Windows 11 and a lighter painted glass elsewhere; Flat stays one click away.
- A new logo and app icon: the Apex P.
- A fraction of the CPU and less memory than the web-view app with the same agents.
- The Race Engineer: an optional agent of your choice (Claude Code, Codex, Gemini CLI, …) that knows Pitwall's CLI and agw, one click from the top bar or ⌘K. Anything risky waits for your approval.
- Settings is a sidebar of pages, and every setting can also be read and changed with `pitwall settings` or in `ui.json` (schema in `docs/settings.schema.json`).
- Opening never waits for a slow or unreachable machine: its agents connect in the background and retry.
- Full Disk Access is optional: per-folder access is the default.
- ⌘-click a file path in a terminal to open it; long pastes arrive whole, also after the app restarts.

### Features

- **website:** Glass look with a night-race sky ([`1ae34b4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/1ae34b41b250b621d6d4bfd9044566162315a946))
- **ui:** A Glass appearance with native window vibrancy ([`da2195c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/da2195cfe97b3e06bcabc527495892335b777dc0))
- **app:** Apex p app icon with a chequered finish ([`50a191d`](https://github.com/AleksandreJavakhishvili/pitwall/commit/50a191d2da0abe1abcd900924f98834397049b72))
- **ui:** Apex p in the top bar wordmark ([`ecfbff1`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ecfbff11296c1ca27d6192bc4f6c5332b6126003))
- **website:** Apex p logo and racing motion ([`ff19566`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ff19566adddfe50a6d3d473b3f5f114f1ffe404f))
- **ui:** Demo bridge follows the site's theme ([`2786559`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2786559732862b72e8ce0e11905b7ec1ded6a155))
- **website:** The live app is the landing page's only demo ([`43e51f2`](https://github.com/AleksandreJavakhishvili/pitwall/commit/43e51f2d8a3929e1374a8b3dc9716e2d253e5f36))
- **core:** Engine for the native app ([`2f5d1b6`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2f5d1b67d49e3188cda316d652650b4a9d334385))
- **app:** Native GPUI app replaces the Tauri shell ([`101f2ce`](https://github.com/AleksandreJavakhishvili/pitwall/commit/101f2ce979960a23a61f41db4b9e09d59d6882ea))
- **ui:** Web demo keeps the native app's look ([`367e04b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/367e04b5ce9d60ac14efafe28ad2aebcbe6955cf))

### Fixes

- **website:** Keep text from showing through the header bar ([`7998616`](https://github.com/AleksandreJavakhishvili/pitwall/commit/799861685124026f5f765fc0935df7bc9b54da0e))
- **website:** Neutral bezel around the app demo ([`8e57749`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8e577491ad296c6a8e0d6ee9b482c912662ff4f9))
- **ui:** Readable layout preset icons in Glass ([`339cf37`](https://github.com/AleksandreJavakhishvili/pitwall/commit/339cf3756e1a355a0418f117d949331bd6793e8c))

### Performance

- **app:** Keep GPU memory of windows out of sight ([`baafe33`](https://github.com/AleksandreJavakhishvili/pitwall/commit/baafe335532668f8ee8ce404287a915972fb0e9a))

### Documentation

- The native app in the readme, roadmap and specs ([`60b3597`](https://github.com/AleksandreJavakhishvili/pitwall/commit/60b3597dd5289fb1b9bf9cf940f2217401b418bf))
- Highlights and roadmap for 0.2.0 ([`68ce8cb`](https://github.com/AleksandreJavakhishvili/pitwall/commit/68ce8cbf2301844157974fff3c3c9c1ffb23d1dd))

### CI/Build

- **deps:** Vendor gpui 0.2.2 with a Metal memory patch ([`9f95234`](https://github.com/AleksandreJavakhishvili/pitwall/commit/9f95234d1f70d14b55745763d3adec1c9dfa8450))
- **release:** Package and release the native app ([`89114ab`](https://github.com/AleksandreJavakhishvili/pitwall/commit/89114ab9555324c4a904d2f4ad1d24cf6223665b))


## [0.1.1](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.1.1) - 2026-10-08

### Highlights

- Windows x64 builds (preview): an installer (`-setup.exe`, per user) and an `.msi`. Not code-signed yet, so SmartScreen asks on first run (More info → Run anyway).
- Fixed on Windows: terminals hung on start (the holder inherited its launcher's handles) and showed no output (ConPTY's cursor-position request went unanswered).

### Features

- **website:** Canonical, link previews, sitemap and robots ([`0bbc12f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0bbc12fa7470eaf336fb2ceb12c08f2331914571))

### Fixes

- **release:** Create the sidecar folder before lipo on macOS ([`5a94436`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5a944367d33017410f74f627ebd00c2f899a4714))
- **hold:** Stop the Windows holder from hanging its launcher ([`b72941a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b72941a5ea9aa931b9709903874430cbcb03beab))
- **daemon:** Keep the stop poke connected until the server takes it ([`c7e2371`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c7e237174405681585561f868f014128a9cef758))

### Documentation

- **website:** Say Windows is coming, not shipped ([`f24bbbe`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f24bbbeb6fe090c4e1fb30c7ab3802c78cb28984))
- Offer the Windows builds as a preview ([`70c394c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/70c394c9fbae3659338d1fdbb4e786e390d28925))

### Tests

- **hold:** Give Windows PowerShell time to start in holder tests ([`c4633dd`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c4633ddf7042f568d8d4ff2fb32c8b6f23ba1d25))
- **core:** Wait for the branch in the fake-agent worktree test ([`f7c3a7f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f7c3a7f36eac3d01590c28054ed8cdf7795af7bc))
- **core:** Wait for exec in the Linux process-table test ([`80c79e7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/80c79e79a5c230df019a74e55b7aa59f27d0e2cf))

### CI/Build

- Require the daemon tests on Windows ([`d173022`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d173022bef2b88339ab8b313cd2dcb0e7fe0b02e))


## [0.1.0](https://github.com/AleksandreJavakhishvili/pitwall/releases/tag/v0.1.0) - 2026-10-08

### Highlights

Pitwall's first public release. These highlights cover the work from before the
commit convention, which the entries generated from commits don't include.

#### Hosting and status

- Terminal agents in tiled spaces with status flags: needs you, done, working, idle, exited, stopped.
- Status from hooks, then screen rules, then activity. Screen rules for Claude Code, Codex, Gemini CLI, opencode, Cursor Agent, GitHub Copilot CLI, Qwen Code, Amp and Aider.
- Needs-you bar with <kbd>⌘J</kbd>, Dock badge, native notifications.
- Per-agent Next up queue with auto-send.
- Terminals live in `pitwall-hold` processes and survive <kbd>⌘Q</kbd>.

#### Screens

- Wall (<kbd>⌘E</kbd>): every terminal, view-only, grouped by project.
- Review (<kbd>⌘R</kbd>): per-task diffs, line comments sent as one prompt, commit & merge.
- First-launch read-only scan that resumes recent conversations.
- Density setting (Comfortable / Compact / Dense), presets up to 4×4 and Auto grid, spaces in separate windows.

#### Setup

- Rules via `rulesync`: library, rule sets, per-project and per-agent assignment.
- Worktrees through the agent's own `--worktree` flag; Pitwall creates no worktrees or branches of its own.

#### Terminals and machines

- <kbd>⌘T</kbd> opens a terminal in the focused agent's folder; an agent started in it is recognised, and Restart resumes it.
- Agents running in other terminal apps are listed under Elsewhere, with "Bring in".
- agw sessions on VMs: add or create them, live terminal, status, Next up, diffs and Review.
- `pitwall` CLI to list and add agents and sessions; changes to another machine wait for approval in the app.

#### Code and changes

- Read-only file explorer per agent: Files tab, full viewer, <kbd>⌘P</kbd> quick open and <kbd>⇧⌘F</kbd> search, on this computer or an agw machine.
- The Changes panel matches `git status` and shows the branch and folder; Review follows the current project.

#### Platforms and performance

- macOS (universal) and Linux (AppImage, .deb). Builds aren't code-signed yet. Windows follows in a later release.
- Git refreshes when files change instead of polling; Review's diff editor and the Wall are lighter on memory and CPU.

### Features

- **ui:** Refresh source control on open, with a refresh button ([`8201cf4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/8201cf4ffe32e57eb205eaab0fcfbe92e73b9631))
- **ui:** Show the branch and folder in the Changes panel ([`040cd0b`](https://github.com/AleksandreJavakhishvili/pitwall/commit/040cd0be2b4c1d977e80ec58d5738855b4ac2ff6))
- **core:** List, read and search an agent's files ([`c3927d5`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c3927d5c5ab77f218aa92b22aabd343983f9d8c9))
- **app:** Explorer commands and a typed TS API ([`28b679a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/28b679a5f1f1bfc59fba202abbaacb6e76aa2459))
- **ui:** Scope Review to the current space ([`7279173`](https://github.com/AleksandreJavakhishvili/pitwall/commit/72791733b78e304ed50f1f3669080f664bdb465c))
- **ui:** Browse an agent's files read-only ([`a9e76e3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a9e76e30ecc5185aa92904a3a552c31efa68197e))
- **website:** Download options for macOS, Windows and Linux ([`a52902a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a52902ac943cdab2778f726fd1eb52c2149027fa))

### Fixes

- **app:** Spawn tools with the login shell PATH ([`a62cc03`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a62cc03a0aead8c4ddbb805610860858f08eb3b4))
- **core:** Match git status in the Changes panel ([`1bb98e4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/1bb98e40a4c9ac1745e43016905ef368c2b5d63b))
- **ui:** Scope Review to the focused agent's project in All ([`b4d5be4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b4d5be4348f203db241ae0281f9c5f1ac9fccde8))
- **core:** Show non-ASCII file names instead of octal escapes ([`40b0428`](https://github.com/AleksandreJavakhishvili/pitwall/commit/40b0428a6fdaad9ff061ebee0ac18819ac9b6cfa))

### Performance

- **core:** Refresh git on file changes instead of polling ([`89c2869`](https://github.com/AleksandreJavakhishvili/pitwall/commit/89c286945a79203f2cb235206df16c571be69c44))
- **detect:** Parse terminals with alacritty_terminal ([`0ffeb44`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0ffeb44af70bc24d5e5ddc83db01c86e516adc22))
- **ui:** Draw Wall tiles from the backend's screen copy ([`7d02bd7`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7d02bd7b05dc742b2cb8567d340927d5d4987c6f))
- **ui:** Render Review diffs with CodeMirror instead of Monaco ([`7a6abb0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/7a6abb032a5fec841009be78fd56feff3e31c2cc))

### Refactors

- **core:** Drop open-in-editor; list ignored, read large files ([`75220e0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/75220e023e316dd98b25c3be4aaaa75c6dea4272))

### Documentation

- Roadmap marks perf pass 1 done and lists pass 2 ([`ad4b7d8`](https://github.com/AleksandreJavakhishvili/pitwall/commit/ad4b7d8dbe838e80da58da5bc6d1ff8592ae66d4))
- Add a public roadmap ([`5f15567`](https://github.com/AleksandreJavakhishvili/pitwall/commit/5f15567c4f0a8cbe5927cbde1372923fc20c92bf))
- Drop the CLI expansion from the public roadmap ([`973df8a`](https://github.com/AleksandreJavakhishvili/pitwall/commit/973df8a41e6f3549cc970cb714a3503ac0f5ba71))
- Describe and measure file-change git refresh ([`c37e7ee`](https://github.com/AleksandreJavakhishvili/pitwall/commit/c37e7ee6b28f88f5d9cde67bea1f85ce11cdb53a))
- Record Review memory results for perf pass 2 ([`d7ad2b3`](https://github.com/AleksandreJavakhishvili/pitwall/commit/d7ad2b360f1c7a19f44d104af5ef76db15482c87))
- Spec a read-only code explorer ([`f46d54f`](https://github.com/AleksandreJavakhishvili/pitwall/commit/f46d54fc5563a3415e8bea7cc64c5107e476baf1))
- Roadmap and changelog highlights for 0.1.0 ([`b9aed04`](https://github.com/AleksandreJavakhishvili/pitwall/commit/b9aed04e1ec3550d5cc80e79ca99eefb2b4602c7))
- Install instructions per platform in the README ([`47cf7bf`](https://github.com/AleksandreJavakhishvili/pitwall/commit/47cf7bfb7935fd8cbf7db3cb279781a79603ae3f))

### CI/Build

- Adopt conventional commits with a commit-msg hook and git-cliff ([`70d0612`](https://github.com/AleksandreJavakhishvili/pitwall/commit/70d0612219bf207aff0d0b3f4584ee7644197c93))
- **website:** Render the changelog and roadmap from Markdown ([`6c8a063`](https://github.com/AleksandreJavakhishvili/pitwall/commit/6c8a063a10a2f8b7375ab9490996fd187768bb63))
- **release:** Ship macOS and Linux; Windows behind RELEASE_WINDOWS ([`2b2c0a0`](https://github.com/AleksandreJavakhishvili/pitwall/commit/2b2c0a00b5dd1442a3c54b40731d14e9e67a4290))

### Chores

- Add a release script that prepends to CHANGELOG.md ([`a6a680c`](https://github.com/AleksandreJavakhishvili/pitwall/commit/a6a680c02691ac8e34ae82cf61e20ef3c62a0c67))
- Trim trailing blank lines in the release script ([`0a115d4`](https://github.com/AleksandreJavakhishvili/pitwall/commit/0a115d444549bcc817621f3d92e254e4f48f8044))
