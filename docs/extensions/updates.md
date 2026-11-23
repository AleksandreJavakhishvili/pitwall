# Updates: one ordered step

An extension with a [service](service.md) and a setup action has three
parts that move together: its files, the server Pitwall keeps running, and
the plugin its setup installs in the user's agents. Updating them one at a
time can leave them out of step: agents still on the previous plugin, a
new server that can't read the old data, or a server replaced by a build
that doesn't start. So Pitwall updates an extension as one ordered step,
and puts it back by itself when a step fails.

## The steps

After the user presses Update on the review, Pitwall runs, in order:

1. **Back up.** If the extension is on, Pitwall runs its service here and
   the service's data folder has something in it, Pitwall stops the
   service and backs up its data (see [Backups](#backups)).
2. **Install.** The new files go in place. The previous copy is kept for
   Go back.
3. **Update the server's program.** If the service's program is missing
   or older than the new manifest's `service.version.requires`, Pitwall
   runs the service's `install` action. The review lists its commands, and
   pressing Update confirms this run.
4. **Start and check.** Pitwall starts the new service and waits until it
   is up (at most 60 seconds). Then it checks that the service is still up
   and that the adapter connects as it did before the update (at most 30
   seconds). An adapter that wasn't connected before only has to keep
   running.
5. **Set up the agents.** Each setup action that was set up before and
   whose steps changed runs again, for the tools' default folders and
   every account. The agents' plugin goes last, after the new server is
   up.

A step that doesn't apply isn't listed: an extension that is off only gets
its new files (its service and setup run when it is turned on), and a
service that runs elsewhere (one the user started on the same address) is
never touched.

The review lists the steps exactly as they will run, with each command
and its settings filled in. Pressing Update confirms the service's new
commands (when the user had confirmed the previous ones) and the setup's
new steps, so nothing asks again halfway.

## When a step fails

- **Steps 1 to 4** put everything back by themselves: the previous files,
  the service's data from the backup, and the service's commands as they
  were confirmed. Then the previous service starts again. The card says
  what failed and what was put back. When step 3 updated the server's
  program, Pitwall can't put the older program back: the card says so and
  what the user can do (run that version's own install command, or update
  again once a fixed release is out).
- **Step 5** doesn't undo the update. The new server already runs, and a
  setup that failed for one account (a tool not logged in, no network)
  says which accounts failed, with Retry.

A backup that fails stops the update before anything changes.

## Order

```json
"update": { "order": "serviceFirst", "agents": "compatible" }
```

`order` is `serviceFirst` (the default) or `agentsFirst`.

- `serviceFirst`: the new server starts before the agents' setup changes,
  so the server must accept clients from the previous release. This is
  the usual promise of a server: new servers accept old clients.
- `agentsFirst`: the setup runs first (step 5 moves before step 3), for
  an extension whose new plugin works with the previous server but not the
  other way round.

## The compatibility promise

An agent loads its plugin and its variables when it starts. An agent
started before an update keeps them until it is restarted. Pitwall keeps,
for each agent, the setup it started under: a digest of the extension's
setup steps that ran, the variables it got and its connection. When that
changes (an update, a setup run with other steps, a setting its variables
use), the agents that started before **use the previous setup**:

- Their row in the sidebar and their pane's header show *old setup*, with
  a tooltip naming the extension.
- The extension's card lists them ("2 running agents use the previous
  setup") with Restart now for each and Restart all idle.
- `pitwall agent list` says so, and `staleFor` in an agent's view (the
  socket API and `--json`) lists the extensions.

`agents` says what that means for the extension:

- `compatible` (the default): agents started before keep working with
  what they started with. The chip is only information.
- `restart`: they stop working with it until restarted. The review says
  so as a risk, and the chip is a warning.

## Restarting the agents

What Pitwall does with those agents is the user's choice:
Settings → Extensions → "After an update, restart agents that use the old
setup" (`extensions.restartAfterUpdate`), and on each card, to override it
for that extension.

- **Ask** (the default): Pitwall restarts nothing by itself.
- **When idle**: Pitwall restarts each one once its own status has been
  idle or done (waiting for the user) for 30 seconds. Never one that is
  working or waiting on an approval. Agents are restarted one at a time,
  each continuing its conversation as Restart does. An agent that can't
  continue its conversation is never restarted by itself. A notice says
  "Restarted <agent> to pick up <extension>", and the extension's Activity
  keeps it.
- **Never**: nothing. The chip stays until the user restarts them.

## Backups

Before the service is replaced, Pitwall stops it and backs up its data
into a new folder of its own:
`<Pitwall's data folder>/extension-backups/<id>/<YYYYMMDD-HHMMSS>` (UTC),
mode 700. It keeps the last three.

A service can name its own backup command, which is best for a database:

```json
"service": {
  "run": ["inbox", "serve", "--data", "{data}"],
  "backup": {
    "run": ["inbox", "backup", "--data", "{data}", "{backup}/inbox.db"],
    "replaces": ["inbox.db", "inbox.db-wal", "inbox.db-shm"]
  }
}
```

- `run`: the command. `{backup}` is the new, empty backup folder, and it
  must appear. The other [placeholders](service.md#placeholders) work as
  in the service's `run`. It runs with the service stopped, in its data
  folder, with the same small environment, for at most 10 minutes.
- `replaces`: the files in the data folder the backup stands for. To put
  the backup back, Pitwall moves those files aside (into the backup
  folder's `replaced/`, never deleting them) and copies the backup's files
  in, mode 600. Without `replaces`, Pitwall can't put the backup back by
  itself: it says where the backup is.

`backup` may also be just the command, as a list.

Without a `backup`, Pitwall copies the data folder itself, up to 2 GiB. A
bigger folder isn't copied: the review says so, and the button becomes
"Update without a backup". If the backup command fails, Pitwall copies the
folder instead.

## Going back

Go back puts back the copy from before the last update. When that update
backed up the service's data, Go back asks whether to put the data back
too:

- **Keep current data** (the default): the service keeps what it stored
  since the update. Right when the previous version reads the new data.
- **Put back the backup too**: the data is as it was before the update.
  What is there now is moved into the backup folder, not deleted. Whatever
  was stored since the update is no longer used.

Go back also confirms the service's commands as they were before the
update, since those are what it puts back.

## Checking it

`pitwall extension check` describes the backup and the `update` block,
and warns when a service has no `backup` or one Pitwall can't put back.
`test/pitwall.json` can assert the steps an update takes
([checks](checks.md#update)).
