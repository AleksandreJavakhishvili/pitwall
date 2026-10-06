import { describe, expect, it } from "vitest";
import type { FileChange } from "../types";
import { buildTree, fileStatus, treeOrder } from "./fileTree";
import { fileIconName, folderIconName, iconUrl } from "./fileIcons";

const fc = (path: string, extra: Partial<FileChange> = {}): FileChange => ({ path, added: 1, removed: 0, untracked: false, binary: false, ...extra });

describe("fileStatus", () => {
  it("prefers the backend status and degrades to U/M", () => {
    expect(fileStatus(fc("a", { status: "D" }))).toBe("D");
    expect(fileStatus(fc("a", { untracked: true }))).toBe("U");
    expect(fileStatus(fc("a"))).toBe("M");
  });
});

describe("buildTree", () => {
  it("compacts single-child folders and sorts folders first", () => {
    const t = buildTree([fc("src/components/review/Review.tsx"), fc("src/components/review/a.ts"), fc("README.md"), fc("src/main.ts"), fc("src/components/x/y.ts")]);
    expect(t.map((n) => n.name)).toEqual(["src", "README.md"]);
    const src = t[0];
    if (src.kind !== "dir") throw new Error();
    expect(src.children.map((n) => n.name)).toEqual(["components", "main.ts"]);
    const comp = src.children[0];
    if (comp.kind !== "dir") throw new Error();
    expect(comp.children.map((n) => [n.name, n.path])).toEqual([
      ["review", "src/components/review"],
      ["x", "src/components/x"],
    ]);
  });
  it("puts a whole single chain on one row", () => {
    const t = buildTree([fc("src/components/review/Review.tsx")]);
    expect(t[0].name).toBe("src/components/review");
    expect(treeOrder([fc("b.ts"), fc("a/z.ts"), fc("a.ts")]).map((f) => f.path)).toEqual(["a/z.ts", "a.ts", "b.ts"]);
  });
});

describe("file icons", () => {
  it("maps exact names, then extensions, then generic", () => {
    expect(fileIconName("package.json")).toBe("nodejs");
    expect(fileIconName("crates/core/Cargo.toml")).toBe("rust");
    expect(fileIconName("README.md")).toBe("readme");
    expect(fileIconName("src/a.test.ts")).toBe("test-ts");
    expect(fileIconName("src/App.tsx")).toBe("react_ts");
    expect(fileIconName("x.unknownext")).toBe("file");
    expect(folderIconName("src/components", true)).toBe("folder-components-open");
    expect(folderIconName("whatever", false)).toBe("folder");
    expect(iconUrl("nope")).toBe(iconUrl("file"));
    expect(iconUrl("file")).toBeTruthy();
  });
});
