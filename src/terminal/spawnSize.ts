// Pure geometry for picking the size a new agent's PTY starts at, so its
// first frames are drawn at the width they will be shown at (docs/spec/api.md).
import { presetFor, PRESET_COUNT } from "../layout/tree";

export interface TermSize {
  cols: number;
  rows: number;
}

export interface Box {
  w: number;
  h: number;
}

/** What a pane spends around its terminal (px): pane head, body insets, xterm scrollbar. */
export const PANE_CHROME: Box = { w: 10 + 4 + 14, h: 34 + 6 + 4 };
/** Padding of `.space-area` (each side) and the width of a split divider. */
export const AREA_PAD = 6;
export const DIVIDER = 6;

/** Cells that fit a pane of `box` px, the way FitAddon would fit it (rounded down). */
export function fitCells(box: Box, cell: Box, chrome: Box = PANE_CHROME): TermSize | null {
  if (!(cell.w > 0 && cell.h > 0)) return null;
  const cols = Math.floor((box.w - chrome.w) / cell.w);
  const rows = Math.floor((box.h - chrome.h) / cell.h);
  if (!Number.isFinite(cols) || !Number.isFinite(rows) || cols < 2 || rows < 1) return null;
  return { cols, rows };
}

/**
 * The pane box agent `index` of `count` gets when they are tiled into a space
 * area of `area` px (layout/tree.ts buildPreset). Extra agents beyond the
 * preset get the smallest tile.
 */
export function tileBox(area: Box, index: number, count: number): Box {
  const preset = presetFor(count);
  const n = PRESET_COUNT[preset];
  const i = Math.min(index, n - 1);
  const W = area.w - 2 * AREA_PAD;
  const H = area.h - 2 * AREA_PAD;
  const split = (total: number, parts: number) => (total - DIVIDER * (parts - 1)) / parts;
  switch (preset) {
    case "1":
      return { w: W, h: H };
    case "2":
      return { w: split(W, 2), h: H };
    case "3": {
      const inner = W - DIVIDER;
      return i === 0 ? { w: inner * 0.6, h: H } : { w: inner * 0.4, h: split(H, 2) };
    }
    case "2x2":
      return { w: split(W, 2), h: split(H, 2) };
    case "3x2":
      return { w: split(W, 3), h: split(H, 2) };
  }
}
