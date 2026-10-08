// Diffs lines first, then characters inside changed lines — the way Monaco's
// diff does — so changes line up as they did before: an inserted block that
// could sit either side of a blank line goes where Monaco put it (the best
// "boundary score" by indentation, earliest on a tie).
import { Change, diff, presentableDiff, type DiffConfig } from "@codemirror/merge";

/** Lines become single characters (Unicode, outside the surrogates), so the character diff diffs lines. */
const CODES = 0xd800 + (0x10000 - 0xe000);
const code = (i: number) => String.fromCharCode(i < 0xd800 ? i : i + 0x800);

const indentation = (line: string) => /^[ \t]*/.exec(line)![0].length;

interface Range {
  /** Lines [a0, a1) of the original replaced by [b0, b1) of the modified. */
  a0: number;
  a1: number;
  b0: number;
  b1: number;
}

function boundary(lines: string[], at: number) {
  const before = at === 0 ? 0 : indentation(lines[at - 1]);
  const after = at === lines.length ? 0 : indentation(lines[at]);
  return 1000 - (before + after);
}

/** Slide pure insertions/deletions along equal lines to the best boundary (Monaco's shiftSequenceDiffs). */
function shift(ranges: Range[], a: string[], b: string[]) {
  for (let i = 0; i < ranges.length; i++) {
    const r = ranges[i];
    const ins = r.a0 === r.a1;
    if (ins === (r.b0 === r.b1)) continue;
    const [seq, s, e] = ins ? [b, r.b0, r.b1] : [a, r.a0, r.a1];
    const prevEnd = i ? (ins ? ranges[i - 1].b1 : ranges[i - 1].a1) : 0;
    const nextStart = i + 1 < ranges.length ? (ins ? ranges[i + 1].b0 : ranges[i + 1].a0) : seq.length;
    let before = 0;
    while (s - before - 1 >= prevEnd && seq[s - before - 1] === seq[e - before - 1]) before++;
    let after = 0;
    while (e + after < nextStart && seq[s + after] === seq[e + after]) after++;
    if (!before && !after) continue;
    let best = 0;
    let bestScore = -Infinity;
    for (let d = -before; d <= after; d++) {
      const other = ins ? r.a0 + d : r.b0 + d;
      const score = boundary(seq, s + d) + boundary(seq, e + d) + 2 * boundary(ins ? a : b, other);
      if (score > bestScore) [best, bestScore] = [d, score];
    }
    ranges[i] = { a0: r.a0 + best, a1: r.a1 + best, b0: r.b0 + best, b1: r.b1 + best };
  }
}

export function lineDiff(config: DiffConfig) {
  return (textA: string, textB: string): readonly Change[] => {
    // A new or deleted file: Monaco shows one change spanning both sides (the empty side's
    // line included). Leaving the final newline out makes merge's chunk cover that line.
    if (!textA !== !textB) {
      const end = (t: string) => (t.endsWith("\n") ? t.length - 1 : t.length);
      return [new Change(0, end(textA), 0, end(textB))];
    }
    const a = textA.split("\n");
    const b = textB.split("\n");
    const ids = new Map<string, string>();
    const enc = (lines: string[]) => lines.map((l) => ids.get(l) ?? (ids.set(l, code(ids.size)), ids.get(l)!)).join("");
    const encA = enc(a);
    const encB = enc(b);
    if (ids.size >= CODES) return presentableDiff(textA, textB, config);
    const ranges: Range[] = diff(encA, encB, config).map((c) => ({ a0: c.fromA, a1: c.toA, b0: c.fromB, b1: c.toB }));
    shift(ranges, a, b);

    const starts = (lines: string[]) => {
      const out = [0];
      for (const l of lines) out.push(out[out.length - 1] + l.length + 1);
      return out;
    };
    const sa = starts(a);
    const sb = starts(b);
    const changes: Change[] = [];
    for (const r of ranges) {
      if ((r.a0 === r.a1 || r.b0 === r.b1) && r.a0 > 0 && r.b0 > 0) {
        // Whole lines in or out: from the end of the line before (newline first), which is
        // how merge reads "these lines", also next to empty lines and at the end of the file.
        changes.push(new Change(sa[r.a0] - 1, sa[r.a1] - 1, sb[r.b0] - 1, sb[r.b1] - 1));
        continue;
      }
      const fromA = sa[r.a0];
      const toA = r.a1 < a.length ? sa[r.a1] : textA.length;
      const fromB = sb[r.b0];
      const toB = r.b1 < b.length ? sb[r.b1] : textB.length;
      if (r.a0 === r.a1 || r.b0 === r.b1) {
        changes.push(new Change(fromA, toA, fromB, toB));
        continue;
      }
      for (const c of presentableDiff(textA.slice(fromA, toA), textB.slice(fromB, toB), config))
        changes.push(new Change(c.fromA + fromA, c.toA + fromA, c.fromB + fromB, c.toB + fromB));
    }
    return changes;
  };
}
