// Shared UI state blob (spaces, layouts, window ownership) and pure operations on it.
// Persisted by the backend via get_ui_state / set_ui_state; every window shares it.
import type { AgentView } from "../types";
import { DEFAULT_DENSITY, DEFAULT_FONT, isDensity, type Density } from "../layout/density";
import { isThemePref, type ThemePref } from "../lib/theme";
import {
  agentsIn,
  buildGrid,
  buildPreset,
  defaultIdGen,
  dropAgent,
  findPane,
  findPaneByAgent,
  fitLayout,
  leaves,
  paneRects,
  presetFor,
  removePane,
  setPaneAgent,
  splitPane,
  PRESET_COUNT,
  type LayoutNode,
  type MinSize,
  type Preset,
  type Side,
} from "../layout/tree";

export const MAIN = "main";
export const ALL_SPACE = "all";
export { DEFAULT_FONT };

export interface Space {
  id: string;
  name: string;
  kind: "all" | "project" | "custom";
  /** For project spaces: the project path. */
  project?: string;
  /** For custom spaces: agents that belong here (panes or chips). */
  members: string[];
  layout: LayoutNode;
  focusedPaneId: string | null;
  maximizedPaneId: string | null;
  /** Overrides the global density for this space. */
  density?: Density;
}

export interface UiState {
  v: 1;
  spaces: Space[];
  /** spaceId → window label; missing means "main". */
  windowOf: Record<string, string>;
  /** Window labels currently in Wall mode. */
  wall: string[];
  /** Base terminal font size (Settings); tiles may shrink below it or override it. */
  fontSize: number;
  /** Global default tile density. */
  density: Density;
  /** Per-agent tile font chosen with ⌘+ / ⌘− (⌘0 removes it). */
  tileFont: Record<string, number>;
  /** Collapsed project groups in the sidebar / wall (project paths). */
  collapsed: string[];
  wallCollapsed: string[];
  /** Ask the window holding `agentId` to focus it (cross-window jumps). */
  focusRequest: { agentId: string; window: string; nonce: number } | null;
  /** Settings: hide the sidebar's "Elsewhere" group (agents in other terminal apps). */
  hideElsewhere: boolean;
  /** Settings → Appearance: System (follow macOS), Dark or Light. */
  theme: ThemePref;
}

const emptyPane = (): LayoutNode => ({ type: "pane", id: defaultIdGen("pane"), agentId: null });

export function defaultUiState(): UiState {
  const layout = emptyPane();
  return {
    v: 1,
    spaces: [
      { id: ALL_SPACE, name: "All", kind: "all", members: [], layout, focusedPaneId: layout.id, maximizedPaneId: null },
    ],
    windowOf: {},
    wall: [],
    fontSize: DEFAULT_FONT,
    density: DEFAULT_DENSITY,
    tileFont: {},
    collapsed: [],
    wallCollapsed: [],
    focusRequest: null,
    hideElsewhere: false,
    theme: "system",
  };
}

/** Accept whatever came from disk; fall back to defaults for anything malformed. */
export function sanitize(raw: unknown): UiState {
  const d = defaultUiState();
  if (!raw || typeof raw !== "object" || (raw as UiState).v !== 1) return d;
  const r = raw as Partial<UiState>;
  const spaces = Array.isArray(r.spaces)
    ? r.spaces.filter((s) => s && typeof s.id === "string" && s.layout && typeof s.layout === "object")
    : [];
  if (!spaces.some((s) => s.id === ALL_SPACE)) spaces.unshift(d.spaces[0]);
  return {
    v: 1,
    spaces: spaces.map((s) => ({
      ...s,
      members: Array.isArray(s.members) ? s.members : [],
      density: isDensity(s.density) ? s.density : undefined,
    })),
    windowOf: r.windowOf && typeof r.windowOf === "object" ? r.windowOf : {},
    wall: Array.isArray(r.wall) ? r.wall : [],
    fontSize: typeof r.fontSize === "number" ? r.fontSize : DEFAULT_FONT,
    density: isDensity(r.density) ? r.density : DEFAULT_DENSITY,
    tileFont:
      r.tileFont && typeof r.tileFont === "object"
        ? Object.fromEntries(Object.entries(r.tileFont).filter(([, v]) => typeof v === "number"))
        : {},
    collapsed: Array.isArray(r.collapsed) ? r.collapsed : [],
    wallCollapsed: Array.isArray(r.wallCollapsed) ? r.wallCollapsed : [],
    focusRequest: r.focusRequest ?? null,
    hideElsewhere: r.hideElsewhere === true,
    theme: isThemePref(r.theme) ? r.theme : "system",
  };
}

// ── queries ────────────────────────────────────────────────────────────────

export const windowOfSpace = (s: UiState, spaceId: string) => s.windowOf[spaceId] ?? MAIN;

export function spacesOf(s: UiState, label: string): Space[] {
  return s.spaces.filter((sp) => windowOfSpace(s, sp.id) === label);
}

export function getSpace(s: UiState, spaceId: string): Space | undefined {
  return s.spaces.find((sp) => sp.id === spaceId);
}

/** The density a space tiles at: its own, else the global default. */
export function densityOf(s: UiState, space: Space | undefined): Density {
  return space?.density ?? s.density;
}

/** Where an agent is shown (it is in at most one pane). */
export function locate(s: UiState, agentId: string): { space: Space; paneId: string; window: string } | null {
  for (const space of s.spaces) {
    const p = findPaneByAgent(space.layout, agentId);
    if (p) return { space, paneId: p.id, window: windowOfSpace(s, space.id) };
  }
  return null;
}

/** Agents that belong to a space (shown in panes or offered as chips). */
export function spaceMembers(space: Space, agents: AgentView[]): AgentView[] {
  const inPanes = new Set(agentsIn(space.layout));
  switch (space.kind) {
    case "all":
      return agents;
    case "project":
      return agents.filter((a) => a.project === space.project || space.members.includes(a.id) || inPanes.has(a.id));
    case "custom":
      return agents.filter((a) => space.members.includes(a.id) || inPanes.has(a.id));
  }
}

// ── edits (all pure) ───────────────────────────────────────────────────────

function mapSpace(s: UiState, spaceId: string, fn: (sp: Space) => Space): UiState {
  return { ...s, spaces: s.spaces.map((sp) => (sp.id === spaceId ? fn(sp) : sp)) };
}

/** Remove an agent from every pane except in `exceptSpace`. */
function unplace(s: UiState, agentId: string, exceptSpace?: string): UiState {
  return {
    ...s,
    spaces: s.spaces.map((sp) => {
      if (sp.id === exceptSpace || !findPaneByAgent(sp.layout, agentId)) return sp;
      const layout = dropAgent(sp.layout, agentId) ?? emptyPane();
      const focusedPaneId = findPane(layout, sp.focusedPaneId ?? "") ? sp.focusedPaneId : leaves(layout)[0].id;
      const maximizedPaneId = findPane(layout, sp.maximizedPaneId ?? "") ? sp.maximizedPaneId : null;
      return { ...sp, layout, focusedPaneId, maximizedPaneId };
    }),
  };
}

/**
 * Show an agent in a space. With `side`, split `paneId` (default: focused pane);
 * otherwise put it in `paneId`, else the first empty pane, else the focused pane.
 * The agent leaves any other pane it was in (one pane per agent).
 */
export function placeAgent(
  s: UiState,
  spaceId: string,
  agentId: string,
  target: { paneId?: string; side?: Side } = {},
): UiState {
  let next = unplace(s, agentId, spaceId);
  next = mapSpace(next, spaceId, (sp) => {
    const members = sp.kind === "custom" && !sp.members.includes(agentId) ? [...sp.members, agentId] : sp.members;
    const already = findPaneByAgent(sp.layout, agentId);
    const ref = target.paneId ?? sp.focusedPaneId ?? leaves(sp.layout)[0].id;
    if (target.side) {
      const anchor = findPane(sp.layout, ref) ? ref : leaves(sp.layout)[0].id;
      const { tree, paneId } = splitPane(sp.layout, anchor, target.side, agentId);
      return { ...sp, members, layout: tree, focusedPaneId: paneId, maximizedPaneId: null };
    }
    if (already && !target.paneId) return { ...sp, members, focusedPaneId: already.id };
    const empty = leaves(sp.layout).find((p) => !p.agentId);
    const paneId = target.paneId && findPane(sp.layout, target.paneId) ? target.paneId : (empty?.id ?? ref);
    let layout = sp.layout;
    if (already && already.id !== paneId) {
      // Moving within the space: swap so the displaced agent takes the old pane.
      const displaced = findPane(layout, paneId)?.agentId ?? null;
      layout = setPaneAgent(layout, already.id, displaced);
    }
    layout = setPaneAgent(layout, paneId, agentId);
    return { ...sp, members, layout, focusedPaneId: paneId };
  });
  return next;
}

// ── drag & drop (docs/spec/layout.md "Revision: drag & drop must rearrange properly") ──

/** Where a drop lands inside a pane: a side splits it, the centre swaps/replaces. */
export type DropZone = Side | "center";
/** The space area and the tile minimum, to keep a drop within the density limits. */
export interface DropFit {
  area: { w: number; h: number };
  min: MinSize;
}
/**
 * noop: nothing changed (dropped on itself / already there) · placed: shown in
 * a new or empty pane · swapped: traded places with the target's agent ·
 * replaced: took the target pane (its agent stays a chip) · chip: no room at
 * this density, so it joined the overflow strip instead.
 */
export type DropOutcome = "noop" | "placed" | "swapped" | "replaced" | "chip";
export interface DropResult {
  state: UiState;
  outcome: DropOutcome;
  /** The pane now showing the agent (null for a chip). */
  paneId: string | null;
}

/** Keep focus / maximise pointing at panes that still exist. */
function tidy(sp: Space): Space {
  const ls = leaves(sp.layout);
  const focusedPaneId = findPane(sp.layout, sp.focusedPaneId ?? "") ? sp.focusedPaneId : ls[0].id;
  const maximizedPaneId = sp.maximizedPaneId && findPane(sp.layout, sp.maximizedPaneId) && ls.length > 1 ? sp.maximizedPaneId : null;
  return focusedPaneId === sp.focusedPaneId && maximizedPaneId === sp.maximizedPaneId ? sp : { ...sp, focusedPaneId, maximizedPaneId };
}

const withMember = (sp: Space, agentId: string): Space =>
  sp.kind === "all" || sp.members.includes(agentId) ? sp : { ...sp, members: [...sp.members, agentId] };
const withoutMember = (sp: Space, agentId: string): Space =>
  sp.members.includes(agentId) ? { ...sp, members: sp.members.filter((m) => m !== agentId) } : sp;

/**
 * Move semantics: take the agent out of every other space — its pane closes
 * (the tree collapses, no holes) and it stops being a member there.
 */
function detach(s: UiState, agentId: string, exceptSpace: string): UiState {
  const next = unplace(s, agentId, exceptSpace);
  return { ...next, spaces: next.spaces.map((sp) => (sp.id === exceptSpace ? sp : withoutMember(sp, agentId))) };
}

/** Panes folded away at `fit` (none without a fit). */
const foldedPanes = (layout: LayoutNode, fit: DropFit | undefined, keep?: string) =>
  fit ? fitLayout(layout, fit.area, { ...fit.min, keep }).hidden : [];

/** Would `after` (with new pane `fresh`) still show `fresh` without folding more panes than `before`? */
function roomFor(before: LayoutNode, after: LayoutNode, fresh: string, fit: DropFit | undefined): boolean {
  if (!fit) return true;
  const hidden = foldedPanes(after, fit, fresh);
  return !hidden.some((p) => p.id === fresh) && hidden.length <= foldedPanes(before, fit).length;
}

/** Same-space layout without the agent's own pane, if it has one. */
function withoutAgent(layout: LayoutNode, agentId: string): LayoutNode {
  const holder = findPaneByAgent(layout, agentId);
  if (!holder) return layout;
  return removePane(layout, holder.id) ?? emptyPane();
}

/** No room: the agent becomes a chip of the space (not shown in any pane). */
function asChip(s: UiState, spaceId: string, agentId: string): DropResult {
  let next = detach(s, agentId, spaceId);
  next = mapSpace(next, spaceId, (sp) => {
    const holder = findPaneByAgent(sp.layout, agentId);
    const layout = holder ? (leaves(sp.layout).length > 1 ? removePane(sp.layout, holder.id)! : setPaneAgent(sp.layout, holder.id, null)) : sp.layout;
    return tidy(withMember({ ...sp, layout }, agentId));
  });
  return { state: next, outcome: "chip", paneId: null };
}

/**
 * Drop `agentId` on pane `paneId` of `spaceId`. A side zone splits that pane
 * (the agent's old pane anywhere closes first); the centre swaps with the
 * pane's agent — the displaced one takes the dragged agent's old pane — or,
 * when the dragged agent isn't shown anywhere, replaces it (it stays a chip).
 * Dropping an agent on its own pane does nothing. With `fit`, a split that
 * would leave a tile below the density minimum becomes a chip instead.
 */
export function dropOnPane(
  s: UiState,
  spaceId: string,
  agentId: string,
  paneId: string,
  zone: DropZone,
  fit?: DropFit,
): DropResult {
  const sp = getSpace(s, spaceId);
  const target = sp && findPane(sp.layout, paneId);
  if (!sp || !target) return dropOnSpace(s, spaceId, agentId, fit);
  if (target.agentId === agentId) return { state: s, outcome: "noop", paneId };

  if (zone === "center" || !target.agentId) {
    const from = locate(s, agentId);
    const displaced = target.agentId;
    let next = s;
    let outcome: DropOutcome = displaced ? "replaced" : "placed";
    if (from && displaced) {
      // Swap: the displaced agent takes the dragged agent's old pane (in whichever space).
      outcome = "swapped";
      next = mapSpace(next, from.space.id, (x) => withMember({ ...x, layout: setPaneAgent(x.layout, from.paneId, displaced) }, displaced));
      next = detach(next, agentId, spaceId);
      next = mapSpace(next, spaceId, (x) => {
        const layout = setPaneAgent(x.layout, paneId, agentId);
        return from.space.id === spaceId ? { ...x, layout } : withoutMember({ ...x, layout }, displaced);
      });
    } else {
      // Into an empty pane (the old one closes) or replacing the target's agent.
      next = detach(next, agentId, spaceId);
      next = mapSpace(next, spaceId, (x) => {
        let layout = x.layout;
        const holder = findPaneByAgent(layout, agentId);
        if (holder) layout = removePane(layout, holder.id) ?? layout;
        layout = setPaneAgent(layout, paneId, agentId);
        return displaced ? withMember({ ...x, layout }, displaced) : { ...x, layout };
      });
    }
    next = mapSpace(next, spaceId, (x) => tidy({ ...withMember(x, agentId), focusedPaneId: paneId }));
    return { state: next, outcome, paneId };
  }

  // Split a side: close the agent's old pane first so it moves, never duplicates.
  const next = detach(s, agentId, spaceId);
  const here = getSpace(next, spaceId)!;
  const before = withoutAgent(here.layout, agentId);
  if (!findPane(before, paneId)) return dropOnSpace(s, spaceId, agentId, fit);
  const { tree, paneId: fresh } = splitPane(before, paneId, zone, agentId);
  if (!roomFor(before, tree, fresh, fit)) return asChip(s, spaceId, agentId);
  const state = mapSpace(next, spaceId, (x) =>
    tidy({ ...withMember(x, agentId), layout: tree, focusedPaneId: fresh, maximizedPaneId: null }),
  );
  return { state, outcome: "placed", paneId: fresh };
}

/**
 * Drop `agentId` on a space (its tab, or the space area): auto-place it in an
 * empty pane, else by splitting the pane with the most room, else as a chip.
 * It leaves wherever it was shown; if it's already shown here, just focus it.
 */
export function dropOnSpace(s: UiState, spaceId: string, agentId: string, fit?: DropFit): DropResult {
  const sp = getSpace(s, spaceId);
  if (!sp) return { state: s, outcome: "noop", paneId: null };
  const here = findPaneByAgent(sp.layout, agentId);
  if (here) return { state: focusPane(s, spaceId, here.id), outcome: "noop", paneId: here.id };

  const next = detach(s, agentId, spaceId);
  const layout = getSpace(next, spaceId)!.layout;
  const empty = leaves(layout).find((p) => !p.agentId);
  if (empty) {
    const state = mapSpace(next, spaceId, (x) =>
      tidy({ ...withMember(x, agentId), layout: setPaneAgent(x.layout, empty.id, agentId), focusedPaneId: empty.id }),
    );
    return { state, outcome: "placed", paneId: empty.id };
  }
  const spot = autoSpot(layout, fit);
  const { tree, paneId: fresh } = splitPane(layout, spot.paneId, spot.side, agentId);
  if (!roomFor(layout, tree, fresh, fit)) return asChip(s, spaceId, agentId);
  const state = mapSpace(next, spaceId, (x) =>
    tidy({ ...withMember(x, agentId), layout: tree, focusedPaneId: fresh, maximizedPaneId: null }),
  );
  return { state, outcome: "placed", paneId: fresh };
}

/** The pane (and side) whose split leaves the roomiest new tile, relative to the minimum. */
export function autoSpot(layout: LayoutNode, fit?: DropFit): { paneId: string; side: "right" | "bottom" } {
  const area = fit?.area ?? { w: 1600, h: 900 };
  const min = fit?.min ?? { minW: 1, minH: 1 };
  let best = { paneId: leaves(layout)[0].id, side: "right" as "right" | "bottom", score: -1 };
  for (const [id, r] of paneRects(layout, { x: 0, y: 0, ...area })) {
    const right = Math.min(r.w / 2 / min.minW, r.h / min.minH);
    const bottom = Math.min(r.w / min.minW, r.h / 2 / min.minH);
    const [side, score] = right >= bottom ? (["right", right] as const) : (["bottom", bottom] as const);
    if (score > best.score + 1e-9) best = { paneId: id, side, score };
  }
  return best;
}

export function focusPane(s: UiState, spaceId: string, paneId: string): UiState {
  const sp = getSpace(s, spaceId);
  if (!sp || sp.focusedPaneId === paneId) return s;
  return mapSpace(s, spaceId, (x) => ({ ...x, focusedPaneId: paneId }));
}

export function toggleMaximize(s: UiState, spaceId: string, paneId?: string | null): UiState {
  return mapSpace(s, spaceId, (sp) => {
    const id = paneId ?? sp.focusedPaneId;
    if (!id || leaves(sp.layout).length < 2) return { ...sp, maximizedPaneId: null };
    return { ...sp, maximizedPaneId: sp.maximizedPaneId === id ? null : id, focusedPaneId: id };
  });
}

export function closePane(s: UiState, spaceId: string, paneId: string): UiState {
  return mapSpace(s, spaceId, (sp) => {
    const layout = removePane(sp.layout, paneId) ?? emptyPane();
    const ls = leaves(layout);
    return {
      ...sp,
      layout,
      focusedPaneId: findPane(layout, sp.focusedPaneId ?? "") ? sp.focusedPaneId : ls[0].id,
      maximizedPaneId: sp.maximizedPaneId === paneId ? null : sp.maximizedPaneId,
    };
  });
}

export function setLayout(s: UiState, spaceId: string, layout: LayoutNode): UiState {
  return mapSpace(s, spaceId, (sp) => ({ ...sp, layout }));
}

/**
 * Re-tile a space with a preset, keeping what's shown first and then filling
 * with members that are not shown anywhere else. "auto" tiles all of those
 * candidates in the rows `autoRows(count)` returns (layout/tree autoGrid);
 * the rest stay chips.
 */
export function applyPreset(
  s: UiState,
  spaceId: string,
  preset: Preset,
  agents: AgentView[],
  autoRows: (n: number) => number[] = (n) => [Math.max(1, n)],
): UiState {
  const sp = getSpace(s, spaceId);
  if (!sp) return s;
  const n = preset === "auto" ? Infinity : PRESET_COUNT[preset];
  const shown = agentsIn(sp.layout);
  const focused = findPane(sp.layout, sp.focusedPaneId ?? "")?.agentId;
  const ordered = [...(focused ? [focused] : []), ...shown.filter((a) => a !== focused)];
  for (const a of spaceMembers(sp, agents)) {
    if (ordered.length >= n) break;
    if (!ordered.includes(a.id) && !locate(s, a.id)) ordered.push(a.id);
  }
  const rows = preset === "auto" ? autoRows(ordered.length) : null;
  const chosen = ordered.slice(0, rows ? rows.reduce((a, b) => a + b, 0) : n);
  const layout = rows ? buildGrid(rows, chosen) : buildPreset(preset as Exclude<Preset, "auto">, chosen);
  return mapSpace(s, spaceId, (x) => ({ ...x, layout, focusedPaneId: leaves(layout)[0].id, maximizedPaneId: null }));
}

/**
 * Tile `agentIds` in a space (first one focused), followed by agents already
 * shown there, up to `max` panes; members that don't fit stay chips.
 */
export function tileAgents(s: UiState, spaceId: string, agentIds: string[], max: number): UiState {
  const sp = getSpace(s, spaceId);
  if (!sp || !agentIds.length) return s;
  const cap = Math.max(1, Math.min(max, PRESET_COUNT["3x2"]));
  const shown = agentsIn(sp.layout).filter((a) => !agentIds.includes(a));
  const chosen = [...new Set([...agentIds, ...shown])].slice(0, cap);
  let next = s;
  for (const id of chosen) next = unplace(next, id, spaceId);
  const layout = buildPreset(presetFor(chosen.length), chosen);
  return mapSpace(next, spaceId, (x) => ({ ...x, layout, focusedPaneId: leaves(layout)[0].id, maximizedPaneId: null }));
}

/** Create (or reuse) a space tiling one project's agents, owned by `label`. */
export function openProjectSpace(
  s: UiState,
  project: string,
  name: string,
  agents: AgentView[],
  label: string,
): { state: UiState; spaceId: string } {
  const existing = s.spaces.find((sp) => sp.kind === "project" && sp.project === project);
  if (existing) return { state: s, spaceId: existing.id };
  const ids = agents.filter((a) => a.project === project).map((a) => a.id);
  const chosen = ids.slice(0, PRESET_COUNT[presetFor(ids.length)]);
  let next = s;
  for (const id of chosen) next = unplace(next, id);
  const layout = buildPreset(presetFor(chosen.length), chosen);
  const id = defaultIdGen("space");
  const space: Space = { id, name, kind: "project", project, members: [], layout, focusedPaneId: leaves(layout)[0].id, maximizedPaneId: null };
  return { state: { ...next, spaces: [...next.spaces, space], windowOf: { ...next.windowOf, [id]: label } }, spaceId: id };
}

export function createCustomSpace(s: UiState, label: string, agentId?: string): { state: UiState; spaceId: string } {
  const n = s.spaces.filter((sp) => sp.kind === "custom").length + 1;
  const layout = emptyPane();
  const id = defaultIdGen("space");
  const space: Space = { id, name: `Space ${n}`, kind: "custom", members: [], layout, focusedPaneId: layout.id, maximizedPaneId: null };
  let state: UiState = { ...s, spaces: [...s.spaces, space], windowOf: { ...s.windowOf, [id]: label } };
  if (agentId) state = dropOnSpace(state, id, agentId).state;
  return { state, spaceId: id };
}

export function renameSpace(s: UiState, spaceId: string, name: string): UiState {
  return mapSpace(s, spaceId, (sp) => ({ ...sp, name: name.trim() || sp.name }));
}

/** Close a space; its agents simply stop being shown there. "All" can't be closed. */
export function closeSpace(s: UiState, spaceId: string): UiState {
  if (spaceId === ALL_SPACE) return s;
  const windowOf = { ...s.windowOf };
  delete windowOf[spaceId];
  return { ...s, spaces: s.spaces.filter((sp) => sp.id !== spaceId), windowOf };
}

export function moveSpaceToWindow(s: UiState, spaceId: string, label: string): UiState {
  return { ...s, windowOf: { ...s.windowOf, [spaceId]: label } };
}

/** A window went away: its spaces come home to main (and it leaves Wall mode). */
export function reclaimWindow(s: UiState, label: string): UiState {
  if (label === MAIN) return s;
  const windowOf = Object.fromEntries(Object.entries(s.windowOf).filter(([, w]) => w !== label));
  return { ...s, windowOf, wall: s.wall.filter((w) => w !== label) };
}

/** Drop references to agents that no longer exist. */
export function pruneAgents(s: UiState, agentIds: Set<string>): UiState {
  let next = s;
  for (const sp of s.spaces) {
    for (const a of agentsIn(sp.layout)) if (!agentIds.has(a)) next = unplace(next, a);
  }
  const fonts = Object.keys(next.tileFont);
  if (fonts.some((a) => !agentIds.has(a)))
    next = { ...next, tileFont: Object.fromEntries(Object.entries(next.tileFont).filter(([a]) => agentIds.has(a))) };
  return {
    ...next,
    spaces: next.spaces.map((sp) =>
      sp.members.every((m) => agentIds.has(m)) ? sp : { ...sp, members: sp.members.filter((m) => agentIds.has(m)) },
    ),
  };
}

/** Set the global density, or (with `spaceId`) a space's own; `null` there means "use the global one". */
export function setDensity(s: UiState, density: Density | null, spaceId?: string): UiState {
  if (!spaceId) return density && density !== s.density ? { ...s, density } : s;
  return mapSpace(s, spaceId, (sp) => {
    const next = { ...sp };
    if (density) next.density = density;
    else delete next.density;
    return next;
  });
}

export function setWall(s: UiState, label: string, on: boolean): UiState {
  const has = s.wall.includes(label);
  if (has === on) return s;
  return { ...s, wall: on ? [...s.wall, label] : s.wall.filter((w) => w !== label) };
}

/** Settings: show or hide the sidebar's "Elsewhere" group. */
export function setHideElsewhere(s: UiState, hide: boolean): UiState {
  return s.hideElsewhere === hide ? s : { ...s, hideElsewhere: hide };
}

/** Settings → Appearance. */
export function setTheme(s: UiState, theme: ThemePref): UiState {
  return s.theme === theme ? s : { ...s, theme };
}

export function toggleIn(list: string[], v: string): string[] {
  return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
}
