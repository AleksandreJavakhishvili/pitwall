import { describe, expect, it } from "vitest";
import { commentStore, composePrompt, conflictPrompt } from "./comments";

describe("review comments", () => {
  it("composes one prompt in file/line order, text verbatim", () => {
    const text = composePrompt([
      { id: "2", path: "src/b.ts", line: 9, text: "rename this" },
      { id: "1", path: "src/a.ts", line: 30, text: "  keep  spacing\nand lines " },
      { id: "3", path: "src/a.ts", line: 4, text: "why?" },
    ]);
    expect(text).toBe(
      "Review comments:\n- src/a.ts:4 — why?\n- src/a.ts:30 —   keep  spacing\nand lines \n- src/b.ts:9 — rename this",
    );
  });

  it("stores comments per agent", () => {
    commentStore.add("a", { path: "x", line: 1, text: "one" });
    commentStore.add("b", { path: "y", line: 2, text: "two" });
    expect(commentStore.get().a).toHaveLength(1);
    const id = commentStore.get().a[0].id;
    commentStore.remove("a", id);
    expect(commentStore.get().a).toHaveLength(0);
    commentStore.clear("b");
    expect(commentStore.get().b).toBeUndefined();
  });

  it("offers a conflict prompt naming the branch", () => {
    expect(conflictPrompt("main")).toBe("Rebase onto main and resolve conflicts.");
  });
});
