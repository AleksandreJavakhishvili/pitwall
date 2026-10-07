// Pure tile-size policy (docs/spec/layout.md "Revision: more and smaller tiles"):
// density → minimum cols × rows, the pixel minimum a pane needs before it
// folds into a chip, and the per-tile font auto-shrink decision.
import type { MinSize } from "./tree";

export type Density = "comfortable" | "compact" | "dense";

export const DENSITIES: Density[] = ["comfortable", "compact", "dense"];
export const DEFAULT_DENSITY: Density = "compact";
export const DENSITY_LABEL: Record<Density, string> = { comfortable: "Comfortable", compact: "Compact", dense: "Dense" };

export interface Cells {
  cols: number;
  rows: number;
}
export interface Box {
  w: number;
  h: number;
}

/** Smallest readable terminal per density. */
export const DENSITY_CELLS: Record<Density, Cells> = {
  comfortable: { cols: 80, rows: 20 },
  compact: { cols: 60, rows: 12 },
  dense: { cols: 40, rows: 8 },
};

export const DEFAULT_FONT = 13;
/** A tile never auto-shrinks below this (unless the base font is already smaller). */
export const FONT_FLOOR = 10;
export const FONT_MIN = 9;
export const FONT_MAX = 22;

/** Estimated cell size per px of font (JetBrains Mono, xterm lineHeight 1.15). */
export const CELL_PER_PX: Box = { w: 0.6, h: 1.32 };
/** Pane chrome around the terminal (header, insets, scrollbar); matches terminal/spawnSize PANE_CHROME. */
export const PANE_CHROME: Box = { w: 28, h: 44 };

export const isDensity = (x: unknown): x is Density => typeof x === "string" && (DENSITIES as string[]).includes(x);

export const clampFont = (n: number) => Math.max(FONT_MIN, Math.min(FONT_MAX, Math.round(n)));

/** Cells that fit `box` at `font`, after `chrome`. */
export function cellsAt(box: Box, font: number, chrome: Box = { w: 0, h: 0 }, cell: Box = CELL_PER_PX): Cells {
  return {
    cols: Math.floor((box.w - chrome.w) / (font * cell.w)),
    rows: Math.floor((box.h - chrome.h) / (font * cell.h)),
  };
}

/** Pixel size of a pane holding `density`'s minimum cells at `font`. */
export function minTilePx(density: Density, font: number, chrome: Box = PANE_CHROME, cell: Box = CELL_PER_PX): MinSize {
  const c = DENSITY_CELLS[density];
  return { minW: Math.ceil(c.cols * font * cell.w + chrome.w), minH: Math.ceil(c.rows * font * cell.h + chrome.h) };
}

/** Smallest font a tile may shrink to with this base font. */
export const floorFont = (base: number) => Math.min(base, FONT_FLOOR);

/**
 * Below this a pane folds into a chip: the density minimum at the floor font,
 * since tiles shrink their font before folding.
 */
export function foldMin(density: Density, base: number): MinSize {
  return minTilePx(density, floorFont(base));
}

/**
 * Per-tile font: the largest size ≤ `base` (whole px, not below the floor) at
 * which `box` still holds the density minimum. Returns the floor when even
 * that doesn't fit (the layout folds such panes away).
 */
export function autoTileFont(box: Box, base: number, density: Density, chrome: Box = { w: 0, h: 0 }, cell: Box = CELL_PER_PX): number {
  const min = DENSITY_CELLS[density];
  const floor = floorFont(base);
  for (let f = Math.floor(base); f > floor; f--) {
    const c = cellsAt(box, f, chrome, cell);
    if (c.cols >= min.cols && c.rows >= min.rows) return f;
  }
  return floor;
}

/** The font a tile shows: the user's per-tile choice, else the auto-shrunk base. */
export function tileFont(
  box: Box | null,
  base: number,
  density: Density,
  override: number | undefined,
  chrome?: Box,
  cell?: Box,
): number {
  if (override !== undefined) return override;
  return box ? autoTileFont(box, base, density, chrome, cell) : base;
}

/**
 * ⌘+ / ⌘− / ⌘0 on a tile: step from what it shows now (`current`), or drop
 * the override (`delta` null) so it follows the base font again.
 */
export function stepTileFont(
  overrides: Record<string, number>,
  agentId: string,
  current: number,
  delta: number | null,
): Record<string, number> {
  const next = { ...overrides };
  if (delta === null) delete next[agentId];
  else next[agentId] = clampFont(current + delta);
  return next;
}
