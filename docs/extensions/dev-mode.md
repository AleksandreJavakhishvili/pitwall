# Dev mode

While you write an extension, Pitwall uses your folder where it is.

## Load from folder

Settings → Extensions → **Load from folder…**, then pick the folder that
has `extension.json`.

- The folder is recorded in Pitwall's `ui.json` (`extensions.<id>.devPath`)
  and read in place. Pitwall never copies, changes, updates or deletes
  it.
- It is listed under Installed and marked **development**. A line under
  its name says where it is loaded from.
- Loading it turns it **off**, so nothing runs until you turn it on.
- Its id comes from the manifest. The folder may be named otherwise
  (`pitwall extension check` notes the difference). If an extension with
  that id is installed, your folder is used in its place until you remove
  it from the list. A second folder with an id that is already loaded is
  refused.
- If the folder goes away or its id changes, its card says so. Remove it
  from the list and load the folder again.

## Reload

**Reload** reads the manifest again and, if the extension is on, starts
its adapter again. Pitwall doesn't watch the folder, so press Reload
after you change the manifest or the adapter.

## Remove from list

**Remove from list** turns the extension off and stops listing it. Your
folder is untouched, so Pitwall doesn't ask first. Its settings and
secrets stay, ready for when you load it again.

## Log

**Log**, on any extension with an adapter, shows the last 500 lines while
it runs:

| Mark | Line |
| --- | --- |
| `→` | Pitwall to the adapter |
| `←` | The adapter to Pitwall |
| `!` | What the adapter printed on stderr (in amber) |
| `·` | Pitwall's own notes: started, stopped and why |

Each line has its time. **Copy** copies them all.

- **Secrets are taken out** before a line is kept: the `secrets` in
  `hello`, the values of your secret settings wherever they appear, and
  anything shaped like an API key or a bearer token. Don't count on it
  for anything else. Never print a secret.
- Lines longer than 2000 characters are cut.
- The log is kept across restarts of the adapter. It is cleared when the
  extension is turned off, and it is never written to disk.

## Stopping

Pitwall starts each adapter in a process group of its own (a job object
on Windows). To stop it, Pitwall closes its stdin, gives it 2 seconds to
exit, then kills the whole group. Anything the adapter started stops with
it. Still, exit when stdin closes:
[`pitwall extension test`](checks.md) checks that you do.
