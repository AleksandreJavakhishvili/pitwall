// Glue between the pure tiling policy and the live window: measures the
// active space area so ⌘K's presets and Auto grid match what SpaceView shows.
import { availablePresets, autoGrid, type MinSize, type Preset } from "../layout/tree";
import { foldMin } from "../layout/density";
import { densityOf, getSpace, type UiState } from "../state/workspace";

export interface TilingContext {
  /** Content box of the space area (px). */
  area: { w: number; h: number };
  /** Below this a pane folds into a chip. */
  min: MinSize;
  /** Breakpoint default columns for Auto grid. */
  maxPerRow: number;
}

/** Measure this window's space area for space `spaceId`; null when it isn't on screen. */
export function measureTiling(s: UiState, spaceId: string, maxPerRow: number): TilingContext | null {
  const el = document.querySelector<HTMLElement>(".space-area");
  if (!el) return null;
  const cs = getComputedStyle(el);
  const w = el.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const h = el.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
  if (!(w > 0 && h > 0)) return null;
  return { area: { w, h }, min: foldMin(densityOf(s, getSpace(s, spaceId)), s.fontSize), maxPerRow };
}

export const autoRows = (c: TilingContext | null) => (c ? (n: number) => autoGrid(n, c.area, c.min, c.maxPerRow) : undefined);

export const presetsFor = (c: TilingContext | null): Preset[] => availablePresets(c?.area ?? null, c?.min ?? { minW: 0, minH: 0 });
