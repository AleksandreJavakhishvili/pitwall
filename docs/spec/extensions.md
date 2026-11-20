# Extensions

Status: **draft**. Built: finding extensions, their manifest, the switch
in Settings, and an empty sidebar section. Open: what an extension shows
(below, "Panels"). The first example is Agention
([`extensions/agention/`](../../extensions/agention/README.md)).

Something that isn't Pitwall's own (a tool, a service the user runs) gets
a place in Pitwall's UI without being part of Pitwall's code. An extension
is a folder with a declarative manifest: **no code from it is loaded or
run**, and it never changes how agents behave.

## Where extensions are
- `<data>/extensions/<id>/extension.json`, in Pitwall's data folder (the
  one with `accounts.json` and `ui.json`: `~/.pitwall` on macOS,
  `$XDG_DATA_HOME/pitwall` on Linux, `$PITWALL_HOME` when set;
  `Paths::extensions_dir`).
- One folder per extension, named after its id. Hidden folders and files
  in `extensions/` are skipped.
- Read when Pitwall starts, and again with "Look again" in Settings →
  General → Extensions. Not watched.
- Installing is copying the folder in (the example's README says how). No
  `pitwall` command for it yet.

## The manifest (`extension.json`, format 1)
```json
{
  "v": 1,
  "id": "agention",
  "name": "Agention",
  "icon": "inbox",
  "description": "Questions your agents ask you, from your own Agention server"
}
```
- `v`: the manifest format (optional, 1). A higher one is refused: "it
  needs a newer Pitwall".
- `id` (required): 1–40 characters, lower-case ASCII letters, digits and
  `-`, starting with a letter, not ending with `-`. It must be the
  folder's name. It keys the extension's settings and its sidebar group.
- `name` (required): one line, at most 40 characters (trimmed).
- `icon` (optional): the name of one of Pitwall's icons (`kit::icons`;
  `inbox`, `bell` and `chat` were added for extensions). Checked like an
  id; a name this Pitwall doesn't have draws the extension icon (a puzzle
  piece), so a manifest written for a later Pitwall still loads.
- `description` (optional): one line, at most 200 characters, for
  Settings and the heading's tooltip.
- Keys Pitwall doesn't know are ignored, so later versions can add theirs
  without breaking this one.
- Not JSON, not an object, a field of the wrong type, a missing `id` or
  `name`, or more than 64 KiB: the extension can't be used, and Settings
  says why (`pitwall_core::extensions::parse`).

## On and off
- Each extension found is listed in Settings → General → Extensions: its
  icon, name, description and a switch. One that can't be used shows its
  folder and why.
- **Off until the user turns it on**, as other opt-in features (an
  extension copied into the folder shows nothing until then).
- Kept in `ui.json` as `extensions.<id>.enabled` (absent: off; the entry
  goes when turned off). Keys a later version adds to an entry are kept.
  The settings schema (`docs/settings.schema.json`) describes it. It isn't
  a `pitwall settings` key (the registry has fixed keys; these are the
  extensions' ids); editing `ui.json` applies live, as for other settings.
- Right-click on its sidebar heading: "Turn off".

## The sidebar section
- Each extension that is on gets a section after the projects (and
  machines), in folder order: a heading shaped like a project's (chevron,
  its icon, its name; its description in the tooltip). Click folds it;
  folded is kept in `ui.json` `collapsed` as `ext:<id>`.
- Open, it shows one quiet line, "Nothing to show yet", until panels are
  decided.
- Not dragged (projects' order doesn't include it), and not in the narrow
  icon rail.

## Panels (open)
What an extension shows under its heading and when it is opened is not
decided. The first idea, web pages in web tiles, was dropped: extensions
get native UI. Until this is designed, the manifest has no panel fields
and the section stays empty.

## Later (not built; the manifest leaves room for these)
- What the extension shows: native panels (above).
- A badge count on the heading (waiting questions, unread items).
- A background process Pitwall starts and stops for it (its server).
- Agent add-ons: an MCP server or plugin offered to a project's agents,
  added only when the user says so; agents keep their own behaviour.
- Notifications.
- Installing from a folder or a link (`pitwall extension install`, a
  Settings row), and watching the folder.
- Settings of its own (an address, a token), kept by Pitwall.
