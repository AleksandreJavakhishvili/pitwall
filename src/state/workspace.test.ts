import { describe, expect, it } from "vitest";
import { agentsIn, leaves } from "../layout/tree";
import {
  ALL_SPACE,
  applyPreset,
  createCustomSpace,
  defaultUiState,
  densityOf,
  getSpace,
  locate,
  placeAgent,
  pruneAgents,
  sanitize,
  setDensity,
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
