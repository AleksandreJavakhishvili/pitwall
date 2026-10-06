export type DiffLine =
  | { kind: "hunk"; text: string }
  | { kind: "add" | "del" | "ctx"; text: string; old: number | null; new: number | null }
  | { kind: "meta"; text: string };

/** Parse a unified diff into renderable lines with old/new line numbers. */
export function parseUnifiedDiff(diff: string): DiffLine[] {
  const out: DiffLine[] = [];
  let o = 0;
  let n = 0;
  let inHunk = false;
  const lines = diff.replace(/\n$/, "").split("\n");
  for (const raw of lines) {
    const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(raw);
    if (m) {
      o = Number(m[1]);
      n = Number(m[2]);
      inHunk = true;
      out.push({ kind: "hunk", text: raw });
      continue;
    }
    if (!inHunk) {
      // Header lines (diff --git, index, ---, +++) are noise here, but keep notes like "Binary files differ".
      if (/^(diff --git|index |--- |\+\+\+ )/.test(raw)) continue;
      if (raw.trim()) out.push({ kind: "meta", text: raw });
      continue;
    }
    if (raw.startsWith("+")) out.push({ kind: "add", text: raw.slice(1), old: null, new: n++ });
    else if (raw.startsWith("-")) out.push({ kind: "del", text: raw.slice(1), old: o++, new: null });
    else if (raw.startsWith("\\")) out.push({ kind: "meta", text: raw });
    else if (raw.startsWith("diff --git")) {
      inHunk = false;
    } else out.push({ kind: "ctx", text: raw.slice(1), old: o++, new: n++ });
  }
  return out;
}
