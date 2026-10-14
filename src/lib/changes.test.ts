import { describe, expect, it } from "vitest";
import type { FileChange } from "../types";
import { changeGroups, shownPath, stageGroups, stageOf } from "./changes";

const f = (path: string, p: Partial<FileChange> = {}): FileChange => ({ path, added: 1, removed: 0, untracked: false, binary: false, ...p });

describe("Changes sections", () => {
  it("splits staged and not staged; a file both staged and changed is in both", () => {
    const both = f("a.ts", { added: 5, removed: 2, parts: { staged: { added: 3, removed: 1 }, unstaged: { added: 2, removed: 1 } } });
    const staged = f("b.ts", { parts: { staged: { added: 1, removed: 0, status: "A" }, unstaged: null } });
    const loose = f("c.ts");
    const g = stageGroups([both, staged, loose]);
    expect(g.map((x) => [x.stage, x.files.map((y) => y.path)])).toEqual([
      ["staged", ["a.ts", "b.ts"]],
      ["unstaged", ["a.ts", "c.ts"]],
    ]);
    expect(g[0].files[0].added).toBe(3);
    expect(g.flatMap((x) => x.files.map(stageOf))).toEqual(["staged", "staged", "unstaged", "unstaged"]);
    expect(stageOf(both)).toBeNull();
  });

  it("lists everything without headers when nothing is staged", () => {
    const g = stageGroups([f("a.ts", { parts: { staged: null, unstaged: { added: 1, removed: 0 } } })]);
    expect(g).toHaveLength(1);
    expect(g[0].stage).toBeNull();
  });

  it("puts the agent's own repository first, then the ones inside its folder", () => {
    const g = changeGroups([f("vendor/sdk/x.ts", { repo: "vendor/sdk" }), f("src/a.ts"), f("libs/b/y.ts", { repo: "libs/b" })]);
    expect(g.map((x) => x.repo)).toEqual([null, "libs/b", "vendor/sdk"]);
    expect(shownPath(g[2].files[0])).toBe("x.ts");
  });
});
