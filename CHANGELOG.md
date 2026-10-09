# Changelog

All notable changes to Pitwall. Generated with [git-cliff](https://git-cliff.org) from [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/); see [CONTRIBUTING.md](CONTRIBUTING.md#commit-messages).

## Unreleased

### Highlights

- Pitwall is now a native app. The web-view (Tauri) app is replaced by one drawn with GPUI in Rust: faster, lighter on memory, with native menus and keyboard handling on macOS, Windows and Linux.
- Same design and same data: it opens your existing agents, spaces, windows and settings from `~/.pitwall`, and agents left running by the previous version are picked up where they are. Install it over the old one: it keeps the name Pitwall and the same app identity.
- The Glass look uses the system material: Liquid Glass on recent macOS, vibrancy on older macOS, Mica on Windows 11 and a lighter painted glass elsewhere; Flat stays one click away.
- A new logo and app icon: the Apex P.
- A fraction of the CPU and about half the memory of the web-view app with the same agents; a window you can't see (minimised, in the tray, on another Space) gives its GPU memory back until you look at it again.

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
