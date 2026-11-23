# A service: a program Pitwall keeps running

Some extensions need a server on this computer for their agents to talk
to. A [format 2](manifest-2.md) manifest may declare one `service`: a
program Pitwall starts while the extension is on and keeps running. The
user turns the extension on, confirms the service's exact commands once,
and signs in once, if the service asks for a first run. Pitwall does the
rest: it installs the service through one of your actions, checks its
version, starts it, restarts it when it stops, and stops it when the
extension is turned off.

```json
{
  "v": 2,
  "id": "my-inbox",
  "name": "My inbox",
  "version": "1.0.0",
  "pitwall": ">=0.9.0",
  "settings": [
    { "key": "server", "type": "url", "label": "Server address", "required": true },
    { "key": "listen", "type": "text", "label": "Listens on", "default": "127.0.0.1:8787" },
    { "key": "apiKey", "type": "secret", "label": "API key" }
  ],
  "actions": [
    { "id": "server", "label": "Install the server", "description": "Installs it with npm.",
      "steps": [{ "run": ["npm", "install", "-g", "my-inbox-server@0.6.0"] }] }
  ],
  "service": {
    "label": "The inbox server",
    "run": ["inbox", "serve", "--data", "{data}", "--origin", "{setting:server}", "--listen", "{setting:listen}"],
    "install": "server",
    "version": { "run": ["inbox", "version"], "match": "(\\d+\\.\\d+\\.\\d+)", "requires": ">=0.6.0" },
    "ready": { "listen": "{setting:listen}" },
    "env": [{ "name": "INBOX_KEY", "secret": "apiKey" }],
    "firstRun": {
      "label": "Sign in for the first time",
      "run": ["inbox", "link", "--data", "{data}"],
      "opens": true,
      "again": "A new link replaces the last one."
    }
  }
}
```

`pitwall extension new <folder> --service` writes a working one with a
tiny Node server ([the starter](starter.md#pitwall-extension-new-folder---service)).

## The keys

| Key | What it is |
| --- | --- |
| `label` | What the card calls it ("The inbox server"), up to 40 characters. |
| `run` | Its command, as a list: the program, then each argument. There is no shell. The program is a name looked up on the user's login PATH (`inbox`, `node`), never a path. |
| `install` | Optional. The id of one of the manifest's `actions` that installs or updates the program. The card offers it while the program isn't on the PATH or is too old, confirmed as any action. |
| `version` | Optional. `run` prints its version; `match` is a pattern whose one group is the version; `requires` (`>=X.Y.Z`) is the lowest version it needs. Below it, the service isn't started, and the card says "The inbox server 0.5.0; this extension needs 0.6.0 or later" with one button: the install action, then the service is started again. A version it can't read doesn't stop it. |
| `ready` | How Pitwall tells it's up: `{ "listen": "<address>" }`, an address on this computer it listens on, or `{ "line": "<pattern>" }`, a line it prints. Without `ready`, it's up once it has run for a second. |
| `env` | Optional, at most 8 variables: `{ "name", "value" }` (placeholders allowed) or `{ "name", "secret": "<a secret setting>" }`. A secret is read from the system's secure store when the service starts and is shown on the confirm card by name. Its value is taken out of the Log. |
| `firstRun` | Optional. A command that runs only when the user presses its button: `label`, `run`, `opens` and `again`. With `opens: true`, Pitwall opens the first `https` link the command prints in the user's browser. `again` says what running it again does; the card asks before it runs again. |

### Placeholders

Each argument can use `{setting:KEY}` (a plain setting of the
extension's default connection; an unset one waits, and a setting's
default counts) and `{data}`: the service's own folder,
`<Pitwall's data folder>/extension-data/<id>`. Pitwall makes `{data}`
private to the user (mode 700 on macOS and Linux), outside the
extension's installed folder, so an update never touches it. A secret
is never a placeholder: pass it through `env`.

### Addresses

`ready.listen` must be an address on this computer: `127.0.0.1`,
`localhost` or `[::1]`, with a port. Pitwall starts nothing that
listens for other computers. To reach it from elsewhere, put a proxy in
front of it (such as `tailscale serve`), which stays yours.

## What the user sees

The extension's checklist gets three steps, after the settings the
service uses:

1. **Install**: ticked once the program is on the PATH and new enough.
   Otherwise the step's button runs the `install` action, after the usual
   confirmation, and then Pitwall looks again.
2. **Confirm and start**: the card says "Pitwall will keep this running
   while the extension is on:" with the exact command, its version
   command, its first run, its variables and where its data is. **Confirm
   and start** stores a digest of these commands (as setup confirmations
   are stored) and starts it. Afterwards the row shows its state:
   *starting*, *running since 14:02*, *stopped* (with why and its last
   lines), *running elsewhere*, or what it waits for. It has Start, Stop
   and Restart, and the card's **Log** button shows its output.
3. **First run** (with `firstRun`): while the service is fresh, meaning
   its data folder was empty when Pitwall started it, its button runs the
   command. Once that succeeds, the step is ticked and the button moves
   under **More**.

"What this extension does on your computer" and the permission screen
say that it runs a local program continuously.

## How Pitwall runs it

- **Only with consent.** Nothing starts before the user confirms the
  commands. If an update changes any of them, the service is stopped and
  the card asks again; Review lists the service's changes with what a
  service can do.
- **In its own group, with little else.** It runs in its own process
  group (a job object on Windows), in the extension's folder, with stdin
  closed. Its environment is only the login PATH, home, user, language,
  time zone and temporary folder, plus the manifest's `env`. Its stdout
  and stderr go to the extension's Log, with secrets taken out.
- **Kept running.** If it stops by itself, Pitwall starts it again after
  1, 5 and 30 seconds. A fourth stop within ten minutes gives up: the row
  says *stopped* with its last lines, and Start tries again.
- **Stopped completely.** Turning the extension off, updating it,
  removing it and quitting Pitwall stop it. Pitwall sends the whole group
  SIGTERM, waits 3 seconds, then kills what's left. On Windows the job is
  closed straight away.
- **Never someone else's.** Before it starts the service, Pitwall checks
  whether its `ready.listen` address is free by binding it; it never
  connects to it. If something else already listens there, the row says
  "Something else is already listening on 127.0.0.1:8787" and *running
  elsewhere*. Pitwall uses that one and never starts, stops or signals it.
  It checks again every 30 seconds without connecting.
- **The first run's link is a secret.** Pitwall opens it in the browser
  and keeps it nowhere. The first run's output isn't logged; when it
  fails, the card shows its last lines with every link replaced by
  `[link]`. The card and the activity log say "Opened a one-time sign-in
  link in your browser".
- **Remove asks about its data.** The remove card names the data folder
  and keeps it unless the user switches on deleting it.

A [development folder](dev-mode.md) follows the same rules: the same
confirmation and the same data folder (by id). After Reload, commands
that changed are asked about again.

## Not supported

- Starting the service at login without Pitwall. It runs while Pitwall
  runs; a server that must outlive Pitwall stays the user's to run, and
  Pitwall then shows it as *running elsewhere*.
- HTTPS, certificates or a public address: put a proxy in front.
- A sandbox. The service runs as the user, like an adapter (see
  [docs/spec/extension-api.md](../spec/extension-api.md#process-sandboxing-per-os)).
- More than one service per extension.

## Checking it

`pitwall extension check` describes the service, says that it runs
continuously, and checks that its program is found (or that `install`
names an action). `pitwall extension test` checks its commands and what
Pitwall would do in the cases you list, with nothing started:
[`service` in `test/pitwall.json`](checks.md#service).
