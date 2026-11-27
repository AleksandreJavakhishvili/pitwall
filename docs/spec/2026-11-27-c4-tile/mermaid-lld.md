Status: built in phase A ([plan.md](plan.md)). Code: `tiles/c4/src/blocks.rs`,
`tiles/c4/src/mermaid/`, `tiles/c4/src/c4.rs`.

# C4 tile: Markdown blocks and the Mermaid subset

How the tile finds Mermaid blocks in Markdown and parses the subset of
[frd.md §6.1](frd.md#61-supported-in-v1) into a model. The C4 reading on
top of that model is in §5.

## 1. Blocks

The block finder runs `pulldown-cmark` (CommonMark, default features
off) over a Markdown file.

- A block is a fenced code block (backticks or `~~~`, at any allowed
  indent, inside lists and quotes too) whose info string's first word is
  `mermaid`, compared without case.
- **Positions.** Lines and columns are 1-based. Columns count Unicode
  characters. For every line of the block's text the finder records the
  file line and the column offset of its first character, so a problem
  at (line, column) inside the block maps to the Markdown file exactly,
  indented fences included. CRLF files map the same way; `\r` is dropped
  from the block text.
- **Heading path.** The ATX and setext headings above the block, outer
  first, each with its level, text and line.
- **Id.** `<path>#<slug>`, where the slug is the nearest heading's text
  in lowercase, letters, digits, `-` and `_` kept, spaces turned into
  `-`, everything else dropped. Under no heading the slug is `top`. The
  second and later blocks under the same heading get `-2`, `-3`, … So an
  edit elsewhere in the file keeps the id.

## 2. The type line

After an optional frontmatter (`---` … `---` at the very start; only
`title:` is read) and any blank, `%%` comment or `%%{…}%%` directive
lines, the first word of the next line is the type:

| First word | Result |
|---|---|
| `flowchart`, `flowchart-elk`, `graph` | flowchart parser (§3) |
| `sequenceDiagram` | sequence parser (§4) |
| `C4Context`, `C4Container`, `C4Component`, `C4Dynamic`, `C4Deployment` | fallback, reason "Mermaid C4 syntax" ([frd §9](frd.md#9-open-questions-for-the-owner)) |
| any other word | fallback, reason "unsupported type", with the word |
| nothing | fallback, reason "empty" |

A fallback carries the type word and, for errors or limits, the
problems. Its note, as the tile shows it: "Class diagrams aren't drawn
here" (known types have a name: class, state, entity relationship,
Gantt, pie, mindmap, timeline, user journey, git graph, quadrant,
requirement, Sankey, XY, block, packet, kanban, architecture, radar
diagrams), "‹word› diagrams aren't drawn here" for unknown words,
"Mermaid's C4 syntax isn't drawn here; write C4 diagrams as flowcharts"
for C4 words, "The block is empty", "This diagram has problems" and
"Too large to draw here".

## 3. Flowchart

### 3.1 Grammar

Statements end at a newline or `;` outside quotes. `%%` starts a comment
line. Keywords are matched without case except node ids.

```text
header    = ("flowchart" | "graph") [dir]
dir       = "TB" | "TD" | "BT" | "LR" | "RL"
statement = subgraph | "end" | "direction" dir | classdef | class
          | skipped | chain
subgraph  = "subgraph" ( id [ws] "[" text "]" | id "[" quoted "]"
                       | quoted | words )
classdef  = "classDef" name ("," name)* [rest ignored]
class     = "class" id ("," id)* name ("," name)*
skipped   = ("style" | "linkStyle" | "click" | "accTitle" | "accDescr") rest
chain     = group (link group)*
group     = vertex ("&" vertex)*
vertex    = id [shape] [":::" name ("," name)*]
id        = (letter | digit | "_" | "." | "-" followed by a letter or digit)+
shape     = open label close        (pairs below)
label     = quoted | text up to the close
link      = [start] body [end] ["|" (text | quoted) "|"]
          | "--" text ("-->" | "---" | "--x" | "--o")
          | "==" text ("==>" | "===")
          | "-." text (".->" | ".-")
start     = "<" | "x" | "o"
body      = "-"{2,} | "="{2,} | "-" "."+ "-" | "~~~"
end       = ">" | "x" | "o"    (x and o only when no id character follows)
```

Shapes, longest open first: `(((` `)))` double circle, `([` `])`
stadium, `[(` `)]` cylinder, `((` `))` circle, `{{` `}}` hexagon, `[[`
`]]` subroutine, `[/` `/]` or `\]`, `[\` `\]` or `/]`, `(` `)` round,
`[` `]` rect, `{` `}` diamond, `>` `]` flag. Double circle, subroutine
and the trapezoids are drawn as plain boxes and listed under "Not
shown"; so is the `@{ … }` shape syntax, which is skipped.

### 3.2 Labels

- `"…"` quotes allow any character, newlines included; a newline is a
  line break.
- `<br>`, `<br/>`, `<br />` (any case) are line breaks.
- Other HTML tags are removed. `&lt;` `&gt;` `&amp;` `&quot;` `&#39;`
  `&nbsp;` and Mermaid's `#quot;` style entities are decoded.
- A Markdown string (`` "`…`" ``) is drawn as plain text and listed
  under "Not shown"; so are `fa:` icons and `<img>`.
- Lines are trimmed; empty lines are dropped.

### 3.3 Model

```text
Flowchart  { direction, title, nodes, boundaries, edges, class_defs, skipped }
Node       { id, lines, shape, classes, boundary, line }
Boundary   { id, title, parent, direction, classes, line }
Edge       { from, to, label, style (solid, dotted, thick, invisible),
             start_head, end_head (none, arrow, cross, circle), line }
End        = Node(i) | Boundary(i)
Skipped    { line, what }
```

- A node is created on first mention; a later mention with a shape sets
  its label. With no shape anywhere its label is its id.
- A node belongs to the innermost subgraph in whose body it is first
  mentioned inside any subgraph (edges count as mentions). Nodes only
  ever mentioned at the top level belong to no boundary.
- An id that names a subgraph is a boundary end in edges, unless it also
  has a shape somewhere.
- `a & b --> c & d` gives four edges; `a --> b --> c` gives two.
- `class` may name boundaries as well as nodes.

### 3.4 Errors

Each error is "line L, column C: message", in Markdown file coordinates
(§1). The parser records the error, skips to the next line and goes on,
so one block can report several. Any error means the fallback.

| Message | When |
|---|---|
| `expected a direction (TB, TD, BT, LR or RL), found 'X'` | bad word after `flowchart`, `graph` or `direction` |
| `expected a node id, found 'X'` | a statement or link not followed by an id |
| `expected an arrow, '&' or the end of the line, found 'X'` | junk after a vertex |
| `unclosed label: expected 'C'` | a shape with no closing delimiter on the statement |
| `unclosed quote` | a `"` with no partner in the block |
| `unclosed edge label: expected '\|'` | `-->|text` with no closing `\|` |
| `edge label with no arrow: expected 'P'` | `-- text` with no closing arrow on the line |
| `'end' without an open subgraph` | extra `end` |
| `subgraph 'T' has no matching 'end'` | at the subgraph's line, at the end of the block |
| `expected a class name` | `classDef` or `class` with no name, `:::` with none |
| `expected node ids and a class name` | `class` with only one word |
| `nested deeper than 16 subgraphs` | limit (hla §8); also gives the fallback reason "too large" |
| `frontmatter has no closing '---'` | at line 1, for any type |

Over 300 nodes or 600 edges the fallback reason is "too large", with no
error line.

## 4. Sequence

### 4.1 Grammar

One statement per line. Keywords without case.

```text
participant = ["create"] ("participant" | "actor") id ["as" alias]
message     = id arrow ["+" | "-"] id ":" [text]
arrow       = "->>" | "-->>" | "->" | "-->" | "-x" | "--x" | "-)" | "--)"
activation  = ("activate" | "deactivate") id
note        = "note" ("left of" | "right of") id ":" text
            | "note over" id ["," id] ":" text
frame       = ("loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect") [text]
branch      = ("else" | "and" | "option") [text]
box         = "box" [colour] [text] (closed by "end", holds participants;
                                     only rgb(…), rgba(…) and transparent
                                     are dropped as colours)
other       = "end" | "autonumber" [numbers] | "title" text
skipped     = "accTitle" | "accDescr" | "link" | "links" | "properties" | "details" | "destroy"
```

A trailing `;` is dropped. Errors point at the statement's first
character unless the table says otherwise.

Ids may contain spaces and single hyphens; they are trimmed. The arrow
is the first arrow token on the line, longest first. `+` activates the
target, `-` deactivates the source. A participant is created on first
mention. `rect` is a frame drawn without its colour, listed under "Not
shown"; so is `create`.

### 4.2 Model

```text
Sequence    { title, participants, boxes, steps, autonumber, skipped }
Participant { id, label (alias or id), actor, box_group, line }
Step        = Message { from, to, text, dotted, head (none, arrow, cross, async),
                        activate_target, deactivate_source, line }
            | Note { placement (left of, right of, over), first, second, text, line }
            | Activate(p) | Deactivate(p)
            | FrameStart { kind, label } | FrameBranch { kind, label } | FrameEnd
```

### 4.3 Errors

| Message | When |
|---|---|
| `expected a participant name` | `participant`, `activate`, a note or a message side with no id |
| `expected ':' and the message text` | a message without `:` (at the end of the line) |
| `expected 'left of', 'right of' or 'over'` | a bad note placement |
| `expected ':' and the note text` | a note without `:` |
| `'else' outside 'alt'`, `'and' outside 'par'`, `'option' outside 'critical'` | misplaced branch |
| `'end' without an open frame` | extra `end` |
| `'K' has no matching 'end'` | an open frame or box at the end of the block |
| `'P' is not active` | `deactivate` or `-` on a participant that isn't active |
| `not a sequence statement: 'X'` | anything else |

Over 300 participants or 600 steps the fallback reason is "too large".

## 5. C4 reading

A flowchart becomes a **structure**: boxes, boundaries and arrows with
C4 meaning. Every flowchart gets one; plain flowcharts just have no
Kinds.

- **Box label.** The first line is `Name [Kind]` when it ends in
  `[…]`; otherwise, if the second line is exactly `[Kind]`, that is the
  Kind and the purpose starts on line three (agents drift into that
  form). The remaining lines, joined with a space, are the purpose. Line
  breaks come from §3.2, so `<br/>` (the canonical form in docs),
  `<br>`, `<br />` and a newline inside a quoted label all work
  ([frd §9](frd.md#9-open-questions-for-the-owner) decision 1).
- **Kind.** Its base is the text before an optional `:` (as in
  `[Container: Rust]`), lowercased; the rest is the technology.
- **Shape.** From the words of the whole Kind, technology included:
  `person` or `actor` → person; `database`, `store` or `db` → cylinder;
  `queue` → pipe (so `[Container: Queue]` is a pipe). Otherwise from the
  Mermaid shape: `[(…)]` → cylinder. Otherwise a rounded box.
- **Classes.** `new`, `changed`, `external` have meaning; others are
  kept by name. A box is external when it has the class or its Kind
  starts with `external`.
- **Boundary.** The subgraph title read as `Name [Kind]`.
- **Name key.** `name(s)` = the Name without `[Kind]`, spaces collapsed,
  trimmed, lowercased. Zoom links and the Changes view compare keys.
- **Level** (FR-11). The frontmatter title, then the heading path from
  the nearest heading outwards: the first that has a word starting with
  `context`, `container` or `component` decides. Otherwise: any box with
  base `component` (or ending in ` component`) → component; any with
  `container` → container; else context.
- **Before/after headings** (frd §3.4, decision 2): "Before"/"After",
  "Current"/"Proposed", "As is"/"To be", "Today"/"Target", without case,
  after an optional section number, as the whole title or followed by a
  non-letter. The word list is one constant in `src/index.rs`.

Arrows carry no classes in v1: Mermaid only classes edges that have an
id (`e1@-->`), which is outside the subset. So FR-16 highlights boxes;
arrows stand out only in the Changes view (FR-17).
