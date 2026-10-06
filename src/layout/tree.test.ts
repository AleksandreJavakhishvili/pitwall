import { describe, expect, it } from "vitest";
import {
  agentsIn,
  autoGrid,
  availablePresets,
  buildGrid,
  buildPreset,
  evenRows,
  presetFits,
  dropAgent,
  dropZone,
  fitLayout,
  leaves,
  removePane,
  resizeSplit,
  setPaneAgent,
  splitPane,
  swapPanes,
  type LayoutNode,
} from "./tree";

const ids = () => {
  let n = 0;
  return (p: string) => `${p}${++n}`;
};

describe("layout tree", () => {
  it("builds presets with the right pane counts", () => {
    const g = ids();
    expect(leaves(buildPreset("1", ["a"], g))).toHaveLength(1);
    expect(leaves(buildPreset("3", ["a", "b", "c"], g))).toHaveLength(3);
    expect(leaves(buildPreset("2x2", ["a"], g))).toHaveLength(4);
    expect(agentsIn(buildPreset("3x2", ["a", "b"], g))).toEqual(["a", "b"]);
  });

  it("splits a pane and keeps one agent per pane", () => {
    const g = ids();
    const t = buildPreset("1", ["a"], g);
    const { tree } = splitPane(t, leaves(t)[0].id, "right", "b", g);
    expect(agentsIn(tree)).toEqual(["a", "b"]);
    const { tree: t2 } = splitPane(tree, leaves(tree)[0].id, "left", "c", g);
    // same direction → flat row of three
    expect(t2.type === "split" && t2.children.length).toBe(3);
    expect(agentsIn(t2)).toEqual(["c", "a", "b"]);
    const s = t2.type === "split" ? t2.sizes.reduce((x, y) => x + y, 0) : 1;
    expect(s).toBeCloseTo(1);
  });

  it("moving an agent by splitting removes its old pane", () => {
    const g = ids();
    const t = buildPreset("2", ["a", "b"], g);
    const [pa] = leaves(t);
    const { tree } = splitPane(t, pa.id, "bottom", "b", g);
    expect(agentsIn(tree)).toEqual(["a", "b"]);
    expect(leaves(tree)).toHaveLength(2);
  });

  it("removes panes and collapses splits", () => {
    const g = ids();
    const t = buildPreset("3", ["a", "b", "c"], g);
    const b = leaves(t)[1];
    const r = removePane(t, b.id) as LayoutNode;
    expect(agentsIn(r)).toEqual(["a", "c"]);
    expect(r.type === "split" && r.children.every((c) => c.type === "pane")).toBe(true);
    const single = buildPreset("1", ["a"], g);
    expect(removePane(single, single.id)).toBeNull();
  });

  it("dropAgent empties the last pane instead of deleting it", () => {
    const g = ids();
    const t = buildPreset("1", ["a"], g);
    const r = dropAgent(t, "a")!;
    expect(leaves(r)).toHaveLength(1);
    expect(agentsIn(r)).toEqual([]);
  });

  it("setPaneAgent and swapPanes keep agents unique", () => {
    const g = ids();
    const t = buildPreset("2", ["a", "b"], g);
    const [p1, p2] = leaves(t);
    expect(agentsIn(setPaneAgent(t, p1.id, "b"))).toEqual(["b"]);
    expect(agentsIn(swapPanes(t, p1.id, p2.id))).toEqual(["b", "a"]);
  });

  it("resizes within bounds", () => {
    const g = ids();
    const t = buildPreset("2", ["a", "b"], g);
    const r = resizeSplit(t, t.id, 0, 0.9);
    expect(r.type === "split" && r.sizes[1]).toBeCloseTo(0.08);
  });

  it("fitLayout folds panes that are too small", () => {
    const g = ids();
    const t = buildPreset("3x2", ["a", "b", "c", "d", "e", "f"], g);
    const keep = leaves(t)[0].id;
    const { tree, hidden } = fitLayout(t, { w: 1300, h: 800 }, { minW: 600, minH: 300, maxPerRow: 3, keep });
    expect(leaves(tree)).toHaveLength(4);
    expect(hidden).toHaveLength(2);
    expect(leaves(tree).some((p) => p.id === keep)).toBe(true);
    const one = fitLayout(t, { w: 900, h: 800 }, { minW: 600, minH: 300, maxPerRow: 1, keep });
    expect(leaves(one.tree)).toHaveLength(2); // two stacked rows of one
  });

  it("maps pointer positions to drop zones", () => {
    expect(dropZone(0.05, 0.5)).toBe("left");
    expect(dropZone(0.5, 0.95)).toBe("bottom");
    expect(dropZone(0.5, 0.5)).toBe("center");
  });

  it("builds the bigger grid presets", () => {
    const g = ids();
    expect(leaves(buildPreset("3x3", [], g))).toHaveLength(9);
    expect(leaves(buildPreset("4x3", [], g))).toHaveLength(12);
    const t = buildPreset("4x4", ["a", "b"], g);
    expect(leaves(t)).toHaveLength(16);
    expect(agentsIn(t)).toEqual(["a", "b"]);
  });

  it("buildGrid lays out uneven rows in reading order", () => {
    const g = ids();
    const t = buildGrid([3, 2], ["a", "b", "c", "d", "e"], g);
    expect(agentsIn(t)).toEqual(["a", "b", "c", "d", "e"]);
    expect(t.type === "split" && t.dir).toBe("col");
    expect(leaves(buildGrid([1], ["a"], g))).toHaveLength(1);
    expect(buildGrid([1], ["a"], g).type).toBe("pane");
  });

  it("spreads panes evenly over rows", () => {
    expect(evenRows(5, 2)).toEqual([3, 2]);
    expect(evenRows(7, 3)).toEqual([3, 2, 2]);
    expect(evenRows(2, 5)).toEqual([1, 1]);
  });

  it("auto grid fits all agents evenly, preferring the breakpoint's columns", () => {
    const min = { minW: 300, minH: 200 };
    const area = { w: 1500, h: 900 }; // room for 5 × 4
    expect(autoGrid(1, area, min, 3)).toEqual([1]);
    expect(autoGrid(4, area, min, 3)).toEqual([2, 2]);
    expect(autoGrid(6, area, min, 3)).toEqual([3, 3]);
    expect(autoGrid(9, area, min, 3)).toEqual([3, 3, 3]);
    // More agents than 3 per row can hold at this height: the default is exceeded, not a cap.
    expect(autoGrid(16, area, min, 3)).toEqual([4, 4, 4, 4]);
  });

  it("auto grid tiles what fits and leaves the rest as chips", () => {
    const rows = autoGrid(30, { w: 1200, h: 600 }, { minW: 300, minH: 200 }, 4);
    expect(rows).toEqual([4, 4, 4]);
    expect(autoGrid(5, { w: 200, h: 100 }, { minW: 300, minH: 200 }, 2)).toEqual([1]);
  });

  it("offers presets only when the window fits them", () => {
    const min = { minW: 300, minH: 200 };
    expect(presetFits("4x4", { w: 1300, h: 820 }, min)).toBe(true);
    expect(presetFits("4x4", { w: 1100, h: 820 }, min)).toBe(false);
    const small = availablePresets({ w: 700, h: 450 }, min);
    expect(small).toContain("2x2");
    expect(small).toContain("auto");
    expect(small).not.toContain("3x3");
    expect(availablePresets(null, min)).toContain("4x4");
  });

  it("fitLayout without maxPerRow is limited only by the minimum size", () => {
    const g = ids();
    const t = buildPreset("4x4", [], g);
    expect(leaves(fitLayout(t, { w: 1300, h: 820 }, { minW: 300, minH: 200 }).tree)).toHaveLength(16);
    expect(leaves(fitLayout(t, { w: 1300, h: 820 }, { minW: 300, minH: 200, maxPerRow: 2 }).tree).length).toBeLessThan(16);
  });
});
