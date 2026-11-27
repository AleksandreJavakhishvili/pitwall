Status: design, not built. Started 2026-11-27.

# C4 diagram tile: functional requirements

The C4 diagram tile is Pitwall's flagship seed tile
([Tile SDK §10.1](../tile-sdk/README.md#101-seed-tiles-first-party-public-sdk-only)).
It shows the C4 diagrams that coding agents write into a project's
Markdown docs, drawn natively, next to the agents. Later, agents use its
tools to create and change those diagrams.

It is a third-party tile in every sense but its author. It is built only
on the public Tile SDK, so every gap it hits is a gap in the SDK.

Related documents:
- [hla.md](hla.md): the architecture, with its own C4 diagrams.
- [plan.md](plan.md): phases, checkboxes and definitions of done.
- [Tile SDK design](../tile-sdk/README.md): the platform it is built on.

Marks used here: **[SDK]** is a fact from the Tile SDK design (not
built yet either). *Guess* marks something this spec assumes and has not
checked. *Proposal* marks a choice this spec makes that the owner may
change.

## 1. Why

Pitwall users run coding agents that write spec-driven development
(SDD) docs: `frd.md`, `hla.md`, `*-lld.md` and `plan.md` in a feature
folder (`.claude/skills/sdd/SKILL.md`). In the owner's variant of that
skill, the architecture is drawn as C4 diagrams in Mermaid. Today the
user reads them as source text in an editor, or opens a browser preview
that sits apart from the agents. Neither shows how the levels connect,
or what an agent's change adds.

The tile answers four questions at a glance, inside Pitwall:
1. What does the system look like? (Draw the diagram.)
2. What is inside this box? (Click it to go one level down.)
3. What does this change add or alter? (New and changed parts stand out;
   before and after side by side when the doc has both.)
4. Did the agent just change it? (The tile follows the files.)

## 2. Personas

| Persona | Who | What they want from the tile |
|---|---|---|
| The engineer | A Pitwall user who runs several agents on a project and reviews their design docs | Read the architecture fast, see what a proposed change touches, jump to the source line |
| The agent | A coding agent Pitwall started (Claude Code, Codex, …) | v2: list and read the project's diagrams; show a proposed change to the user; write it into the doc after approval; export SVG |
| The tile author | Pitwall's own team, writing the tile on the public SDK | Prove that canvas, files and agent tools are enough for a real tile; find the gaps before WIT 1.0 |

## 3. The input format

The tile reads the C4 convention of the owner's SDD skill variant. This
section describes that convention as the tile's input. The convention
itself is not in this repository, so its details here come from the
owner's description (2026-11-27). Points marked *guess* need the owner
to confirm.

### 3.1 Where diagrams live

- Fenced code blocks with the info string `mermaid` in Markdown files.
- Default places to look, relative to the project root:
  `docs/**/*.md`, `**/hla.md`, `**/*-lld.md`. The user can change the
  list in the tile's settings.
- Several diagrams per file are normal. The heading above a block names
  it.

### 3.2 C4 structure diagrams (context, container, component)

- A Mermaid `flowchart` (or `graph`).
- **One `subgraph` per boundary.** The subgraph's title names the
  system or container that the boxes inside belong to.
- **Boxes are labelled `Name [Kind]` plus a one-line purpose.** For
  example `Scanner [Component]` and "finds Mermaid blocks in Markdown".
  The purpose sits on a second line of the label. The tile accepts
  `<br/>`, `<br>` and a line break inside a quoted label as the
  separator (*guess*: the exact separator the skill writes).
- **Arrows are labelled** with what flows or what is called:
  `A -->|reads files| B` or `A -- reads files --> B`.
- **`classDef new`, `classDef changed`, `classDef external`** mark boxes
  that a change adds, alters, or that sit outside the system. Classes
  are applied with `class a,b new` or `a:::new`.
- The level (context, container, component) is not written in the
  diagram. The tile infers it ([§5.3](#53-levels-and-zoom-v1)).

A container diagram in this convention looks like this:

```mermaid
flowchart LR
  user["Engineer [Person]<br/>reviews agents' docs"]
  subgraph pitwall["Pitwall [Software System]"]
    app["pitwall-app [Container]<br/>draws tiles with GPUI"]
    daemon["pitwalld [Container]<br/>runs agents"]
    cache["Cache [Container]<br/>keeps parsed docs"]
  end
  repo["Project repo [External System]<br/>Markdown docs"]
  user -->|reads diagrams| app
  app -->|asks for state| daemon
  app -->|reads| cache
  daemon -->|reads files| repo
  classDef new stroke-dasharray: 4 2
  classDef external fill:#eee
  class cache new
  class repo external
```

### 3.3 Sequence diagrams

- A Mermaid `sequenceDiagram`, used for order-critical flows.
- Participants are usually the same names as boxes in the structure
  diagrams (*guess*: the skill asks for this, but agents may drift).

### 3.4 Before and after

A change may show the system before and after it. The tile pairs two
diagrams as before and after when they sit under sibling headings whose
words say so: "Before" / "After", "Current" / "Proposed", "As is" /
"To be", "Today" / "Target" (*proposal*; the skill variant may use other
words, to confirm). Without a pair, the `new` and `changed` classes
alone show the change.

## 4. User stories

1. **Open.** I open "C4 diagrams" from the palette. The tile asks which
   project it may read. I pick `my-app`. It lists the diagrams it found,
   grouped by file, and draws the first context diagram.
2. **Zoom in.** I click the box "Pitwall [Software System]". The tile
   draws the container diagram whose boundary is "Pitwall". The
   breadcrumb reads "Context › Pitwall". I click "pitwall-app" and get
   its component diagram. Backspace (or the breadcrumb) goes back up.
3. **See the change.** An agent wrote a new LLD with a container diagram
   where "Cache" is `new` and "pitwalld" is `changed`. The tile draws
   Cache in the "new" colour and pitwalld in the "changed" colour, with
   a legend.
4. **Before and after.** The LLD has "Before" and "After" sections, each
   with a diagram. The tile shows a Before | After | Changes switch.
   "Changes" draws the after diagram with removed boxes as faint dashed
   ghosts and added ones highlighted.
5. **Follow the agent.** The agent edits `hla.md` while I watch. Within
   a second of the save, the tile redraws. If the new text doesn't parse,
   the tile keeps the last good drawing, marks it "out of date", and
   shows the error line.
6. **Jump to source.** I click "Open source" on a diagram. Pitwall's
   editor opens the Markdown file at the block's first line.
7. **Unsupported diagram.** A doc has a `classDiagram`. The tile lists
   it, and when selected, shows "Class diagrams aren't drawn here" with
   the source in a read-only code view and "Open source".
8. **(v2) Ask an agent.** I tell my agent "add the cache container". It
   reads the container diagram through the tile, sends a proposal, and
   the tile shows it over the current diagram with "from api" and Undo.
   I press Apply, or the agent asks to write it; Pitwall asks me first,
   showing the exact change to the file.
9. **(v2) Export.** I ask for an SVG of the context diagram in
   `docs/img/`. Pitwall asks me first; the file appears.

## 5. Functional requirements

### 5.1 Finding diagrams (v1)

- **FR-1.** The tile reads only projects the user granted it under
  `files.read` [SDK]. It never asks for more.
- **FR-2.** It finds Markdown files matching the path list (§3.1),
  skipping `.git/`, `node_modules/`, `target/` and hidden folders. Limits
  (*proposal*): 2 000 Markdown files and 1 MiB per file; past a limit it
  says so and shows what it has.
- **FR-3.** In each file it finds fenced `mermaid` blocks with their line
  numbers and the heading path above them.
- **FR-4.** It follows changes: a created, changed, renamed or deleted
  file updates the list and the drawing within 1 s of the change while
  the tile is shown (*proposal*). While hidden, the tile is suspended
  [SDK]; it checks for changes when shown again.

### 5.2 Drawing (v1)

- **FR-5.** It draws supported diagrams (§6) on the SDK canvas [SDK],
  laid out automatically. Boxes show Name, Kind and purpose. Shapes
  follow the Kind: Person as a person, Database or Store as a cylinder,
  Queue as a pipe, the rest as rounded boxes (*proposal*).
- **FR-6.** Boundaries are drawn as titled dashed frames around their
  boxes, nested as the subgraphs are.
- **FR-7.** Arrows are drawn with their labels, in the direction the
  diagram sets (`TB`, `TD`, `BT`, `LR`, `RL`).
- **FR-8.** Colours come from Pitwall's theme tokens, so the drawing
  follows Dark, Light, Flat and Glass [SDK]. `new` uses the "good" tones,
  `changed` the "warn" tones, `external` the muted ones. Colours written
  in `classDef` are ignored on purpose, so every diagram reads the same
  way (*proposal*).
- **FR-9.** Pan by dragging. Zoom with ⌘+ / ⌘− and fit with ⌘0 (and the
  pointer wheel or a pinch once the SDK delivers them,
  [hla §7](hla.md#7-sdk-gaps)). A new diagram
  opens fitted to the tile.
- **FR-10.** A legend shows the classes the diagram uses.

### 5.3 Levels and zoom (v1)

- **FR-11.** The tile infers each structure diagram's level: a heading or
  title that says "context", "container" or "component" wins; otherwise
  the Kinds decide (any `[Component]` → component; any `[Container]` →
  container; else context) (*proposal*).
- **FR-12.** A box is "detailed" by another diagram when that diagram
  has a boundary (subgraph) whose title is the box's Name (compared
  without the `[Kind]` part, case and extra spaces). Detailed boxes show
  a small zoom mark.
- **FR-13.** Clicking a detailed box opens the diagram that details it.
  When several diagrams detail it, the tile prefers one in the same file,
  then the same folder; if still more than one, it shows a menu.
- **FR-14.** A breadcrumb shows the path from the top. Each crumb goes
  back to its level. Backspace goes up one level.
- **FR-15.** Clicking a box that is not detailed selects it. The footer
  shows its Name, Kind, purpose, its class, and the sequence diagrams in
  which a participant has the same Name. Clicking one opens it.

### 5.4 Change views (v1)

- **FR-16.** Boxes and arrows with `new` or `changed` are highlighted
  (FR-8).
- **FR-17.** For a before/after pair (§3.4), a switch offers Before,
  After and Changes. Changes draws the after diagram, plus boxes only in
  before as dashed ghosts, and highlights boxes only in after even
  without a class. Boxes match by Name; arrows by both ends' Names and
  the label.

### 5.5 Problems and fallbacks (v1)

- **FR-18.** A block of an unsupported diagram type is listed, with its
  type, and shows the fallback: a short note, the source in a read-only
  code view [SDK], and "Open source".
- **FR-19.** A supported block that fails to parse shows the same
  fallback with each problem as "line L, column C: message", the line
  counted in the Markdown file.
- **FR-20.** Parts of a supported diagram that the tile doesn't
  understand (a `click` line, `linkStyle`, an unknown shape) are skipped
  or drawn as plain boxes, and listed under "Not shown" in the footer.
  The rest still draws.
- **FR-21.** A diagram over the size limit (*proposal*: 300 boxes or
  600 arrows) shows the fallback with "too large to draw here".
- **FR-22.** After a file change that breaks a diagram already drawn,
  the tile keeps the last good drawing, marked "out of date", with the
  problems listed.

### 5.6 Navigation and state (v1)

- **FR-23.** An outline lists files, and the diagrams in each, with
  their level or type. It is keyboard friendly: ↑ ↓ move, Enter opens.
- **FR-24.** "Open source" opens the file at the block's line in
  Pitwall's editor (`viewer.open`, `show` permission [SDK]).
- **FR-25.** The tile remembers the project, the open diagram, the zoom
  path and pan per diagram across restarts (own storage [SDK]).
- **FR-26.** Keyboard on the canvas: arrows move the selection between
  boxes, Enter zooms into a detailed box, Backspace goes up, `f` fits,
  `b` cycles Before / After / Changes. No key that Pitwall reserves
  [SDK].

### 5.7 Agent tools (v2)

Kinds are the SDK's four ([agent-tools §11.2](../tile-sdk/agent-tools.md#112-four-kinds)).

- **FR-27.** `show`: open the tile at a given diagram.
- **FR-28.** `read`: list the project's diagrams (file, line, title,
  level or type, boundaries, which box each details, problems).
- **FR-29.** `read`: get one diagram: its Mermaid text, the boxes (Name,
  Kind, purpose, class, boundary), the arrows and the problems.
- **FR-30.** `present`: propose a new or changed diagram for a block in a
  file, or a new block under a heading. The tile shows it over the
  current one as a change view, marked with the agent and with Undo
  [SDK]. Nothing is written. Input with errors changes nothing and
  returns the problems.
- **FR-31.** `act`: apply a proposal to the Markdown file. The approval
  dialog shows the file and the exact text change [SDK]. If the file
  changed since the proposal, it refuses and says so. The user can also
  press Apply on a proposal in the tile, which writes with the tile's own
  `files.write` grant.
- **FR-32.** `act`: export a diagram as an SVG file into the project.
  The SVG uses fixed colours from the current theme (*proposal*), since a
  file can't follow Pitwall's theme.
- **FR-33.** A skill (`agents/SKILL.md`) tells agents the convention
  (§3) and when to use each tool [SDK]. Export is also a palette
  command and a CLI command.

## 6. Mermaid support

### 6.1 Supported in v1

| Area | Supported |
|---|---|
| Diagram types | `flowchart` and `graph`; `sequenceDiagram` |
| Flowchart direction | `TB`, `TD`, `BT`, `LR`, `RL`; `direction` inside a subgraph is read but the whole diagram uses one direction (*proposal*: dagre lays out one direction) |
| Nodes | ids; labels in `[...]`, `(...)`, `([...])`, `[(...)]`, `((...))`, `{...}`, `{{...}}`, `[/.../]`, `[\...\]`, `>...]`; quoted labels; `<br/>` and line breaks; the shape is a hint, the Kind decides (FR-5) |
| Edges | `-->`, `---`, `-.->`, `-.-`, `==>`, `===`, `--x`, `--o`, `<-->`; labels as `-->|text|` and `-- text -->`; chains `a --> b --> c`; `a & b --> c` |
| Subgraphs | `subgraph id [title]`, `subgraph id["title"]`, `subgraph title`; nesting; edges to a subgraph id |
| Classes | `classDef`, `class a,b name`, `a:::name`; `new`, `changed`, `external` have meaning, others are kept as names |
| Ignored, listed under "Not shown" | `style`, `linkStyle`, `click`, `%%{init}%%`, icons and images in labels, Markdown strings in labels (drawn as plain text) |
| Comments | `%%` lines |
| Frontmatter | `---\ntitle: …\n---` gives the title |
| Sequence | `participant` and `actor`, with `as` aliases; messages `->`, `-->`, `->>`, `-->>`, `-x`, `--x`, `-)`, `--)`; `+` / `-` and `activate` / `deactivate`; `Note left of / right of / over`; `loop`, `alt` / `else`, `opt`, `par` / `and`, `critical` / `option`, `break` frames; `autonumber`; `box` groups as frames |

### 6.2 Not supported in v1

- Every other diagram type (class, state, entity relationship, Gantt,
  pie, mindmap, timeline, …) gets the fallback (FR-18).
- Mermaid's own C4 syntax (`C4Context`, `C4Container`, …) gets the
  fallback too. The owner's convention uses flowcharts, and Mermaid
  calls that syntax experimental (*guess*: check before deciding).
  Supporting it later means mapping its elements onto the same model.
- Hand placement of boxes. The layout is always automatic.

## 7. Non-functional requirements

- **NFR-1. Native look.** Fonts, colours and focus come from Pitwall
  [SDK]; nothing looks like a web page.
- **NFR-2. Speed** (*proposal*, to measure in the plan): a 50-box diagram
  is parsed, laid out and drawn within 100 ms of opening it; a redraw
  after pan or zoom fits in the SDK's 8 ms budget for `draw`
  ([runtime §2.5](../tile-sdk/runtime.md#25-sandbox-and-limits-summary-details-in-5)).
- **NFR-3. Least privilege.** v1 asks only for `files.read` on chosen
  projects and `show`. v2 adds `files.write`, which starts off [SDK].
- **NFR-4. No network.** The tile never asks for `net`.
- **NFR-5. Never writes in v1.** v1 changes no file in the project.
- **NFR-6. Accessible outline.** Everything the canvas shows can also be
  reached from the outline and the footer by keyboard, since a canvas has
  no accessible structure of its own.
- **NFR-7. Platforms.** macOS, Linux and Windows, one `tile.wasm` [SDK].

## 8. Out of scope

- Editing diagrams by hand in the tile (drag boxes, rename). Later, if
  users ask. The source stays the Markdown file.
- Diagram types other than those in §6.1.
- Git history views (the diagram as of a commit). They would need
  `git.read` [SDK, on demand].
- Following the project of the active space or focused agent. That needs
  `projects.read` or the focused-agent read [SDK, on demand]. v1 shows
  the project the user picked, with a switcher among granted projects.
- PNG export.

## 9. Open questions for the owner

1. The exact label form for the purpose line (§3.2).
2. The heading words that mark before and after (§3.4).
3. Whether the tile should also draw Mermaid's own `C4Context` syntax
   (§6.2).
4. The extension id. This spec uses `c4`, so agents see tools such as
   `c4__get_diagram` ([agent-tools §11.4](../tile-sdk/agent-tools.md#114-how-agents-reach-them)).
