// The code explorer's UI logic (lib/explorer.ts): quick open, the lazy tree, search grouping.
import { describe, expect, it } from "vitest";
import type { DirListing, FileEntry, SearchMatch } from "../explorerApi";
import {
  absolutePath,
  findEntry,
  fuzzyMatch,
  globList,
  groupMatches,
  highlightRuns,
  matchRuns,
  parentDirs,
  quickOpen,
  sizeLabel,
  TreeModel,
  treeRows,
} from "./explorer";

const FILES = [
  "README.md",
  "package.json",
  "src/app.ts",
  "src/orders/handler.ts",
  "src/orders/pricing.ts",
  "src/orders/validation/cart.ts",
  "test/orders/pricing.test.ts",
  "docs/api.md",
];

describe("quick open", () => {
  it("needs every character in order, case-insensitively", () => {
    expect(fuzzyMatch("src/orders/handler.ts", "SOH")).not.toBeNull();
    expect(fuzzyMatch("src/orders/handler.ts", "hos")).toBeNull();
  });

  it("ranks a file-name match first, then shorter paths", () => {
    const hits = quickOpen(FILES, "pric").map((h) => h.path);
    expect(hits).toEqual(["src/orders/pricing.ts", "test/orders/pricing.test.ts"]);
    expect(quickOpen(FILES, "handler")[0].path).toBe("src/orders/handler.ts");
    // Scattered matches still count, below the name matches.
    expect(quickOpen(FILES, "ovc").map((h) => h.path)).toContain("src/orders/validation/cart.ts");
  });

  it("matches space-separated parts anywhere", () => {
    const hits = quickOpen(FILES, "orders cart").map((h) => h.path);
    expect(hits[0]).toBe("src/orders/validation/cart.ts");
    expect(hits).not.toContain("README.md");
  });

  it("lists recent files first when nothing is typed, capped", () => {
    const hits = quickOpen(FILES, "  ", { recent: ["docs/api.md", "gone.ts"], limit: 3 });
    expect(hits.map((h) => h.path)).toEqual(["docs/api.md", "README.md", "package.json"]);
  });

  it("marks the matched characters for highlighting", () => {
    const h = fuzzyMatch("src/app.ts", "app")!;
    expect(h.positions).toEqual([4, 5, 6]);
    expect(highlightRuns("app.ts", h.positions, 4)).toEqual([
      { text: "app", hit: true },
      { text: ".ts", hit: false },
    ]);
  });
});

describe("paths and labels", () => {
  it("parents, absolute paths, sizes, globs", () => {
    expect(parentDirs("src/a/b.ts")).toEqual(["src", "src/a"]);
    expect(parentDirs("b.ts")).toEqual([]);
    expect(absolutePath("/opt/agentworks/workspaces/w/", "src/a.ts")).toBe("/opt/agentworks/workspaces/w/src/a.ts");
    expect(absolutePath("C:\\code\\app", "src/a.ts")).toBe("C:\\code\\app\\src\\a.ts");
    expect(sizeLabel(512)).toBe("512 B");
    expect(sizeLabel(3_480_000)).toBe("3.3 MB");
    expect(globList(" *.ts, src/** ,, ")).toEqual(["*.ts", "src/**"]);
  });
});

const entry = (path: string, kind: FileEntry["kind"] = "file", extra: Partial<FileEntry> = {}): FileEntry => ({
  name: path.split("/").pop()!,
  path,
  kind,
  status: null,
  changes: 0,
  ignored: false,
  ...extra,
});
const listing = (dir: string, entries: FileEntry[]): DirListing => ({ dir, entries, truncated: false, git: true });

function fakeFs(): { load(dir: string): Promise<DirListing>; calls: string[]; dirs: Record<string, FileEntry[]> } {
  const dirs: Record<string, FileEntry[]> = {
    "": [entry("src", "dir", { changes: 1 }), entry("README.md", "file", { status: "M" })],
    src: [entry("src/orders", "dir", { changes: 1 }), entry("src/app.ts")],
    "src/orders": [entry("src/orders/handler.ts", "file", { status: "U" })],
  };
  const calls: string[] = [];
  return {
    dirs,
    calls,
    load: (dir) => {
      calls.push(dir);
      return dirs[dir] ? Promise.resolve(listing(dir, dirs[dir])) : Promise.reject(`${dir}: not found`);
    },
  };
}

describe("the lazy tree", () => {
  it("lists the agent's folder, then folders as they open", async () => {
    const fs = fakeFs();
    const m = new TreeModel(fs.load, () => 42);
    await m.refresh();
    expect(m.state.updatedAt).toBe(42);
    expect(treeRows(m.state).map((r) => (r.kind === "entry" ? r.entry.path : r.kind))).toEqual(["src", "README.md"]);
    await m.toggle("src");
    expect(treeRows(m.state).map((r) => (r.kind === "entry" ? `${r.depth}:${r.entry.path}` : r.kind))).toEqual([
      "0:src",
      "1:src/orders",
      "1:src/app.ts",
      "0:README.md",
    ]);
    m.collapse("src");
    expect(treeRows(m.state)).toHaveLength(2);
    // Opening again uses what was read.
    await m.expand("src");
    expect(fs.calls).toEqual(["", "src"]);
  });

  it("shows a folder as loading until listed", () => {
    const m = new TreeModel(() => new Promise(() => {}));
    void m.expand("src");
    expect(m.state.loading).toEqual(["src"]);
    expect(treeRows({ ...m.state, listings: { "": listing("", [entry("src", "dir")]) } }).map((r) => r.kind)).toEqual(["entry", "loading"]);
  });

  it("reveals a file by opening its folders, and finds its entry", async () => {
    const m = new TreeModel(fakeFs().load);
    await m.refresh();
    await m.reveal("src/orders/handler.ts");
    expect(m.state.expanded).toEqual(["src", "src/orders"]);
    expect(findEntry(m.state, "src/orders/handler.ts")?.status).toBe("U");
    expect(findEntry(m.state, "nope.ts")).toBeNull();
  });

  it("refresh reads every open folder again; folders that vanished close", async () => {
    const fs = fakeFs();
    const m = new TreeModel(fs.load);
    await m.refresh();
    await m.reveal("src/orders/handler.ts");
    delete fs.dirs["src/orders"];
    fs.calls.length = 0;
    await Promise.all([m.refresh(), m.refresh()]); // coalesced
    expect(fs.calls.sort()).toEqual(["", "src", "src/orders"]);
    expect(m.state.expanded).toEqual(["src"]);
    expect(m.state.refreshing).toBe(false);
  });

  it("keeps an error next to the folder that failed", async () => {
    const m = new TreeModel((dir) => (dir === "" ? Promise.resolve(listing("", [entry("x", "dir")])) : Promise.reject("x: permission denied")));
    await m.refresh();
    await m.expand("x");
    expect(treeRows(m.state).map((r) => r.kind)).toEqual(["entry", "error"]);
  });

  it("a new loader (Show ignored files) forgets what was read and reads again", async () => {
    const m = new TreeModel(fakeFs().load);
    await m.refresh();
    await m.setLoader(() => Promise.resolve(listing("", [entry("node_modules", "dir", { ignored: true })])));
    expect(m.state.listings[""].entries.map((e) => e.name)).toEqual(["node_modules"]);
  });
});

describe("search results", () => {
  const m = (path: string, line: number, text: string, ranges: [number, number][]): SearchMatch => ({
    path,
    line,
    column: ranges[0][0] + 1,
    text,
    textOffset: 0,
    ranges: ranges.map(([start, end]) => ({ start, end })),
  });

  it("groups by file in first-seen order and splits lines into runs", () => {
    const g = groupMatches([m("b.ts", 1, "x", [[0, 1]]), m("a.ts", 2, "x", [[0, 1]]), m("b.ts", 5, "x", [[0, 1]])]);
    expect(g.map((x) => [x.path, x.matches.map((y) => y.line)])).toEqual([
      ["b.ts", [1, 5]],
      ["a.ts", [2]],
    ]);
    expect(matchRuns(m("a.ts", 1, "a currency and currency", [[2, 10], [15, 23]]))).toEqual([
      { text: "a ", hit: false },
      { text: "currency", hit: true },
      { text: " and ", hit: false },
      { text: "currency", hit: true },
    ]);
  });
});
