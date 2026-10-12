# Detection rules

Pitwall reads each agent's terminal screen to tell whether it is working, waiting for you, or idle. What to look for is written per agent kind in a small TOML file of rules. Pitwall ships rules for the agents it supports, and you can replace them or add rules for kinds of your own. The approach and most bundled rules are adapted from [Herdr](https://github.com/herdrdev/herdr)'s detection manifests (Apache License 2.0; see NOTICE).

## Where status comes from

Three signals decide an agent's status, in this order:

1. **Hooks.** When the agent has sent a hook this run (Claude Code always, Codex once its hooks are installed), the hook wins.
2. **The screen.** Pitwall keeps a headless copy of every terminal and matches the visible screen and the window title against the kind's rules about every 400 ms. The first rule that matches decides.
3. **Activity.** When no rule matches: output in the last 1.2 s means working, otherwise unknown.

The status you see is folded from that: an agent that goes idle after working shows **done** until you look at it, and **blocked** shows as **needs you**. `pitwall agent status <agent>` shows which signal decided and what each one said last.

## Where the files live

The bundled rules are built into the app; their sources are in [`crates/pitwall-detect/detect/`](../../crates/pitwall-detect/detect/), one `<kind>.toml` per kind.

Your own go in the `detect` folder inside Pitwall's data folder, next to your [agent definitions](agents.md):

| System | Folder |
|---|---|
| macOS | `~/.pitwall/detect/` |
| Linux | `$XDG_DATA_HOME/pitwall/detect/` (default `~/.local/share/pitwall/detect/`) |
| Windows | `%APPDATA%\Pitwall\detect\` |

With `$PITWALL_HOME` set, the folder is `$PITWALL_HOME/detect/` on every system.

- The file name (without `.toml`) is the agent kind's `id`.
- A file for a kind with bundled rules replaces those rules completely.
- A file for any other kind adds rules for it, for example a kind you defined in [agent definitions](agents.md).
- A file that doesn't parse, has an unknown key, a bad regex or an unknown region is ignored with a message on Pitwall's error output, and the bundled rules for that kind stay in use.
- Rules are read once per run. Restart Pitwall after editing a file.

## A rule file

```toml
id = "drift"
version = "2026.10.12"

# Hint shown for a blocked agent when its rule has no `detail`.
detail_line = ['(?i)^\s*(allow [^\n]*\?)\s*$']

[[rules]]
id = "approval"
state = "blocked"
priority = 900
region = "bottom_non_empty_lines(6)"
contains = ["allow", "(y/n)"]

[[rules]]
id = "spinner"
state = "working"
priority = 800
region = "bottom_non_empty_lines(3)"
line_regex = ['^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] ']

[[rules]]
id = "prompt"
state = "idle"
priority = 100
region = "bottom_lines(2)"
line_regex = ['^drift> ']
not = [ { contains = ["thinking"] } ]
```

Rules are tried from the highest `priority` down, and the first one that matches decides. Here a visible approval question beats the spinner, and the spinner beats the prompt.

## File keys

| Key | Type | Meaning |
|---|---|---|
| `id` | string | Optional. A name for the file; not used to pick the kind (the file name does that). |
| `version` | string | Optional. Free text, for your own tracking. |
| `detail_line` | list of regexes | Optional. Patterns for the hint shown next to a blocked agent, used by every rule that has neither `detail` nor its own `detail_line`. |
| `rules` | list of tables | The rules, written as `[[rules]]` tables. |

## Rule keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | string | required | The rule's name, shown in error messages. |
| `state` | string | required | What a match means: `working`, `blocked`, `idle` or `unknown`. |
| `priority` | integer | `0` | Higher is tried first. Rules with equal priority keep their order in the file. |
| `region` | string | `screen` | Which part of the screen the rule looks at (see [Regions](#regions)). |
| `contains` | list of strings | `[]` | Every string must appear in the region. Ignores case. |
| `regex` | list of regexes | `[]` | Every pattern must match somewhere in the region. |
| `line_regex` | list of regexes | `[]` | Every pattern must match at least one line of the region. |
| `all` | list of gates | `[]` | Every gate must match. |
| `any` | list of gates | `[]` | At least one gate must match (when the list isn't empty). |
| `not` | list of gates | `[]` | No gate may match. |
| `detail` | string | none | A fixed hint shown next to a blocked agent ("Trust this folder?"). |
| `detail_line` | list of regexes | the file's `detail_line` | Patterns for the hint, overriding the file-level ones for this rule. |

A rule matches when all of its conditions hold. A rule with no conditions always matches.

A **gate** is an inline table with the same condition keys: `contains`, `regex`, `line_regex`, `all`, `any` and `not`. Gates can nest up to 8 levels:

```toml
any = [
  { contains = ["esc to interrupt"] },
  { all = [ { contains = ["running"] }, { not = [ { contains = ["done"] } ] } ] },
]
```

Typos are errors: an unknown key in a rule or gate makes the whole file invalid, so a misspelled key is never silently ignored.

## States

| State | Meaning |
|---|---|
| `working` | The agent is busy. |
| `blocked` | The agent waits for you: a permission prompt, a question, a trust dialog. Shown as needs you, sorted first, and notified when Pitwall isn't focused. |
| `idle` | The agent waits for a prompt. After work, this shows as done until you look. |
| `unknown` | Stops here and says nothing: Pitwall falls back to output activity. Use it for screens that don't tell the agent's status, such as a transcript viewer or a menu, so lower rules don't misread them. |

## Hints for blocked agents

A blocked agent can show a short hint, like the question it asks. With `detail` on the rule, that text is shown. Otherwise Pitwall searches the last 30 non-empty lines from the bottom up, trying each `detail_line` pattern in order, and shows the first match: capture group 1 when the pattern has one, else the whole match, cut to 160 characters.

## Regions

Rules should look at a narrow part of the screen near the bottom, where the agent's own interface sits (footers, dialog buttons, the prompt box), and never at the whole conversation. Text that the agent or you print in the conversation then can't fake a state.

Trailing blank lines are dropped from the screen before any region is taken. Counts must be 1 or more.

| Region | What it is |
|---|---|
| `screen` | The whole visible screen (also `whole_recent`). |
| `title` | The terminal's window title set by the program (OSC 0/2; also `osc_title`). The user can't type into it. |
| `bottom_lines(N)` | The last N lines. |
| `bottom_non_empty_lines(N)` | From the Nth-last non-empty line to the bottom. |
| `top_non_empty_lines(N)` | From the top to the Nth non-empty line. |
| `after_last_horizontal_rule` | The lines after the last `────` line (empty when there is none). |
| `prompt_box_body` | The lines between the last two `────` lines: Claude Code's prompt box. |
| `above_prompt_box` | Everything above that prompt box (the whole screen when there is none). |
| `above_prompt_box_bottom(N)` | The last N non-empty lines above the prompt box. |
| `last_non_empty_above_prompt_box` | The last non-empty line above the prompt box. |
| `after_last_prompt_marker` | The lines after the last `›` prompt line (Codex). |
| `before_current_prompt_marker` | The lines before Codex's current composer line. |
| `whole_recent_without_current_prompt_marker` | The whole screen, or nothing while Codex's composer is shown. |
| `current_prompt_line` | Codex's current composer line itself (empty when there is none). |

A `────` line is a line made only of `─`, or one starting with at least three `─` followed by a label.

Regexes use Rust's [regex syntax](https://docs.rs/regex/latest/regex/#syntax). Use TOML single-quoted strings (`'...'`) so backslashes stay as written, and `(?i)` to ignore case. `contains` already ignores case.

## Writing and checking rules

1. Start the agent in Pitwall and bring it into each state you care about: working, asking for approval, idle.
2. Note text that only the agent's own interface shows in that state, near the bottom of the screen: a footer like `esc to interrupt`, the buttons of a dialog, the prompt line.
3. Write one rule per state, with the most specific (blocked) at the highest priority and the plain prompt (idle) at the lowest.
4. Save the file, restart Pitwall, and run `pitwall agent status <agent>` (or `--human`) in each state. It shows what the screen rules said, which signal decided, and why.

The bundled rules are tested against real screens captured from the agents: see the fixtures and tests in [`crates/pitwall-detect/src/detect/`](../../crates/pitwall-detect/src/detect/). Contributors changing a bundled rule add a fixture for the screen it fixes and run `cargo test -p pitwall-detect`.
