// Pure tiling model for a space: a tree of splits whose leaves are panes.
// Every function returns a new tree; inputs are never mutated.

export type Dir = "row" | "col"; // row = side by side, col = stacked
export type Side = "left" | "right" | "top" | "bottom";
/** Fixed tilings; "3" is 1 big + 2 stacked, "CxR" is C columns × R rows. */
export type GridPreset = "1" | "2" | "3" | "2x2" | "3x2" | "3x3" | "4x3" | "4x4";
/** "auto" fits all of a space's agents as evenly as possible (see autoGrid). */
export type Preset = GridPreset | "auto";

export interface PaneNode {
  type: "pane";
  id: string;
  agentId: string | null;
}
export interface SplitNode {
  type: "split";
  id: string;
  dir: Dir;
  children: LayoutNode[];
  /** Fractions, same length as children, summing to 1. */
  sizes: number[];
}
export type LayoutNode = PaneNode | SplitNode;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type IdGen = (prefix: string) => string;

let counter = 0;
export const defaultIdGen: IdGen = (prefix) =>
  `${prefix}-${Date.now().toString(36)}${(counter++).toString(36)}${Math.random().toString(36).slice(2, 5)}`;

export const PRESETS: Preset[] = ["1", "2", "3", "2x2", "3x2", "3x3", "4x3", "4x4", "auto"];
export const PRESET_COUNT: Record<GridPreset, number> = {
  "1": 1,
  "2": 2,
  "3": 3,
  "2x2": 4,
  "3x2": 6,
  "3x3": 9,
  "4x3": 12,
  "4x4": 16,
};

export const PRESET_LABEL: Record<Preset, string> = {
  "1": "1",
  "2": "2",
  "3": "1 big + 2",
  "2x2": "2×2",
  "3x2": "3×2",
  "3x3": "3×3",
  "4x3": "4×3",
  "4x4": "4×4",
  auto: "Auto grid",
};

// ── queries ────────────────────────────────────────────────────────────────

export function leaves(node: LayoutNode | null): PaneNode[] {
  if (!node) return [];
  if (node.type === "pane") return [node];
  return node.children.flatMap(leaves);
}

export function findPane(node: LayoutNode | null, paneId: string): PaneNode | null {
  return leaves(node).find((p) => p.id === paneId) ?? null;
}

export function findPaneByAgent(node: LayoutNode | null, agentId: string): PaneNode | null {
  return leaves(node).find((p) => p.agentId === agentId) ?? null;
}

export function agentsIn(node: LayoutNode | null): string[] {
  return leaves(node)
    .map((p) => p.agentId)
    .filter((a): a is string => !!a);
}

// ── edits ──────────────────────────────────────────────────────────────────

function mapPanes(node: LayoutNode, fn: (p: PaneNode) => PaneNode): LayoutNode {
  if (node.type === "pane") return fn(node);
  return { ...node, children: node.children.map((c) => mapPanes(c, fn)) };
}

/** Put `agentId` in pane `paneId`; any other pane showing that agent is emptied. */
export function setPaneAgent(node: LayoutNode, paneId: string, agentId: string | null): LayoutNode {
  return mapPanes(node, (p) => {
    if (p.id === paneId) return { ...p, agentId };
    if (agentId && p.agentId === agentId) return { ...p, agentId: null };
    return p;
  });
}

/** Swap the agents of two panes. */
export function swapPanes(node: LayoutNode, a: string, b: string): LayoutNode {
  const pa = findPane(node, a);
  const pb = findPane(node, b);
  if (!pa || !pb) return node;
  return mapPanes(node, (p) =>
    p.id === a ? { ...p, agentId: pb.agentId } : p.id === b ? { ...p, agentId: pa.agentId } : p,
  );
}

function normalizeSizes(sizes: number[]): number[] {
  const total = sizes.reduce((s, x) => s + x, 0);
  if (total <= 0) return sizes.map(() => 1 / sizes.length);
  return sizes.map((x) => x / total);
}

/** Collapse one-child splits and merge nested splits of the same direction. */
export function normalize(node: LayoutNode | null): LayoutNode | null {
  if (!node || node.type === "pane") return node;
  const children: LayoutNode[] = [];
  const sizes: number[] = [];
  node.children.forEach((c, i) => {
    const n = normalize(c);
    if (!n) return;
    if (n.type === "split" && n.dir === node.dir) {
      n.children.forEach((cc, j) => {
        children.push(cc);
        sizes.push(node.sizes[i] * n.sizes[j]);
      });
    } else {
      children.push(n);
      sizes.push(node.sizes[i]);
    }
  });
  if (children.length === 0) return null;
  if (children.length === 1) return children[0];
  return { ...node, children, sizes: normalizeSizes(sizes) };
}

/**
 * Split pane `paneId` by adding a new pane holding `agentId` on `side`.
 * The agent is removed from any other pane first (an agent lives in one pane).
 */
export function splitPane(
  node: LayoutNode,
  paneId: string,
  side: Side,
  agentId: string | null,
  idGen: IdGen = defaultIdGen,
): { tree: LayoutNode; paneId: string } {
  let tree = node;
  if (agentId && findPaneByAgent(tree, agentId)) {
    const holder = findPaneByAgent(tree, agentId)!;
    if (holder.id === paneId) return { tree, paneId };
    tree = removePane(tree, holder.id) ?? { type: "pane", id: idGen("pane"), agentId: null };
    if (!findPane(tree, paneId)) {
      // The target was the only remaining pane's replacement; just place the agent.
      const only = leaves(tree)[0];
      return { tree: setPaneAgent(tree, only.id, agentId), paneId: only.id };
    }
  }
  const dir: Dir = side === "left" || side === "right" ? "row" : "col";
  const before = side === "left" || side === "top";
  const fresh: PaneNode = { type: "pane", id: idGen("pane"), agentId };

  const rec = (n: LayoutNode): LayoutNode => {
    if (n.type === "pane") {
      if (n.id !== paneId) return n;
      return {
        type: "split",
        id: idGen("split"),
        dir,
        children: before ? [fresh, n] : [n, fresh],
        sizes: [0.5, 0.5],
      };
    }
    const idx = n.children.findIndex((c) => c.type === "pane" && c.id === paneId);
    if (idx >= 0 && n.dir === dir) {
      // Same direction: insert a sibling and halve the target's share.
      const half = n.sizes[idx] / 2;
      const children = [...n.children];
      const sizes = [...n.sizes];
      const at = before ? idx : idx + 1;
      children.splice(at, 0, fresh);
      sizes.splice(idx, 1, half, half);
      return { ...n, children, sizes };
    }
    return { ...n, children: n.children.map(rec) };
  };
  return { tree: normalize(rec(tree))!, paneId: fresh.id };
}

/** Remove a pane; its space goes to its neighbours. Returns null if it was the last pane. */
export function removePane(node: LayoutNode, paneId: string): LayoutNode | null {
  const rec = (n: LayoutNode): LayoutNode | null => {
    if (n.type === "pane") return n.id === paneId ? null : n;
    const kept: LayoutNode[] = [];
    const sizes: number[] = [];
    n.children.forEach((c, i) => {
      const r = rec(c);
      if (r) {
        kept.push(r);
        sizes.push(n.sizes[i]);
      }
    });
    if (!kept.length) return null;
    return { ...n, children: kept, sizes: normalizeSizes(sizes) };
  };
  return normalize(rec(node));
}

/** Take an agent out of the layout: its pane is removed, or emptied if it is the only pane. */
export function dropAgent(node: LayoutNode | null, agentId: string): LayoutNode | null {
  if (!node) return null;
  const pane = findPaneByAgent(node, agentId);
  if (!pane) return node;
  if (leaves(node).length === 1) return setPaneAgent(node, pane.id, null);
  return removePane(node, pane.id);
}

/** Move the divider between child `index` and `index + 1` of split `splitId` by `delta` (fraction). */
export function resizeSplit(node: LayoutNode, splitId: string, index: number, delta: number, minFrac = 0.08): LayoutNode {
  if (node.type === "pane") return node;
  if (node.id === splitId) {
    const sizes = [...node.sizes];
    const a = sizes[index];
    const b = sizes[index + 1];
    if (a === undefined || b === undefined) return node;
    const d = Math.max(minFrac - a, Math.min(b - minFrac, delta));
    sizes[index] = a + d;
    sizes[index + 1] = b - d;
    return { ...node, sizes };
  }
  return { ...node, children: node.children.map((c) => resizeSplit(c, splitId, index, delta, minFrac)) };
}

/** Replace sizes of one split (used to commit a drag). */
export function setSplitSizes(node: LayoutNode, splitId: string, sizes: number[]): LayoutNode {
  if (node.type === "pane") return node;
  if (node.id === splitId && sizes.length === node.children.length) return { ...node, sizes: normalizeSizes(sizes) };
  return { ...node, children: node.children.map((c) => setSplitSizes(c, splitId, sizes)) };
}

/** Build a preset layout filled with `agentIds` in order (missing slots are empty panes). */
export function buildPreset(preset: GridPreset, agentIds: (string | null)[], idGen: IdGen = defaultIdGen): LayoutNode {
  if (preset === "3") {
    const pane = (i: number): PaneNode => ({ type: "pane", id: idGen("pane"), agentId: agentIds[i] ?? null });
    const col: SplitNode = { type: "split", id: idGen("split"), dir: "col", children: [pane(1), pane(2)], sizes: [0.5, 0.5] };
    return { type: "split", id: idGen("split"), dir: "row", children: [pane(0), col], sizes: [0.6, 0.4] };
  }
  const [cols, rows] = preset === "1" ? [1, 1] : preset === "2" ? [2, 1] : preset.split("x").map(Number);
  return buildGrid(Array.from({ length: rows }, () => cols), agentIds, idGen);
}

/**
 * A grid of rows (top to bottom), `rowCounts[i]` panes side by side in row i,
 * filled with `agentIds` in reading order (missing slots are empty panes).
 */
export function buildGrid(rowCounts: number[], agentIds: (string | null)[], idGen: IdGen = defaultIdGen): LayoutNode {
  let next = 0;
  const pane = (): PaneNode => ({ type: "pane", id: idGen("pane"), agentId: agentIds[next++] ?? null });
  const split = (dir: Dir, children: LayoutNode[]): LayoutNode =>
    children.length === 1
      ? children[0]
      : { type: "split", id: idGen("split"), dir, children, sizes: children.map(() => 1 / children.length) };
  const counts = rowCounts.filter((n) => n > 0);
  if (!counts.length) return pane();
  return split(
    "col",
    counts.map((n) => split("row", Array.from({ length: n }, pane))),
  );
}

/** Smallest preset that holds `n` agents. */
export function presetFor(n: number): "1" | "2" | "3" | "2x2" | "3x2" {
  if (n <= 1) return "1";
  if (n === 2) return "2";
  if (n === 3) return "3";
  if (n === 4) return "2x2";
  return "3x2";
}

/** Split `n` panes over `rows` rows as evenly as possible (longer rows first). */
export function evenRows(n: number, rows: number): number[] {
  const r = Math.max(1, Math.min(rows, n));
  const base = Math.floor(n / r);
  const extra = n % r;
  return Array.from({ length: r }, (_, i) => base + (i < extra ? 1 : 0));
}

export interface MinSize {
  minW: number;
  minH: number;
}

/**
 * Auto grid: row counts that tile as many of `n` agents as fit at `min`
 * (the rest become chips), spread as evenly as possible. `maxPerRow` is the
 * preferred column count (breakpoint default), exceeded only when the agents
 * would not fit otherwise; among equals the grid with the biggest tiles wins.
 */
export function autoGrid(n: number, area: { w: number; h: number }, min: MinSize, maxPerRow = Infinity): number[] {
  if (n <= 1) return [1];
  const maxCols = Math.max(1, Math.floor(area.w / min.minW));
  const maxRows = Math.max(1, Math.floor(area.h / min.minH));
  const k = Math.min(n, maxCols * maxRows);
  let best: { rows: number; over: boolean; scale: number } | null = null;
  for (let rows = 1; rows <= Math.min(k, maxRows); rows++) {
    const cols = Math.ceil(k / rows);
    if (cols > maxCols || (rows - 1) * cols >= k) continue; // too wide / a row would be empty
    const over = cols > Math.max(1, maxPerRow);
    const scale = Math.min(area.w / cols / min.minW, area.h / rows / min.minH);
    if (!best || (best.over && !over) || (best.over === over && scale > best.scale + 1e-9)) best = { rows, over, scale };
  }
  return evenRows(k, best?.rows ?? 1);
}

/** Whether every pane of `preset` would be at least `min` in `area` ("1" and "auto" always fit). */
export function presetFits(preset: Preset, area: { w: number; h: number }, min: MinSize): boolean {
  if (preset === "1" || preset === "auto") return true;
  let n = 0;
  const tree = buildPreset(preset, [], (p) => `${p}${n++}`);
  for (const r of paneRects(tree, { x: 0, y: 0, w: area.w, h: area.h }).values()) {
    if (r.w < min.minW || r.h < min.minH) return false;
  }
  return true;
}

/** Presets offered for a space of `area` px at a minimum tile size. */
export function availablePresets(area: { w: number; h: number } | null, min: MinSize): Preset[] {
  return area ? PRESETS.filter((p) => presetFits(p, area, min)) : PRESETS;
}

// ── geometry ───────────────────────────────────────────────────────────────

/** Pixel rects of every pane inside `rect` (dividers ignored). */
export function paneRects(node: LayoutNode | null, rect: Rect): Map<string, Rect> {
  const out = new Map<string, Rect>();
  const rec = (n: LayoutNode, r: Rect) => {
    if (n.type === "pane") {
      out.set(n.id, r);
      return;
    }
    let offset = 0;
    n.children.forEach((c, i) => {
      const f = n.sizes[i];
      if (n.dir === "row") rec(c, { x: r.x + offset * r.w, y: r.y, w: f * r.w, h: r.h });
      else rec(c, { x: r.x, y: r.y + offset * r.h, w: r.w, h: f * r.h });
      offset += f;
    });
  };
  if (node) rec(node, rect);
  return out;
}

/**
 * Render-time fit: drop panes that would be smaller than `minW`×`minH` (or,
 * when given, crowd more than `maxPerRow` side by side), until everything fits
 * (one pane always stays). `keep` (e.g. the focused pane) is removed last.
 * Returns the tree to render and the ids of panes that were folded away.
 */
export function fitLayout(
  node: LayoutNode | null,
  size: { w: number; h: number },
  opts: { minW: number; minH: number; maxPerRow?: number; keep?: string | null },
): { tree: LayoutNode | null; hidden: PaneNode[] } {
  if (!node) return { tree: null, hidden: [] };
  const minW = opts.maxPerRow ? Math.max(opts.minW, size.w / (opts.maxPerRow + 1) + 1) : opts.minW;
  const minH = opts.minH;
  let tree: LayoutNode | null = node;
  const hidden: PaneNode[] = [];
  for (;;) {
    const ls = leaves(tree);
    if (ls.length <= 1 || !tree) break;
    const rects = paneRects(tree, { x: 0, y: 0, w: size.w, h: size.h });
    const bad = ls.filter((p) => {
      const r = rects.get(p.id)!;
      return r.w < minW || r.h < minH;
    });
    if (!bad.length) break;
    // Fold the last offender in reading order, sparing `keep` and filled panes when possible.
    const order = [...bad].reverse();
    const victim =
      order.find((p) => p.id !== opts.keep && !p.agentId) ?? order.find((p) => p.id !== opts.keep) ?? order[0];
    hidden.push(victim);
    tree = removePane(tree, victim.id);
  }
  return { tree, hidden };
}

/** Which drop zone a point (relative 0..1 coords inside a pane) falls in. */
export function dropZone(fx: number, fy: number, edge = 0.28): Side | "center" {
  const d = { left: fx, right: 1 - fx, top: fy, bottom: 1 - fy };
  const [side, dist] = (Object.entries(d) as [Side, number][]).sort((a, b) => a[1] - b[1])[0];
  return dist < edge ? side : "center";
}
