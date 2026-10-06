import { describe, expect, it } from "vitest";
import {
  autoTileFont,
  cellsAt,
  DENSITY_CELLS,
  foldMin,
  minTilePx,
  stepTileFont,
  tileFont,
  FONT_FLOOR,
  FONT_MAX,
} from "./density";

const cell = { w: 0.6, h: 1.32 };

describe("density", () => {
  it("maps density to minimum cols × rows", () => {
    expect(DENSITY_CELLS.comfortable).toEqual({ cols: 80, rows: 20 });
    expect(DENSITY_CELLS.compact).toEqual({ cols: 60, rows: 12 });
    expect(DENSITY_CELLS.dense).toEqual({ cols: 40, rows: 8 });
  });

  it("turns density into a pixel minimum, smaller for denser settings", () => {
    const chrome = { w: 0, h: 0 };
    expect(minTilePx("comfortable", 10, chrome, cell)).toEqual({ minW: 480, minH: 264 });
    const c = minTilePx("compact", 13);
    const d = minTilePx("dense", 13);
    expect(d.minW).toBeLessThan(c.minW);
    expect(d.minH).toBeLessThan(c.minH);
  });

  it("folds at the floor font, since tiles shrink before folding", () => {
    expect(foldMin("compact", 16)).toEqual(minTilePx("compact", FONT_FLOOR));
    expect(foldMin("compact", 9)).toEqual(minTilePx("compact", 9)); // base below the floor
  });

  it("keeps the base font when the tile is big enough", () => {
    expect(autoTileFont({ w: 2000, h: 1000 }, 13, "comfortable")).toBe(13);
  });

  it("shrinks the font just enough to keep the density minimum", () => {
    // 60 cols at 12px needs 432px; 13px needs 468px.
    const box = { w: 450, h: 400 };
    const f = autoTileFont(box, 13, "compact", { w: 0, h: 0 }, cell);
    expect(f).toBe(12);
    const c = cellsAt(box, f, { w: 0, h: 0 }, cell);
    expect(c.cols).toBeGreaterThanOrEqual(60);
    expect(c.rows).toBeGreaterThanOrEqual(12);
  });

  it("never shrinks below the floor", () => {
    expect(autoTileFont({ w: 100, h: 50 }, 13, "dense")).toBe(FONT_FLOOR);
    expect(autoTileFont({ w: 100, h: 50 }, 9, "dense")).toBe(9);
  });

  it("a tile's own size wins over auto-shrink", () => {
    expect(tileFont({ w: 100, h: 50 }, 13, "dense", 18)).toBe(18);
    expect(tileFont(null, 13, "dense", undefined)).toBe(13);
  });

  it("steps the focused tile from what it shows, ⌘0 drops the override", () => {
    const a = stepTileFont({}, "x", 11, 1);
    expect(a).toEqual({ x: 12 });
    expect(stepTileFont(a, "x", 12, null)).toEqual({});
    expect(stepTileFont({}, "x", FONT_MAX, 1)).toEqual({ x: FONT_MAX });
  });
});
