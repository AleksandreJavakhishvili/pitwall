// The explorer's tree markup and the mock backend the demo uses.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { TreeModel, EMPTY_TREE, type TreeState } from "../../lib/explorer";
import type { FileEntry } from "../../explorerApi";
import { ExplorerTree } from "./ExplorerTree";
import { createExplorerMock } from "../../mockExplorer";
import type { AgentView, FileChange } from "../../types";

const e = (path: string, kind: FileEntry["kind"], extra: Partial<FileEntry> = {}): FileEntry => ({ name: path.split("/").pop()!, path, kind, status: null, changes: 0, ignored: false, ...extra });

describe("ExplorerTree", () => {
  it("shows change letters, a dot on folders with changes, ignored entries dimmed", () => {
    const state: TreeState = {
      ...EMPTY_TREE,
      expanded: ["src"],
      listings: {
        "": { dir: "", git: true, truncated: false, entries: [e("src", "dir", { changes: 2 }), e("dist", "dir", { ignored: true }), e("README.md", "file", { status: "M" })] },
        src: { dir: "src", git: true, truncated: true, entries: [e("src/new.ts", "file", { status: "U" })] },
      },
    };
    const html = renderToStaticMarkup(<ExplorerTree state={state} model={new TreeModel(() => new Promise(() => {}))} selected="src/new.ts" onOpen={() => {}} label="Files" />);
    expect(html).toContain('title="2 changes inside"');
    expect(html).toMatch(/data-ex-row="dist"[^>]*data-ignored="true"/);
    expect(html).toMatch(/data-status="U"[^>]*>U</);
    expect(html).toMatch(/data-ex-row="src\/new.ts"[^>]*aria-current="true"/);
    expect(html).toContain("Only the first 5 000 entries are shown");
    // Read-only: nothing to type into.
    expect(html).not.toMatch(/<input|<textarea|contenteditable/);
  });
});

describe("explorer mock", () => {
  const agent = { id: "a", name: "api-fix", projectDisplay: "orders-api" } as AgentView;
  const changes: FileChange[] = [
    { path: "src/orders/handler.ts", added: 2, removed: 1, untracked: false, binary: false },
    { path: "src/orders/legacy-handler.ts", added: 0, removed: 3, untracked: false, binary: false, status: "D" },
    { path: "test/new.test.ts", added: 4, removed: 0, untracked: true, binary: false },
  ];
  const api = createExplorerMock({ find: () => agent, changes: () => changes });

  it("lists like git: letters, counts, ignored only when asked", async () => {
    const root = await api.listFiles("a", "");
    expect(root.entries.map((x) => x.name)).not.toContain("node_modules");
    expect(root.entries.find((x) => x.name === "src")?.changes).toBe(1);
    const all = await api.listFiles("a", "", true);
    expect(all.entries.find((x) => x.name === "node_modules")?.ignored).toBe(true);
    const orders = await api.listFiles("a", "src/orders");
    expect(orders.entries.map((x) => [x.name, x.status])).toContainEqual(["handler.ts", "M"]);
    expect(orders.entries.map((x) => x.name)).not.toContain("legacy-handler.ts");
  });

  it("too large unless loaded anyway; search skips node_modules by default", async () => {
    expect((await api.readFile("a", "fixtures/orders-dump.json")).kind).toBe("tooLarge");
    expect((await api.readFile("a", "fixtures/orders-dump.json", true)).kind).toBe("text");
    const r = await api.searchFiles("a", { query: "fastify", regex: false, caseSensitive: false, wholeWord: false, include: [], exclude: [], maxResults: null, hidden: null, defaultExcludes: null });
    expect(r.matches.every((m) => !m.path.startsWith("node_modules/"))).toBe(true);
    expect(r.matches.length).toBeGreaterThan(0);
  });
});
