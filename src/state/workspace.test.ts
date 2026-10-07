import { describe, expect, it } from "vitest";
import { agentsIn, buildGrid, findPane, leaves } from "../layout/tree";
import {
  ALL_SPACE,
  applyPreset,
  createCustomSpace,
  defaultUiState,
  densityOf,
  dropOnPane,
  dropOnSpace,
  type DropFit,
  type UiState,
  getSpace,
  locate,
  placeAgent,
  pruneAgents,
  sanitize,
  setDensity,
  setHideElsewhere,
  setTheme,
  spaceMembers,
  tileAgents,
} from "./workspace";
import type { AgentView } from "../types";

describe("tileAgents", () => {
  it("tiles new agents in the All space, first one focused, the rest as chips", () => {
    const ids = ["a", "b", "c", "d", "e"];
    const s = tileAgents(defaultUiState(), ALL_SPACE, ids, 4);
    const all = getSpace(s, ALL_SPACE)!;
    expect(agentsIn(all.layout)).toEqual(["a", "b", "c", "d"]);
    expect(leaves(all.layout).find((p) => p.id === all.focusedPaneId)?.agentId).toBe("a");
    // "e" isn't in a pane but is still a member of All (shown as a chip).
    const agents = ids.map((id) => ({ id }) as AgentView);
    expect(spaceMembers(all, agents).map((a) => a.id)).toContain("e");
  });

  it("keeps what was shown after the new ones and moves agents out of other spaces", () => {
    let s = placeAgent(defaultUiState(), ALL_SPACE, "old");
    const { state, spaceId } = createCustomSpace(s, "main", "x");
    s = tileAgents(state, ALL_SPACE, ["x", "n"], 6);
    expect(agentsIn(getSpace(s, ALL_SPACE)!.layout)).toEqual(["x", "n", "old"]);
    expect(agentsIn(getSpace(s, spaceId)!.layout)).toEqual([]);
    expect(locate(s, "x")?.space.id).toBe(ALL_SPACE);
  });

  it("does nothing without agents", () => {
    const s = defaultUiState();
    expect(tileAgents(s, ALL_SPACE, [], 4)).toBe(s);
  });
});

describe("density and auto grid", () => {
  const agents = ["a", "b", "c", "d", "e", "f", "g"].map((id) => ({ id }) as AgentView);

  it("defaults to compact; a space can override and reset it", () => {
    let s = defaultUiState();
    expect(s.density).toBe("compact");
    s = setDensity(s, "dense", ALL_SPACE);
    expect(densityOf(s, getSpace(s, ALL_SPACE))).toBe("dense");
    s = setDensity(s, "comfortable");
    expect(s.density).toBe("comfortable");
    expect(densityOf(s, getSpace(s, ALL_SPACE))).toBe("dense");
    s = setDensity(s, null, ALL_SPACE);
    expect(densityOf(s, getSpace(s, ALL_SPACE))).toBe("comfortable");
  });

  it("persists density and tile fonts through sanitize, dropping junk", () => {
    const s = setDensity({ ...defaultUiState(), tileFont: { a: 11 } }, "dense", ALL_SPACE);
    const back = sanitize(JSON.parse(JSON.stringify(s)));
    expect(back.tileFont).toEqual({ a: 11 });
    expect(getSpace(back, ALL_SPACE)!.density).toBe("dense");
    const junk = sanitize({ ...JSON.parse(JSON.stringify(s)), density: "huge", tileFont: { a: "x" } });
    expect(junk.density).toBe("compact");
    expect(junk.tileFont).toEqual({});
    expect(sanitize({ v: 1 }).density).toBe("compact");
  });

  it("forgets tile fonts of removed agents", () => {
    const s = pruneAgents({ ...defaultUiState(), tileFont: { a: 11, gone: 12 } }, new Set(["a"]));
    expect(s.tileFont).toEqual({ a: 11 });
  });

  it("auto grid tiles every member it can, in the rows given", () => {
    const s = applyPreset(defaultUiState(), ALL_SPACE, "auto", agents, (n) => (n === 7 ? [4, 3] : [n]));
    expect(agentsIn(getSpace(s, ALL_SPACE)!.layout)).toEqual(["a", "b", "c", "d", "e", "f", "g"]);
    const capped = applyPreset(defaultUiState(), ALL_SPACE, "auto", agents, () => [2, 2]);
    expect(agentsIn(getSpace(capped, ALL_SPACE)!.layout)).toEqual(["a", "b", "c", "d"]);
  });

  it("grid presets fill up to their size", () => {
    const s = applyPreset(defaultUiState(), ALL_SPACE, "3x3", agents);
    const layout = getSpace(s, ALL_SPACE)!.layout;
    expect(leaves(layout)).toHaveLength(9);
    expect(agentsIn(layout)).toHaveLength(7);
  });
});

// docs/spec/layout.md "Revision: drag & drop must rearrange properly"
describe("drag & drop", () => {
  let n = 0;
  const ids = (p: string) => `${p}${n++}`;
  /** All space tiled with `all`; plus a custom space "c" tiled with `custom`. */
  const setup = (all: (string | null)[], custom: (string | null)[] = []) => {
    const base = defaultUiState();
    const allLayout = buildGrid([all.length], all, ids);
    const cLayout = buildGrid([Math.max(1, custom.length)], custom, ids);
    const s: UiState = {
      ...base,
      spaces: [
        { ...base.spaces[0], layout: allLayout, focusedPaneId: leaves(allLayout)[0].id },
        { id: "c", name: "C", kind: "custom", members: custom.filter((x): x is string => !!x), layout: cLayout, focusedPaneId: leaves(cLayout)[0].id, maximizedPaneId: null },
      ],
    };
    return s;
  };
  const pane = (s: UiState, space: string, agent: string | null, i = 0) =>
    leaves(getSpace(s, space)!.layout).filter((p) => p.agentId === agent)[i]!.id;
  const shown = (s: UiState, space: string) => agentsIn(getSpace(s, space)!.layout);
  const everywhere = (s: UiState) => s.spaces.flatMap((sp) => agentsIn(sp.layout));
  const fit = (w: number, h: number): DropFit => ({ area: { w, h }, min: { minW: 300, minH: 200 } });

  it("dropping an agent on its own pane is a no-op (centre or any side)", () => {
    const s = setup(["a", "b"]);
    for (const zone of ["center", "left", "right", "top", "bottom"] as const) {
      const r = dropOnPane(s, ALL_SPACE, "a", pane(s, ALL_SPACE, "a"), zone);
      expect(r.outcome).toBe("noop");
      expect(r.state).toBe(s);
    }
  });

  it("splits each side of the target pane, in the right direction and order", () => {
    const cases = [
      ["left", "row", ["x", "a"]],
      ["right", "row", ["a", "x"]],
      ["top", "col", ["x", "a"]],
      ["bottom", "col", ["a", "x"]],
    ] as const;
    for (const [zone, dir, order] of cases) {
      const s = setup(["a"]);
      const r = dropOnPane(s, ALL_SPACE, "x", pane(s, ALL_SPACE, "a"), zone);
      const layout = getSpace(r.state, ALL_SPACE)!.layout;
      expect(r.outcome).toBe("placed");
      expect(layout.type === "split" && layout.dir).toBe(dir);
      expect(agentsIn(layout)).toEqual(order);
      expect(getSpace(r.state, ALL_SPACE)!.focusedPaneId).toBe(r.paneId);
    }
  });

  it("moves, never duplicates: the source pane closes and its tree collapses", () => {
    const s = setup(["a", "b", "c"]);
    const r = dropOnPane(s, ALL_SPACE, "a", pane(s, ALL_SPACE, "c"), "bottom");
    expect(everywhere(r.state).filter((x) => x === "a")).toHaveLength(1);
    expect(leaves(getSpace(r.state, ALL_SPACE)!.layout)).toHaveLength(3); // no empty hole left behind
    const layout = getSpace(r.state, ALL_SPACE)!.layout;
    expect(layout.type === "split" && layout.dir).toBe("row");
    expect(agentsIn(layout)).toEqual(["b", "c", "a"]);
  });

  it("moving across spaces closes the pane in the source space and leaves it tidy", () => {
    const s = setup(["a"], ["x", "y"]);
    const r = dropOnPane(s, ALL_SPACE, "x", pane(s, ALL_SPACE, "a"), "right");
    expect(shown(r.state, ALL_SPACE)).toEqual(["a", "x"]);
    const c = getSpace(r.state, "c")!;
    expect(agentsIn(c.layout)).toEqual(["y"]);
    expect(c.layout.type).toBe("pane"); // collapsed, not a split with a hole
    expect(c.members).toEqual(["y"]); // moved out of the custom space
    expect(findPane(c.layout, c.focusedPaneId!)).not.toBeNull();
  });

  it("the last agent of a space leaves an empty pane, not a broken layout", () => {
    const s = setup(["a"], ["x"]);
    const r = dropOnPane(s, ALL_SPACE, "x", pane(s, ALL_SPACE, "a"), "left");
    const c = getSpace(r.state, "c")!;
    expect(leaves(c.layout)).toHaveLength(1);
    expect(agentsIn(c.layout)).toEqual([]);
  });

  it("centre swaps two agents of the same space", () => {
    const s = setup(["a", "b", "c"]);
    const r = dropOnPane(s, ALL_SPACE, "a", pane(s, ALL_SPACE, "c"), "center");
    expect(r.outcome).toBe("swapped");
    expect(shown(r.state, ALL_SPACE)).toEqual(["c", "b", "a"]);
  });

  it("centre swaps across spaces: the displaced agent takes the dragged one's old pane", () => {
    const s = setup(["a", "b"], ["x"]);
    const r = dropOnPane(s, ALL_SPACE, "x", pane(s, ALL_SPACE, "b"), "center");
    expect(r.outcome).toBe("swapped");
    expect(shown(r.state, ALL_SPACE)).toEqual(["a", "x"]);
    expect(shown(r.state, "c")).toEqual(["b"]);
    expect(getSpace(r.state, "c")!.members).toEqual(["b"]);
  });

  it("centre replaces when the dragged agent isn't shown anywhere (the old one stays a chip)", () => {
    let s = setup(["a"], ["x"]);
    s = { ...s, spaces: s.spaces.map((sp) => (sp.id === "c" ? { ...sp, members: ["x"] } : sp)) };
    const r = dropOnPane(s, "c", "z", pane(s, "c", "x"), "center");
    expect(r.outcome).toBe("replaced");
    expect(shown(r.state, "c")).toEqual(["z"]);
    expect(getSpace(r.state, "c")!.members).toEqual(expect.arrayContaining(["x", "z"]));
  });

  it("dropping on an empty pane fills it; a side zone on an empty pane also just fills it", () => {
    const s = setup(["a", null]);
    const r = dropOnPane(s, ALL_SPACE, "z", pane(s, ALL_SPACE, null), "left");
    expect(r.outcome).toBe("placed");
    expect(shown(r.state, ALL_SPACE)).toEqual(["a", "z"]);
    expect(leaves(getSpace(r.state, ALL_SPACE)!.layout)).toHaveLength(2);
  });

  it("moving into an empty pane of the same space closes the old pane", () => {
    const s = setup(["a", "b", null]);
    const r = dropOnPane(s, ALL_SPACE, "a", pane(s, ALL_SPACE, null), "center");
    expect(shown(r.state, ALL_SPACE)).toEqual(["b", "a"]);
    expect(leaves(getSpace(r.state, ALL_SPACE)!.layout)).toHaveLength(2);
  });

  it("drop on a space tab auto-places: empty pane first, else a split — never replaces", () => {
    const s = setup(["a", "b"], [null]);
    const r1 = dropOnSpace(s, "c", "a");
    expect(shown(r1.state, "c")).toEqual(["a"]);
    expect(shown(r1.state, ALL_SPACE)).toEqual(["b"]); // the All layout closed the gap
    expect(leaves(getSpace(r1.state, ALL_SPACE)!.layout)).toHaveLength(1);
    const r2 = dropOnSpace(r1.state, "c", "b");
    expect(shown(r2.state, "c")).toEqual(["a", "b"]);
    expect(getSpace(r2.state, "c")!.members).toEqual(["a", "b"]);
    expect(getSpace(r2.state, "c")!.focusedPaneId).toBe(r2.paneId);
  });

  it("drop on the tab of the space already showing it only focuses it", () => {
    const s = setup(["a", "b"]);
    const r = dropOnSpace(s, ALL_SPACE, "b");
    expect(r.outcome).toBe("noop");
    expect(shown(r.state, ALL_SPACE)).toEqual(["a", "b"]);
    expect(getSpace(r.state, ALL_SPACE)!.focusedPaneId).toBe(pane(s, ALL_SPACE, "b"));
  });

  it("auto-place splits the roomiest pane along its long side", () => {
    const s = setup(["a", "b"]); // two side-by-side panes of 800×900
    const r = dropOnSpace(s, ALL_SPACE, "x", fit(1600, 900));
    const layout = getSpace(r.state, ALL_SPACE)!.layout;
    expect(agentsIn(layout)).toEqual(["a", "x", "b"]);
    expect(layout.type === "split" && layout.children[0].type === "split" && layout.children[0].dir).toBe("col");
  });

  it("respects density: a split that would be too small becomes a chip", () => {
    const s = setup(["a", "b"], ["x"]);
    // 2 panes of 400px wide; splitting one sideways would give 200px < 300px.
    const r = dropOnPane(s, ALL_SPACE, "x", pane(s, ALL_SPACE, "a"), "right", fit(800, 300));
    expect(r.outcome).toBe("chip");
    expect(r.paneId).toBeNull();
    expect(shown(r.state, ALL_SPACE)).toEqual(["a", "b"]); // nothing else got squeezed out
    expect(everywhere(r.state)).not.toContain("x"); // moved out of its old pane
    expect(getSpace(r.state, "c")!.members).toEqual([]);
    // With room it splits.
    expect(dropOnPane(s, ALL_SPACE, "x", pane(s, ALL_SPACE, "a"), "right", fit(1600, 300)).outcome).toBe("placed");
  });

  it("respects density on a tab drop too", () => {
    const s = setup(["a", "b"], ["x"]);
    const r = dropOnSpace(s, ALL_SPACE, "x", fit(800, 300));
    expect(r.outcome).toBe("chip");
    expect(shown(r.state, ALL_SPACE)).toEqual(["a", "b"]);
    const custom = dropOnSpace(setup(["x"], ["p", "q"]), "c", "x", fit(700, 300));
    expect(custom.outcome).toBe("chip");
    expect(getSpace(custom.state, "c")!.members).toContain("x"); // offered as a chip there
  });

  it("a new space made from a drop takes the agent out of where it was", () => {
    const s = setup(["a", "b"]);
    const { state, spaceId } = createCustomSpace(s, "main", "a");
    expect(shown(state, spaceId)).toEqual(["a"]);
    expect(shown(state, ALL_SPACE)).toEqual(["b"]);
  });
});

describe("hideElsewhere", () => {
  it("defaults to shown, survives sanitize, and toggles without churn", () => {
    const d = defaultUiState();
    expect(d.hideElsewhere).toBe(false);
    const hidden = setHideElsewhere(d, true);
    expect(hidden.hideElsewhere).toBe(true);
    expect(setHideElsewhere(hidden, true)).toBe(hidden);
    expect(sanitize(JSON.parse(JSON.stringify(hidden))).hideElsewhere).toBe(true);
    expect(sanitize({ ...hidden, hideElsewhere: "yes" }).hideElsewhere).toBe(false);
  });
});

describe("theme", () => {
  it("defaults to System, survives sanitize, rejects junk, and sets without churn", () => {
    const d = defaultUiState();
    expect(d.theme).toBe("system");
    const light = setTheme(d, "light");
    expect(light.theme).toBe("light");
    expect(setTheme(light, "light")).toBe(light);
    expect(sanitize(JSON.parse(JSON.stringify(light))).theme).toBe("light");
    expect(sanitize({ ...light, theme: "solarized" }).theme).toBe("system");
    const { theme: _, ...old } = light;
    expect(sanitize(old).theme).toBe("system");
  });
});
