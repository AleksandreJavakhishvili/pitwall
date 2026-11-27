# Tile SDK diagrams

The Mermaid blocks of `docs/spec/tile-sdk/` (runtime.md, packaging.md,
extension-model.md) under the headings they sit under there, copied on
2026-11-27 as test input. They are plain flowcharts, not C4.

## runtime.md

### 2.1 Where it fits

```mermaid
flowchart LR
  subgraph App["pitwall-app (GPUI, main thread)"]
    TT["tiles::TileView<br/>(a tile of the layout)"]
    R["tiles::render<br/>tree → GPUI elements"]
    K["kit (button, input, list, …)"]
  end
  subgraph Host["pitwall-tile-host (no GPUI)"]
    W["worker thread per tile<br/>Wasmtime Store + Instance"]
    API["host APIs<br/>(permission-checked)"]
    C["compile cache"]
  end
  subgraph Core["pitwall-core"]
    M["extensions::{Manifest, install,<br/>permissions, activity}"]
    P["extensions::package<br/>(.pwtile, signatures)"]
    T["tile::model<br/>(element tree, limits, diff)"]
  end
  D["in-process daemon<br/>METHODS table"]
  TT -- "event (click, key, …)" --> W
  W -- "tree / patch" --> TT
  TT --> R --> K
  W --> API --> D
  API -- "UI requests" --> TT
  W --> C
  Host --> T
  Host --> M
  App --> P
```

### 2.3 Lifecycle

```mermaid
stateDiagram-v2
  [*] --> Installed: install / update (files only)
  Installed --> Compiled: turned on → compile or load cache
  Compiled --> Running: a tile opens → instantiate, init
  Running --> Suspended: no tile shows it for 60 s
  Suspended --> Running: shown again (events resume)
  Suspended --> Dropped: hidden 10 min / memory pressure → save-state
  Dropped --> Running: shown again → instantiate, restore
  Running --> Stopped: trap, hang, over limits
  Stopped --> Running: Restart (user)
  Running --> Installed: turned off (instance dropped at once)
```

### 2.4 Threading with GPUI

```mermaid
sequenceDiagram
  participant U as User
  participant V as TileView (main thread)
  participant W as Tile worker
  participant G as Guest (wasm)
  U->>V: click on "Re-run"
  V->>W: Event{handler: 17, click} (channel, no wait)
  W->>G: handle(event) [deadline 1 s]
  G->>W: host call: http.send(...) → task 4
  G-->>W: returns "changed"
  W->>G: render(view) [deadline 250 ms]
  G-->>W: tree
  W->>V: Frame{tree} (via cx.spawn → update)
  V->>V: diff, notify, draw
  Note over W,G: later: http answer → handle(task-done 4) → render
  G->>W: host call: show.open-web(url)
  W->>V: UiRequest::OpenWeb (applied with cx.defer)
```

## packaging.md

### 6.3 Install

```mermaid
flowchart TD
  A["Browse: Install / file / URL / CLI"] --> B["Download to a hidden staging folder<br/>(https only, ≤ 50 MiB)"]
  B --> C{"SHA-256 matches the index?<br/>(file/URL: skipped)"}
  C -- no --> X["Refuse: 'The file isn't the one listed'"]
  C -- yes --> D{"Signature by a trusted key?"}
  D -- no --> X2["Refuse, or for an unlisted file:<br/>show the key, ask to trust it"]
  D -- yes --> E["Unpack with the rules (6.1)<br/>check wasm imports"]
  E --> F["Review: name, author, Verified or not,<br/>permissions with reasons, what it adds"]
  F -- Install --> G["Move into &lt;data&gt;/extensions/&lt;id&gt;/ atomically<br/>marker: origin, version, sha256, key"]
  G --> H["Off until turned on → permission screen → compile → run"]
```

## extension-model.md

### 15.3 What an extension can contain

```mermaid
flowchart TB
  E["Extension (.pwtile package)<br/>extension.json: id, version, source,<br/>signature, permissions"]
  E --> D["Declarations (no code)"]
  E --> C["Component (extension.wasm)<br/>sandboxed, in the app"]
  E --> S["Service (optional)"]
  D --> D1["Settings and secrets"]
  D --> D2["Agent skill (agents/SKILL.md)"]
  D --> D3["Web views (a URL)"]
  D --> D4["Setup actions and agent variables<br/>(confirmed native commands)"]
  C --> C1["Tiles (views)"]
  C --> C2["Agent tools and commands"]
  C --> C3["Badges, notifications,<br/>floating-control questions, inbox"]
  C --> C4["Next up, reads, by grant"]
  S --> S1["Managed native program<br/>'Runs a program on your computer'"]
  S --> S2["Wasm service<br/>(own host process)"]
```
