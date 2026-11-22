# Accounts, connections and profiles

Status: Phase A built (storage, migration, resolution, launches,
extension connections in the hub, CLI). Phase B: Settings, extension
cards and the project's Accounts… built; the inbox's connection filter
comes after the inbox's own window (#65).

Pitwall launches an agent with an identity: which login of its tool it
runs on, who it commits as, which sign-ins its web pages use, and which
of the user's services its extensions connect it to. This page says what
those things are, how a project picks them, and how one choice is found
for every launch. **They only decide the environment and identity Pitwall
launches an agent with**; nothing here changes what an agent may do.

## The things a project picks

| Slot | What it is | Kept as | "Default" means |
|---|---|---|---|
| a tool kind (`claude`, `codex`, `gemini`, `copilot`) | an **account**: one login folder of that tool (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`, `COPILOT_HOME`) | `accounts[]`, `kind` = the tool | the user's own login |
| `web` | a **browser account**: a store of cookies and site data for Pitwall's web pages (docs/spec/web.md) | `accounts[]`, `kind` = `web` | the store pages used before browser accounts |
| `git` | a **git identity**: who commits, its GitHub / Azure DevOps logins, its SSH key | `git[]` | the user's own git setup |
| `ext:<extension id>` | an **extension connection**: one set of an extension's settings and secrets ("Agention · Work") | `accounts[]`, `kind` = `ext:<id>` | the extension's default connection |

Accounts are the real things (a login, a key, a folder, a server
address); profiles and projects only name them.

### Extension connections
- An extension whose manifest declares `settings` can have **several
  connections**, each with its own values for those settings and its own
  secrets. A connection is an account of kind `ext:<id>` (`:` can't be in
  an agent kind's id, so it never collides with a tool or `web`), with id
  `ext-<id>-<name>` (made unique), a name unique among that extension's
  connections, and `dir: ""` (it has no folder).
- Its plain settings are in the account (`values`), which keys of its
  secrets are set too (`secretsSet`); the secrets themselves are in the
  system's secure store under `<secretKey>.<setting key>` (plus the data
  folder's hash for a data folder other than the default one), where
  `secretKey` is the connection's id, or the extension's id for a
  connection carried over from before connections (below), so its
  keychain items keep their names.
- **The default connection** of an extension (`defaults["ext:<id>"]`) is
  what applies wherever nothing picks one, as a tool's own login does.
  The first connection made for an extension becomes its default; the
  user can make another the default, or none. Removing the default
  connection leaves the extension with none.
- Extensions without settings (the demo inbox) have no connections and
  behave as before: their variables go on every agent while they are on.
- No manifest key is added: every extension with settings can have
  connections (a service the user reaches with two keys or two servers is
  the common case; an extension with one makes one).

### Profiles
- A **profile** is a named bundle of picks ("Work", "Personal"): for any
  slot above, an account id, `default` (that slot's Default), or, for an
  extension slot, `none` (no connection). A slot it leaves out isn't
  decided by it.
- `profiles: [{id, name, picks}]` in `accounts.json`; ids are
  `profile-<name>` (made unique); names unique, not empty.
- A **project picks a profile** (`projectProfiles[<folder>]`), or none.
- A project may **override** single slots: the picks it already has
  (`projects[<folder>][<slot>]`, and `gitProjects[<folder>]` for git) are
  its overrides. A value `default` or `none` there is an override too.
  A project with no profile keeps its per-slot picks exactly as before:
  nothing is migrated.

## Resolution (one place)
`pitwall_core::accounts::Accounts::resolve_slot(folder, slot)` and, for a
launch, `resolve_launch(cwd, project, slot)`. Everything that needs a pick
goes through them: agent launch environment (accounts, git identity,
extension variables), web pages' browser account, extension thread
matching, the sidebar's account sections, the CLI.

1. **The project's override** for the slot;
2. else **its profile's** pick for it;
3. else **the slot's Default** (the user's own login / store / git setup;
   for an extension, its default connection, or none).

`default` at any step means step 3's answer; `none` means no connection;
an id that names nothing of that slot (removed, or `accounts.json` edited
by hand) counts as Default. A launch in a folder (`cwd`) that is itself a
project with an override or profile for the slot uses that folder's
answer; otherwise its project's (a repository's top). The result says
where it came from (project, profile, or default), for the screens.

An agent's tool account is still settled when it is created and kept in
its record (moving it is "Run on account"); its git identity, extension
variables and pages' browser account are worked out again at each launch.

## Storage

`accounts.json` (keys an older Pitwall doesn't know are ignored by it;
keys a later one adds are kept by this one):
```json
{
  "accounts": [
    {"id": "claude-work", "kind": "claude", "name": "Work", "dir": "/Users/sam/.pitwall/accounts/claude-work"},
    {"id": "web-work", "kind": "web", "name": "Work", "dir": "/Users/sam/.pitwall/webview/profile-web-work"},
    {"id": "ext-agention-agention", "kind": "ext:agention", "name": "Agention", "dir": "",
     "values": {"server": "https://agention.work.example"}, "secretsSet": ["apiKey"], "secretKey": "agention"},
    {"id": "ext-agention-personal", "kind": "ext:agention", "name": "Personal", "dir": "",
     "values": {"server": "https://agention.home.example"}, "secretsSet": ["apiKey"]}
  ],
  "projects": {"/code/side": {"ext:agention": "ext-agention-personal"}, "/code/scratch": {"ext:agention": "none"}},
  "git": [{"id": "git-work", "name": "Work", "userName": "Sam Doe", "email": "sam@work.example", "dir": "…"}],
  "gitProjects": {"/code/api": "git-work"},
  "profiles": [
    {"id": "profile-work", "name": "Work",
     "picks": {"claude": "claude-work", "codex": "default", "git": "git-work", "web": "web-work", "ext:agention": "ext-agention-agention"}}
  ],
  "projectProfiles": {"/code/api": "profile-work", "/code/web": "profile-work"},
  "defaults": {"ext:agention": "ext-agention-agention"}
}
```
`ui.json` keeps what is per extension, not per connection:
`extensions.<id>.enabled`, `.selected` (the thread its inbox shows),
`.window` once the inbox can have a window of its own (one inbox per
extension, so the window is the extension's) and `.actionsDone` for setup actions that
don't use `{setting:KEY}`. A setup action that does is per connection:
its tick is the connection's `actionsDone` in `accounts.json`.

## Migration from before connections
On start, an extension with `values` or `secretsSet` in `ui.json` and no
connection yet becomes **one connection named after the extension**, with
those values, its secrets' keys and `secretKey` = the extension id (so
the keychain items it already has are used as they are: nothing is
re-entered and nothing is read or rewritten in the keychain), made the
extension's **default connection**, with the ticks of the setup actions
that use `{setting:KEY}` as its `actionsDone`; `selected` is given the
connection's prefix.

**Downgrade-safe**: `values`, `secretsSet` and those ticks **stay** in
`ui.json` as a copy of that carried-over connection (the account whose
`secretKey` is the extension id), rewritten whenever it changes, so an
older Pitwall started again finds the extension set up as it left it.
On each start, a copy that differs from the connection means an older
build changed it: the connection takes the copy's values, secrets' keys
and ticks. Removing that connection clears the copy (it isn't adopted
again). Other connections never touch the copy, so an older build only
ever sees the carried-over one. (A crash between writing
`accounts.json` and the copy would, on the next start, take the copy's
older values: the one window in which a change can be undone.)

Why a default connection rather than "every current project picks it":
before, the variables went on **every** agent Pitwall launched, in
folders that aren't projects too, and in projects added later. Only a
default reproduces that exactly; naming it in each project would miss
those, and would pin projects to it when the user later makes another
connection the default.

## Removing things
- **An account, connection or git identity** removed: the projects and
  profiles that name it forget it, and so does `defaults`; each place
  then falls back a step (a project's override to its profile's pick,
  a profile's pick to Default). Its folder and login stay on disk (as
  before); a connection's secrets are deleted from the secure store.
  Settings says which projects and profiles use it before removing.
  Agents already running keep running; one whose account is gone can't
  restart on it (as before).
- **A profile** removed: its accounts are left alone. Each of its
  projects goes back to no profile **with its effective picks kept**:
  every slot the profile decided and the project didn't override is
  written into the project as an override, so nothing any agent runs
  under changes.

## Changing a project's profile or picks
The same as changing a pick in the project's Accounts… dialog today:
- a tool account that changes: the project's agents of that tool move to
  it at once (their conversations with them; an account not logged in yet
  starts its login terminal first and they move once it is), as dropping
  a project on another account in the sidebar does;
- a git identity that changes: the project's agents restart to take it,
  busy ones once they're free; terminals take it when restarted (a toast
  says so);
- a browser account or extension connection that changes: pages and
  agents started from then on use it (nothing restarts).
`Accounts::changed_slots(before, after, folder)` says which slots
changed. In the app every pick change (Settings' Profiles, a project's
Accounts…) goes through `AccountsModel::change_picks`, which emits
`PicksChanged` with each project's changed slots; the main screen applies
them as above, and says a connection change in a toast when the
project's agents are running. From the CLI the change is recorded and its answer lists the
running agents that still use the old pick; nothing restarts without the
user doing it in Pitwall.

## Extension adapters, one per connection
- Every configured connection (its required settings set) of an enabled
  extension has its own adapter process, whose `hello` carries that
  connection's settings and secrets. They run whether or not a project
  using them is open, so the inbox shows asks from all of them. At most
  **8 per extension** run (`hub::MAX_RUNNING`); a connection past that
  says so instead of starting.
- **One inbox per extension**: the threads of all its connections in one
  list, each labelled with its connection's name; ids are
  `<connection id>:<adapter's id>`, so requests go to the right adapter.
  A connection that stops or can't reach its service shows as offline
  (a line over the list naming it) without hiding the others' threads.
  Counts and badges add up the connections'.
- **A filter** in the inbox's bar, next to "Needs you / All": "All
  connections" (the default), then each connection by name, with the
  names of the profiles that use it ("Work"), remembered per tile. A
  thread's row names its connection whenever there is more than one; with
  one connection there is neither label nor filter. The merged state
  carries each thread's connection (`ExtState::connection_of`) and counts
  per connection as well as in total (`ExtState::counts_of`), so the
  filter needs nothing from the protocol.
- **Agent variables** come from the agent's connection for that
  extension (resolution above), with `{setting:KEY}` filled from that
  connection's settings; with none, the extension sets nothing on it.
- **Matching** (`matchAgents`): a thread from connection C is agent A's
  only if C is A's connection for that extension now.
- **Setup actions** that use `{setting:KEY}` run for one connection (its
  settings filled in); the others are extension-wide.

## Where the user sets them
- **Settings → Accounts**: agent logins, git identities, browser
  accounts as before; then, once an extension has two connections or
  more, **Extension connections** (read-only: each connection's address,
  the default marked, the profiles and projects using it, "Edit in
  Extensions"); then **Profiles** once there is something to bundle (a
  second login, a git identity, a browser account, a second connection)
  or a profile exists: add, rename, delete (asks; says which projects
  keep its picks as their own), and each slot's choices as chips: "Not
  set", the default, each account of that kind only, and for an
  extension None.
- **The extension's card** owns its connections: with one connection the
  card is as before plus an "Add connection" button; with two or more, a
  row of connection tabs (a status dot each, the default marked, the
  state in its tooltip) over the selected connection's checklist and
  setup actions, with Rename, Make default and Remove. Removing the
  default asks which becomes the default, or none; it never promotes one
  silently. The pill says "N of M connected".
- **A project's Accounts…**: "Profile" first (only once a profile
  exists); each slot says where its answer comes from ("from Work",
  "this project's own" with "Back to Work", "default"); a chip picked
  there is the project's own. An extension gets a row with two or more
  connections, or when the project or its profile picks one.
- **The sidebar** groups by the tool account each project resolves to
  (its own pick, else its profile's, else the default); dropping a
  project on another account writes the project's own pick, which
  overrides its profile.

## CLI
`pitwall account list`, `pitwall profile list|create|rename|delete|set`,
`pitwall project accounts <folder>` (each slot's answer and where it came
from), `pitwall project profile <folder> <profile|none>` and
`pitwall project pick <folder> <slot> <value|inherit>`. Slots are a tool
kind, `git`, `browser` (or `web`) and `ext:<extension id>`; values an
account's name or id, `default`, or `none` (extensions). Changes that
decide what agents launch with ask the user in Pitwall when anyone else
asks. See docs/reference/cli.md.

## Naming
The bundle is a **Profile**. What was called a "browser profile" is a
**browser account** in everything the user reads (screens, docs, CLI
help, tool results). Stored ids, folders (`webview/profile-<id>`) and
`ui.json` `webTiles[id].profile` stay; `show_web`'s answer carries
`browserAccount`, and `profile` (the same value) as an alias for older
readers.
