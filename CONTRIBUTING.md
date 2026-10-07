# Contributing to Pitwall

Thanks for helping. This page covers building and testing, the commit
convention, what a pull request needs, and the licence your contribution is
under.

## Build and test

You need Rust (`rustup`; the version is pinned in `rust-toolchain.toml` and
installed automatically), Node.js 20.19+ and `pnpm`, plus the
[Tauri 2 prerequisites](https://tauri.app/start/prerequisites/) for your OS
(Xcode Command Line Tools on macOS, WebKitGTK on Linux, MSVC build tools and
WebView2 on Windows).

```bash
pnpm install
pnpm tauri dev            # the app with hot reload
pnpm dev                  # UI only, in a browser, with mock agents (no Rust needed)
cargo test --workspace    # Rust: core, detection, holder, providers, CLI
cargo clippy --workspace --all-targets -- -D warnings
pnpm test                 # UI tests (vitest)
pnpm build                # type-check + UI build
```

The website (plain HTML + Vite, embedding the app's mock build as a demo):

```bash
cd website
pnpm install --ignore-workspace
pnpm dev                  # local preview
pnpm build --base /pitwall/
```

Where things live is described in the README ("Development") and in
[`docs/spec/architecture.md`](docs/spec/architecture.md).

## Commit messages

Pitwall uses [Conventional Commits 1.0](https://www.conventionalcommits.org/en/v1.0.0/).
The changelog and the release notes are generated from them
([git-cliff](https://git-cliff.org), `cliff.toml`), so the header is what users
read.

```
<type>[(<scope>)][!]: <summary>

[body]

[footers]
```

**Types**

| Type       | For                                               | Changelog section |
|------------|---------------------------------------------------|-------------------|
| `feat`     | a new capability users can see                    | Features          |
| `fix`      | a bug fix                                         | Fixes             |
| `perf`     | faster or lighter, same behaviour                 | Performance       |
| `refactor` | restructuring without behaviour change            | Refactors         |
| `docs`     | documentation and specs only                      | Documentation     |
| `test`     | tests only                                        | Tests             |
| `ci`       | GitHub Actions workflows                          | CI/Build          |
| `build`    | build system, packaging, toolchain                | CI/Build          |
| `chore`    | maintenance that fits nowhere else                | Chores            |
| `revert`   | reverting an earlier commit                       | Chores            |

**Scopes** (optional, one per commit): `core`, `providers`, `agw`, `hold`,
`cli`, `daemon`, `detect`, `ui`, `app`, `website`, `release`, `deps`. Leave the
scope out when a change spans several areas.

**Rules**

- The summary is imperative and lower case ("add", not "Added" or "Adds"),
  with no trailing period.
- The whole first line is at most 72 characters.
- A blank line separates the header from the body. The body explains why, not
  what the diff already shows.
- Breaking changes put `!` before the colon **and** add a
  `BREAKING CHANGE: <what breaks and what to do>` footer.
- Messages git writes itself (`Merge ...`, `Revert "..."`) and autosquash
  commits (`fixup!`, `squash!`, `amend!`) are accepted as they are.

**Examples**

```
feat(ui): show the worktree branch in the session header
fix(core): keep the PATH from the login shell on macOS
perf(detect): skip screens that haven't changed since the last tick
refactor(providers): split the agw transport from the session model
docs: describe agw machines in the README
test(hold): cover a restart while ~/code/my-app is checked out
ci: run the Windows suites in separate steps
build(deps): bump tauri to 2.9
chore(release): v0.2.0
```

```
feat(cli)!: rename --host to --target

The flag selects an agw machine such as my-vm, not a host name.

BREAKING CHANGE: scripts that pass --host must pass --target instead.
```

### Check your messages locally

The repository ships a `commit-msg` hook (POSIX `sh`, no Node needed). Turn it
on once per clone:

```bash
git config core.hooksPath .githooks
```

To check a range of existing commits, or a pull-request title:

```bash
scripts/check-commits.sh origin/main..HEAD
scripts/check-commits.sh --title "fix(ui): keep the selection after a refresh"
sh scripts/test-commit-msg.sh      # the hook's own tests
```

CI runs the same checks on every pull request (job "Commit messages"): each
commit in the PR and the PR title must pass. Commits already on `main` from
before the convention aren't checked. If a commit fails, reword it
(`git rebase -i` / `git commit --amend`) and force-push the branch.

## Pull requests

- **CI is green** on macOS, Linux and Windows (Rust test + clippy on each, UI
  build + tests, website build, commit messages). Windows suites marked "not
  yet ported" in `.github/workflows/ci.yml` are informational, but don't make
  them worse.
- **Behaviour changes come with tests**: Rust tests next to the code or in the
  crate's `tests/`, vitest for the UI. A bug fix adds a test that fails
  without it.
- **No personal data** in fixtures, mocks, screenshots, docs or logs: no real
  names, e-mail addresses, host names, user names or home-directory paths. Use
  neutral examples such as `my-vm`, `~/code/my-app`, `user@example.com`.
- One topic per pull request; keep refactors apart from behaviour changes when
  you can. The PR title follows the commit convention (it becomes the commit
  header when the PR is squash-merged).
- Update the docs (`README.md`, `docs/spec/`) when you change what users or the
  specs describe. `CHANGELOG.md` is generated; don't edit it by hand.

## Licence

Pitwall is licensed under the [Apache License 2.0](LICENSE). By contributing
you agree that your contributions are licensed under the same licence
(Apache-2.0, section 5), without additional terms or conditions. Third-party
code or assets you add must be under a compatible licence and credited in
[NOTICE](NOTICE) or [LICENSES/](LICENSES) as the existing entries are.
