import { describe, expect, it } from "vitest";
import { Chunk } from "@codemirror/merge";
import { Text } from "@codemirror/state";
import { lineDiff } from "./lineDiff";

const conf = { override: lineDiff({}) };

/** Changed line ranges (1-based, inclusive; null when empty) per chunk. */
function lines(a: string, b: string) {
  const ta = Text.of(a.split("\n"));
  const tb = Text.of(b.split("\n"));
  return Chunk.build(ta, tb, conf).map((c) => [
    c.fromA < c.toA ? [ta.lineAt(c.fromA).number, ta.lineAt(c.endA).number] : null,
    c.fromB < c.toB ? [tb.lineAt(c.fromB).number, tb.lineAt(c.endB).number] : null,
  ]);
}

describe("lineDiff", () => {
  it("puts an inserted block before the blank line, as Monaco does", () => {
    const a = "fn() {\n  a;\n\n  b;\n}\n";
    const b = "fn() {\n  a;\n\n  if (x) {\n    y;\n  }\n\n  b;\n}\n";
    // Monaco: lines 3–6 (blank, if, y, }) inserted after "a;".
    expect(lines(a, b)).toEqual([[null, [3, 6]]]);
  });

  it("diffs characters inside replaced lines", () => {
    const a = "x\nconst c = items[0].currency;\ny\n";
    const b = "x\nconst c = items[0]?.currency;\ny\n";
    const [chunk] = Chunk.build(Text.of(a.split("\n")), Text.of(b.split("\n")), conf);
    expect(chunk.changes.map((c) => b.slice(chunk.fromB + c.fromB, chunk.fromB + c.toB))).toEqual(["?"]);
  });

  it("handles insertions and deletions at the start and the end", () => {
    expect(lines("a\nb\n", "a\nb\nc\n")).toEqual([[null, [3, 3]]]);
    expect(lines("a\nb\nc\n", "a\nb\n")).toEqual([[[3, 3], null]]);
    expect(lines("b\n", "a\nb\n")).toEqual([[null, [1, 1]]]);
  });

  it("treats a new or deleted file as one change, the empty side's line included", () => {
    expect(lines("", "a")).toEqual([[[1, 1], [1, 1]]]);
    expect(lines("", "a\nb\n")).toEqual([[[1, 1], [1, 2]]]);
    expect(lines("a\nb\n", "")).toEqual([[[1, 2], [1, 1]]]);
  });
});
