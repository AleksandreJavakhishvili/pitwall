# Changelog

All notable changes to Pitwall. Generated with [git-cliff](https://git-cliff.org) from [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/); see [CONTRIBUTING.md](CONTRIBUTING.md#commit-messages).

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
