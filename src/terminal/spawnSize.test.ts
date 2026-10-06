import { describe, expect, it } from "vitest";
import { AREA_PAD, DIVIDER, PANE_CHROME, fitCells, tileBox } from "./spawnSize";

const cell = { w: 8, h: 15 };

describe("fitCells", () => {
  it("fits like FitAddon, rounding down", () => {
    const box = { w: PANE_CHROME.w + 8 * 100 + 7, h: PANE_CHROME.h + 15 * 30 + 14 };
    expect(fitCells(box, cell)).toEqual({ cols: 100, rows: 30 });
  });
  it("gives up on unusable input", () => {
    expect(fitCells({ w: 10, h: 10 }, cell)).toBeNull();
    expect(fitCells({ w: 800, h: 600 }, { w: 0, h: 15 })).toBeNull();
  });
});

describe("tileBox", () => {
  const area = { w: 1212, h: 812 };
  const W = area.w - 2 * AREA_PAD;
  const H = area.h - 2 * AREA_PAD;
  it("one agent gets the whole area", () => {
    expect(tileBox(area, 0, 1)).toEqual({ w: W, h: H });
  });
  it("splits for presets", () => {
    expect(tileBox(area, 1, 2)).toEqual({ w: (W - DIVIDER) / 2, h: H });
    expect(tileBox(area, 0, 3).w).toBeCloseTo((W - DIVIDER) * 0.6);
    expect(tileBox(area, 2, 3).h).toBe((H - DIVIDER) / 2);
    expect(tileBox(area, 3, 4)).toEqual({ w: (W - DIVIDER) / 2, h: (H - DIVIDER) / 2 });
    expect(tileBox(area, 5, 6).w).toBe((W - 2 * DIVIDER) / 3);
  });
  it("overflow agents get the smallest tile", () => {
    expect(tileBox(area, 9, 9)).toEqual(tileBox(area, 5, 6));
  });
});
