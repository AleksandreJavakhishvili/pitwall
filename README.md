# Pitwall

<p align="center">
  <img src="docs/brand/pitwall-app-icon-1024.png" alt="Pitwall" width="96" />
</p>

<p align="center">
  <a href="https://aleksandrejavakhishvili.github.io/pitwall/">website</a> ·
  <a href="#install">install</a> ·
  <a href="https://aleksandrejavakhishvili.github.io/pitwall/docs/quick-start/">quick start</a> ·
  <a href="https://aleksandrejavakhishvili.github.io/pitwall/docs/how-it-works/">how it works</a> ·
  <a href="https://github.com/AleksandreJavakhishvili/pitwall/releases/latest">download</a>
</p>

---

https://github.com/user-attachments/assets/65903f5d-1b2c-41a0-ab35-a2451a26d7c0

<p align="center"><sub>Recorded from the app's browser mock mode. <a href="https://aleksandrejavakhishvili.github.io/pitwall/">Try the clickable live demo on the website.</a></sub></p>

**You call the strategy. Agents drive.**

Pitwall is a macOS app for running terminal coding agents (Claude Code, Codex, Gemini CLI and the rest) side by side, and seeing at a glance which one is waiting for you. It is a tool, not an agent: it never rewrites your prompts or decides what an agent does. Text you send goes in exactly as written.

- **The one that needs you turns amber.** A permission prompt or a question flags the agent, sorts it to the top, lights a bar along the bottom of the window and counts it on the Dock badge. `⌘J` jumps there. Done agents get a flag of their own until you look.
- **Status without wrapping the agent.** Hooks when the agent offers them (Claude Code per launch, Codex opt-in), otherwise its terminal screen matched against per-agent rules, otherwise output activity. Hooks are never required.
- **Wall** (`⌘E`): every live terminal at once, grouped by project, the ones that need you first. View-only; click a tile to take over.
- **Next up** (`⌘.`): queue the next instruction while the agent works. It's sent when the agent is done or idle and you aren't typing in it.
- **Review** (`⌘R`): what each agent changed, per task. Comment on lines and send them back as one prompt; commit and merge only when you click.
- **Spaces and tiles**: from one pane up to 4×4, spaces in their own windows, density settings for big walls.
- **Terminals anywhere** (`⌘T`): a plain terminal in the focused agent's folder. Start `claude` or `codex` in it and the pane becomes that agent; Restart resumes it.
- **Rules through [rulesync](https://github.com/dyoshikawa/rulesync)**: one rule library, written into CLAUDE.md, AGENTS.md and friends per project or per agent. Generated files stay out of your diffs.
- **Worktrees through the agent's own flag** (`claude --worktree`, `codex --worktree`). Pitwall creates no branches or folders of its own.
- **First launch is a read-only scan**: agent CLIs on your PATH, recent projects and conversations you can continue. Nothing changes until you tick a box.

## Supported agents

| Agent | Command | Status from | Resume |
|---|---|---|---|
| Claude Code | `claude` | hooks + screen | yes |
| Codex | `codex` | hooks (opt-in) + screen | yes |
| Gemini CLI | `gemini` | screen | yes |
| opencode | `opencode` | screen | yes |
| Cursor Agent | `cursor-agent` | screen | yes |
| GitHub Copilot CLI | `copilot` | screen | yes |
| Qwen Code | `qwen` | screen | yes |
| Amp | `amp` | screen | yes |
| Aider | `aider` | screen | no |
| any other CLI / shell | `$SHELL` | output activity | no |

Agents launch through your login shell (`$SHELL -l -i -c`), so they see the same PATH, logins and config as in your terminal. A new agent is one TOML file: definitions live in [`crates/pitwall-core/agents/`](crates/pitwall-core/agents/) and detection rules in [`crates/pitwall-detect/detect/`](crates/pitwall-detect/detect/); put your own in `~/.pitwall/agents/` and `~/.pitwall/detect/`.

## Terminals that survive restarts

Each agent's terminal is owned by its own small `pitwall-hold` process, not by the app. `⌘Q` closes the UI only: dev servers and half-finished turns keep running, and the next launch re-attaches with the scrollback replayed. Only Stop, Restart, Remove or **Quit and Stop Agents** (app menu) end a terminal. After a reboot, agents come back stopped and Restart resumes the conversation for CLIs that support it.

```
Pitwall.app      UI, quit any time
   │  attach · write · resize · replay      ~/.pitwall/run/hold/<agent>.sock
   ▼
pitwall-hold     one per agent, setsid, owns the PTY
   ▼
claude --session-id …   still running
```

## agw machines

If you run agents on VMs with `agw`, its sessions sit next to your local agents, grouped by machine.

- **Works today:** add an existing session and get its live terminal, its status (screen detection), Next up, stop and start; diffs and Review run git on the VM through agw; New agent → *Runs on* creates a session on a VM using agw's own workspaces, users and templates.
- **Not yet:** hooks and rules on VMs, merging from Review, and deleting a session on the VM. Removing an agw agent from Pitwall only detaches it.

## The `pitwall` CLI and approvals

Settings → *Install command-line tool* links `pitwall` into `~/.local/bin` or `/usr/local/bin`. It talks to the running app over a local socket and prints JSON (`--human` for text), so agents can use it too. Inside a Pitwall pane it finds the app on its own.

```bash
pitwall agent list
pitwall agent new --kind claude --project ~/code/api --name api
pitwall session list --provider agw
pitwall session add agw <vm> <session> --start
pitwall machine form <vm>      # what a new session on that VM can be
```

Today the CLI lists and adds; it can't stop, remove or prompt agents. Anything that changes another machine (starting or creating a session on a VM) opens an approval dialog in Pitwall and the command waits for your answer; a denial exits with status 3. Pitwall identifies the caller from the connecting process, so an agent can't approve its own request. There's an agent skill for it in [`skills/pitwall/SKILL.md`](skills/pitwall/SKILL.md).

## Install

macOS only for now, Apple silicon and Intel. Windows and Linux are planned.

### Download

1. Get the `.dmg` from the [latest release](https://github.com/AleksandreJavakhishvili/pitwall/releases/latest) and drag Pitwall to Applications.
2. Open it the first time with right-click → **Open**, then **Open** in the dialog.

Releases aren't signed with an Apple Developer ID or notarized yet, so Gatekeeper blocks a plain double-click on first launch. If macOS says the app "is damaged", clear the quarantine flag instead:

```bash
xattr -dr com.apple.quarantine /Applications/Pitwall.app
```

Until releases are Developer ID signed, macOS sees each new version as a different app and may ask again for folder access (Desktop, Documents) after an update. Each release has a `SHA256SUMS.txt` next to the downloads.

### Homebrew

Coming. A cask is ready in [`packaging/homebrew/pitwall.rb`](packaging/homebrew/pitwall.rb) and will be published as `brew install --cask aleksandrejavakhishvili/tap/pitwall` once the tap exists.

### Build from source

You need the Xcode Command Line Tools, Rust (`rustup`), Node.js 20.19+ and `pnpm` (the [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/)), plus at least one agent CLI you're logged in to.

```bash
git clone https://github.com/AleksandreJavakhishvili/pitwall.git
cd pitwall
pnpm install
pnpm tauri build --bundles app     # → target/release/bundle/macos/Pitwall.app
```

The build also compiles `pitwall-hold` and the `pitwall` CLI and puts both inside the app bundle.

Then either run it in place (`open target/release/bundle/macos/Pitwall.app`) or copy it to `/Applications`. A build you made yourself isn't quarantined, so it opens without the right-click step, but macOS will re-ask for folder access after every rebuild because each build has a different signature.

To avoid that, sign your builds with a local certificate: `scripts/install-local.sh` quits Pitwall (agents keep running), copies the build to `/Applications`, signs it and its helpers with an identity called **Pitwall Local Signing**, and reopens it. Privacy grants then survive rebuilds. Create the identity once:

1. Keychain Access → Certificate Assistant → **Create a Certificate…**
2. Name: `Pitwall Local Signing`, Identity Type: **Self Signed Root**, Certificate Type: **Code Signing**. Create.
3. Check: `security find-identity -p codesigning` lists it.

```bash
scripts/install-local.sh                         # uses target/release/bundle/macos/Pitwall.app
PITWALL_SIGN_IDENTITY="My Identity" scripts/install-local.sh path/to/Pitwall.app
```

## Development

```bash
pnpm install
pnpm tauri dev            # the app with hot reload
pnpm dev                  # UI only, in a browser, with mock agents (no Rust needed)
cargo test --workspace    # Rust: core, detection, holder, providers, CLI
pnpm test                 # UI tests (vitest)
pnpm build                # type-check + UI build
```

The code is a Cargo workspace: `src-tauri/` is the app shell, `crates/pitwall-core` holds the logic without Tauri, `crates/pitwall-detect` the screen rules, `crates/pitwall-hold` the terminal holder, `crates/pitwall-providers` where agents run (this Mac, agw), and `crates/pitwall-{proto,client,daemon,cli}` the socket protocol and CLI. The UI is React + TypeScript in `src/`. Start with [`docs/spec/architecture.md`](docs/spec/architecture.md); the rest of the specs are indexed in [`docs/CONTRACT.md`](docs/CONTRACT.md) and the plan is in [`docs/spec/roadmap.md`](docs/spec/roadmap.md).

### Website

The site in [`website/`](website/) is plain HTML + Vite and embeds the app's mock build as a live demo. `cd website && pnpm install --ignore-workspace && pnpm build --base /pitwall/` builds both (see [`website/README.md`](website/README.md)).

It deploys to GitHub Pages from `.github/workflows/pages.yml` on every push to `main`. To turn that on once: **Settings → Pages → Build and deployment → Source: GitHub Actions**.

### CI and releases

- `.github/workflows/ci.yml`: `cargo test` and `cargo clippy` on macOS, UI build + tests and the website build on Linux.
- `.github/workflows/release.yml`: push a tag like `v0.1.0` to build a universal `.dmg` and `.app.tar.gz` with checksums and publish a GitHub Release. Builds are ad-hoc signed unless the Apple signing secrets listed at the top of the workflow are set, in which case they're Developer ID signed and notarized.

## Credits

- Agent detection rules and parts of the rule model are adapted from [Herdr](https://github.com/herdrdev/herdr) (Apache-2.0). Details in [NOTICE](NOTICE).
- File and folder icons from [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme) (MIT).
- Fonts: Inter, JetBrains Mono and Barlow Condensed (SIL Open Font License 1.1), bundled through `@fontsource`. Licence texts are in [LICENSES/](LICENSES).

Pitwall is not affiliated with Formula 1 or with any agent vendor named here.

## License

Pitwall is licensed under the [Apache License 2.0](LICENSE).
