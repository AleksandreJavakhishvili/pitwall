Status: built in phase A ([plan.md](plan.md)), except the sequence
layout (§5, phase B). Code: `tiles/c4/src/text.rs`, `tiles/c4/src/layout/`.

# C4 tile: sizing and layout

How a structure diagram ([mermaid-lld.md §5](mermaid-lld.md#5-c4-reading))
gets sizes and positions, behind a `Layout` trait so the engine can be
swapped ([hla §3.5](hla.md#35-layout)).

## 1. Text widths

`build.rs` reads `assets/fonts/Inter-Regular.ttf` and
`Inter-SemiBold.ttf` (copies of the fonts Pitwall ships, OFL-1.1, with
the licence beside them) with `rustybuzz`, a build-dependency only, and
writes per weight:
- the advance, in font units, of printable ASCII, Latin-1 and the
  punctuation labels use (dashes, curly quotes, `…`, `•`, `‹ ›`, `€`,
  `™`);
- the kerning of every pair of printable ASCII and that punctuation,
  measured by shaping the pair (about 1 600 non-zero pairs per weight).

Other characters count as the average lowercase advance. Pitwall's Inter
has no arrows and no `⌘`, so the host draws those with a fallback font
and the table can only guess them. Advances scale linearly with size, so
one table serves every size. Without kerning the table was up to 4 % off
("WAVE AVATAR"); with it, `tests/text_widths.rs` shapes 100 labels (the
fixtures' Names, Kinds, purposes and arrow labels, plus hand-written
ones) with `rustybuzz` and the table is within 0.36 % of the font for
both weights.

`text_width(text, px, weight)` sums advances and pair kerning. The Sizer
adds 8 % slack (hla §3.7) on top.

Gap G2 is closed: the SDK measures text (`ui.measure-text`, from the
app's own font files by the same method). The table stays all the same:
the core (parse, layout) builds and is tested without the SDK
(`--no-default-features`, the speed numbers below), and a layout measures
thousands of strings while it wraps, where host calls are counted (200 a
second). `tests/text_widths.rs` checks the table against Pitwall's
measure for the 100 labels at both weights: equal, so a change to
Pitwall's fonts fails there rather than drawing cut labels.

## 2. Sizes

Pixel sizes stand in for the canvas text tokens until gap G3 is settled.

| Part | Text | Size |
|---|---|---|
| Box | Name, SemiBold 14 px, line 18 px; `[Kind]`, Regular 12 px, line 16 px; purpose, Regular 12 px, line 16 px, wrapped at word boundaries to at most 2 lines, the second cut with `…` | padding 12 px across, 10 px down; width = widest line × 1.08 + 24, clamped to 120–240 px |
| Person | as a box | + 24 px on top for the head |
| Cylinder | as a box | + 12 px for the top ellipse |
| Pipe | as a box | + 16 px across for the ends |
| Arrow label | Regular 12 px, one line, capped at 200 px | width × 1.08 + 8, height 18 px |
| Boundary | title `Name [Kind]`, SemiBold 13 px | title band 24 px, padding 16 px |

## 3. The graph and the trait

```text
LayoutGraph { direction, nodes: [ {w, h, parent} ], clusters: [ {parent, title_w} ],
              edges: [ {from, to, label: Option<(w, h)>} ] }
End         = Node(i) | Cluster(i)
Placed      { nodes: [Rect], clusters: [Rect], edges: [ {points, label: Option<Rect>} ],
              width, height }
trait Layout { fn name(&self) -> &str; fn layout(&self, &LayoutGraph) -> Placed }
```

Rects are top-left based, in pixels, the drawing starting at (0, 0).
`build(structure)` makes the graph: a node per box, a cluster per
boundary (nested), an edge per arrow. Layout runs only after a parse or
a metrics change, never on pan or zoom (hla §3.5 step 5); caching is the
caller's.

## 4. dagre (v1)

- A compound graph (`directed`, `multigraph`, `compound`). Each box a
  node with its size; each boundary a parent node; each arrow an edge
  named by its index, so parallel arrows stay apart, with the label's
  size (`labelpos` centre).
- An arrow to a boundary goes to a hidden 1 × 1 anchor node inside it
  (one per boundary); the boundary's frame grows to hold the anchor, and
  after layout the edge's points are cut where they enter the frame. A
  boundary with nothing inside is an ordinary node as big as its title.
- Options: `rankdir` from the diagram (`TD` = `TB`); `nodesep` 40,
  `ranksep` 60, `edgesep` 20, margins 16; network simplex ranking.
- dagre gives a parent node only the space its children take. After
  layout each boundary's rect grows by its padding, plus the title band
  on top, innermost first, so nested frames stay apart; `ranksep` and
  `nodesep` leave room for that.
- Edge points come out with their ends on the box borders. The Painter
  draws them as a smooth curve through the points.

## 5. Sequence layout (phase B)

No graph layout: participants left to right in order of first mention
(boxes from `box` keep their members together), each column as wide as
its header or its widest neighbouring message label, plus 24 px. One row
per step: 32 px for a message (plus 16 px per extra label line), a note
as tall as its text, 24 px for a frame start or branch, 8 px for a frame
end. Frames are rects from their start row to their end row, across the
participants their steps touch, nested 8 px inwards per level.
Activations are 10 px bars from activate to deactivate.

## 6. Limits and checks

- Over 300 boxes or 600 arrows, or subgraphs nested deeper than 16, a
  diagram gets the fallback before layout (FR-21, hla §8).
- Tests lay out every fixture and check: no two boxes overlap, every box
  is inside its boundary, every nested boundary is inside its parent,
  boundaries not nested in each other don't overlap, and no box overlaps
  a boundary it isn't in (`layout::check`).
- How long layout takes in Wasmtime is measured in phase B (NFR-2); the
  300-box limit moves to match. Native numbers are in §7.
- **Measured in Wasmtime** (phase B, the release `tile.wasm` in the
  harness on an Apple M5 Max; one `c4__show` each: parse, C4 reading,
  layout, the reply and the first draw; boundaries of ten boxes with a
  chain of labelled arrows in each; `cargo test --release --test tile
  speed -- --ignored --nocapture` in `tiles/c4`): 50 boxes 24 ms, 150
  boxes 76 ms, 300 boxes 164 ms. NFR-2's 100 ms for 50 boxes holds with
  room; 300 boxes is well inside a tool call's 10 s, so **the limit stays
  at 300 boxes** (FR-21). Pan and zoom don't lay out again: they redraw
  the kept drawing under a new transform.

## 7. dagre against rust-sugiyama

From `tests/compare_layouts.rs` (`cargo test --release --features
sugiyama --test compare_layouts -- --nocapture`), natively on an M-series
Mac, 2026-11-27. The `sugiyama` feature keeps the rust-sugiyama engine
(`src/layout/sugiyama.rs`) out of the default build. It has no compound
graphs, so it lays out each boundary's inside first and then the
boundary as one big node one level up; it returns no edge routes, so
edges are straight lines between box borders with the label at the
middle.

Quality, summed over the 22 structure diagrams of the fixtures:

| Engine | Layout check failures (§6) | Edge crossings | Edges through other boxes | Labels on boxes | Area |
|---|---|---|---|---|---|
| dagre | 0 | 7 | 3 | 0 | 9.4 Mpx² |
| rust-sugiyama | 0 | 8 | 21 | 35 | 6.9 Mpx² |

Time, release build, one layout:

| Engine | All 22 fixtures | 50 boxes | 150 boxes | 300 boxes |
|---|---|---|---|---|
| dagre | 18 ms | 15 ms | 55 ms | 171 ms |
| rust-sugiyama | 0.2 ms | 0.1 ms | 0.3 ms | 0.7 ms |

(The 50 to 300 box graphs are synthetic: boundaries of 10 boxes, chains
inside, links between neighbouring boundaries, a label on every third
arrow.)

**v1 uses dagre.** It routes edges around boxes and gives labels their
own space; rust-sugiyama's drawings are smaller but run arrows through
boxes and put labels on them, and fixing that means writing edge routing
and label placement ourselves. rust-sugiyama is two orders of magnitude
faster, because each boundary is a small separate problem. dagre's
300-box time is over the SDK's 50 ms warning natively and will be slower
in Wasmtime: phase B measures it there (NFR-2), and if the limit has to
drop far, laying out boundaries separately (as the rust-sugiyama wrapper
does) is the fallback to try with dagre too.
