// The read-only code explorer's UI logic (docs/spec/explorer.md), kept free of
// React so it can be tested: quick-open matching, the lazy tree's state and
// search result grouping.
import type { DirListing, FileEntry, SearchMatch } from "../explorerApi";

// ── quick open (⌘P) ──────────────────────────────────────────────────────

export interface QuickHit {
  path: string;
  score: number;
  /** Indexes into `path` of the matched characters (for highlighting). */
  positions: number[];
}

const SEP = /[/\\_\-.\s]/;

/**
 * VS Code-style fuzzy match of `query` against `path`: every query character
 * in order, case-insensitive; spaces separate parts that may match anywhere.
 * Matches in the file name, at word starts and in a row score higher.
 * `null`: no match.
 */
export function fuzzyMatch(path: string, query: string): QuickHit | null {
  const parts = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!parts.length) return { path, score: 0, positions: [] };
  const lower = path.toLowerCase();
  const baseStart = path.lastIndexOf("/") + 1;
  let score = 0;
  const positions: number[] = [];
  for (const part of parts) {
    const hit = matchPart(path, lower, part, baseStart);
    if (!hit) return null;
    score += hit.score;
    positions.push(...hit.positions);
  }
  // Shorter paths first among equals (less to wade through).
  score -= path.length * 0.01;
  return { path, score, positions: [...new Set(positions)].sort((a, b) => a - b) };
}

function matchPart(path: string, lower: string, q: string, baseStart: number): { score: number; positions: number[] } | null {
  // A plain substring of the file name beats any scattered match.
  const inBase = lower.indexOf(q, baseStart);
  if (inBase >= 0) {
    const positions = Array.from({ length: q.length }, (_, i) => inBase + i);
    const exact = inBase === baseStart && lower.length - baseStart === q.length;
    const stem = inBase === baseStart && (lower.length === baseStart + q.length || lower[baseStart + q.length] === ".");
    return { score: 100 + q.length * 10 + (inBase === baseStart ? 40 : 0) + (stem ? 30 : 0) + (exact ? 30 : 0), positions };
  }
  // Prefer matching from the file name backwards: find the last place the
  // query can start so its tail lands as far right as possible.
  const positions: number[] = [];
  let qi = q.length - 1;
  for (let i = lower.length - 1; i >= 0 && qi >= 0; i--) {
    if (lower[i] === q[qi]) {
      positions.push(i);
      qi--;
    }
  }
  if (qi >= 0) return null;
  positions.reverse();
  let score = 0;
  for (let k = 0; k < positions.length; k++) {
    const p = positions[k];
    const prev = positions[k - 1];
    if (k > 0 && p === prev + 1) score += 8;
    if (p === 0 || SEP.test(path[p - 1]) || (path[p] !== lower[p] && path[p - 1] === lower[p - 1])) score += 6;
    if (p >= baseStart) score += 4;
    score += 1;
  }
  return { score, positions };
}

/** The best `limit` matches, best first; an empty query lists `recent` first, then the rest in order. */
export function quickOpen(files: readonly string[], query: string, opts: { limit?: number; recent?: readonly string[] } = {}): QuickHit[] {
  const limit = opts.limit ?? 60;
  if (!query.trim()) {
    const recent = (opts.recent ?? []).filter((p) => files.includes(p));
    const seen = new Set(recent);
    return [...recent, ...files.filter((f) => !seen.has(f))].slice(0, limit).map((path) => ({ path, score: 0, positions: [] }));
  }
  const hits: QuickHit[] = [];
  for (const f of files) {
    const h = fuzzyMatch(f, query);
    if (h) hits.push(h);
  }
  hits.sort((a, b) => b.score - a.score || a.path.localeCompare(b.path));
  return hits.slice(0, limit);
}

/** `text` split into plain and highlighted runs at `positions` (sorted indexes). */
export function highlightRuns(text: string, positions: readonly number[], offset = 0): { text: string; hit: boolean }[] {
  const set = new Set(positions.map((p) => p - offset));
  const out: { text: string; hit: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const hit = set.has(i);
    const last = out[out.length - 1];
    if (last && last.hit === hit) last.text += text[i];
    else out.push({ text: text[i], hit });
  }
  return out;
}

// ── paths ────────────────────────────────────────────────────────────────

/** "src/a/b.ts" → ["src", "src/a"]: the folders to expand to show it. */
export function parentDirs(path: string): string[] {
  const parts = path.split("/").slice(0, -1);
  return parts.map((_, i) => parts.slice(0, i + 1).join("/"));
}

/** The agent's folder joined with a relative path, with its own separator (what "Copy path" copies). */
export function absolutePath(root: string, rel: string): string {
  if (!rel) return root;
  const win = /^[A-Za-z]:\\/.test(root) || (root.includes("\\") && !root.includes("/"));
  const sep = win ? "\\" : "/";
  return root.replace(/[/\\]+$/, "") + sep + (win ? rel.replace(/\//g, "\\") : rel);
}

/** "12 KB", "3.4 MB" (binary units, as VS Code shows them). */
export function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
  const mb = kb / 1024;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

/** Include / exclude fields: comma-separated globs ("*.ts, src/**"). */
export function globList(text: string): string[] {
  return text
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

// ── the lazy tree ────────────────────────────────────────────────────────

export interface TreeState {
  /** Listed folders by path ("" = the agent's folder). */
  listings: Record<string, DirListing>;
  /** Folders that couldn't be listed, with why. */
  errors: Record<string, string>;
  /** Expanded folders (whether listed yet or not). */
  expanded: readonly string[];
  /** Folders being listed for the first time. */
  loading: readonly string[];
  /** A refresh of everything shown is running (↻, ⌘⇧R, opening). */
  refreshing: boolean;
  updatedAt: number | null;
}

export const EMPTY_TREE: TreeState = { listings: {}, errors: {}, expanded: [], loading: [], refreshing: false, updatedAt: null };

export type TreeRow =
  | { kind: "entry"; entry: FileEntry; depth: number; open: boolean }
  | { kind: "loading"; dir: string; depth: number }
  | { kind: "error"; dir: string; depth: number; error: string }
  | { kind: "truncated"; dir: string; depth: number };

/** What the tree shows, top to bottom: entries of expanded folders nested under them. */
export function treeRows(s: TreeState): TreeRow[] {
  const open = new Set(s.expanded);
  const out: TreeRow[] = [];
  const walk = (dir: string, depth: number) => {
    const l = s.listings[dir];
    if (!l) {
      if (s.errors[dir]) out.push({ kind: "error", dir, depth, error: s.errors[dir] });
      else out.push({ kind: "loading", dir, depth });
      return;
    }
    for (const entry of l.entries) {
      const isOpen = entry.kind === "dir" && open.has(entry.path);
      out.push({ kind: "entry", entry, depth, open: isOpen });
      if (isOpen) walk(entry.path, depth + 1);
    }
    if (l.truncated) out.push({ kind: "truncated", dir, depth });
    if (s.errors[dir]) out.push({ kind: "error", dir, depth, error: s.errors[dir] });
  };
  walk("", 0);
  return out;
}

/** The entry for `path` among the listed folders, if listed. */
export function findEntry(s: TreeState, path: string): FileEntry | null {
  const dir = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  return s.listings[dir]?.entries.find((e) => e.path === path) ?? null;
}

type Listener = (s: TreeState) => void;

/**
 * One agent's tree: which folders are open and what they hold. Folders load
 * when first expanded; `refresh` lists the agent's folder and every open
 * folder again (coalesced while one runs). Shared by the Files tab and the
 * viewer, so both show the same folders open.
 */
export class TreeModel {
  private s: TreeState = EMPTY_TREE;
  private listeners = new Set<Listener>();
  private running: Promise<void> | null = null;
  private gen = 0;

  constructor(
    private load: (dir: string) => Promise<DirListing>,
    private now: () => number = Date.now,
  ) {}

  get state(): TreeState {
    return this.s;
  }

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => {
      this.listeners.delete(fn);
    };
  }

  private set(p: Partial<TreeState>) {
    this.s = { ...this.s, ...p };
    this.listeners.forEach((l) => l(this.s));
  }

  /** A new way of listing (e.g. "Show ignored files" toggled): forget what was read, read again. */
  setLoader(load: (dir: string) => Promise<DirListing>): Promise<void> {
    this.load = load;
    this.gen++;
    this.running = null;
    this.set({ listings: {}, errors: {}, loading: [] });
    return this.refresh();
  }

  isOpen(dir: string): boolean {
    return this.s.expanded.includes(dir);
  }

  toggle(dir: string): Promise<void> {
    return this.isOpen(dir) ? (this.collapse(dir), Promise.resolve()) : this.expand(dir);
  }

  collapse(dir: string): void {
    if (this.isOpen(dir)) this.set({ expanded: this.s.expanded.filter((d) => d !== dir) });
  }

  /** Open a folder; listed now unless it already is. */
  expand(dir: string): Promise<void> {
    if (!this.isOpen(dir)) this.set({ expanded: [...this.s.expanded, dir] });
    if (this.s.listings[dir] || this.s.loading.includes(dir)) return Promise.resolve();
    return this.listOne(dir, true);
  }

  /** Open every folder above `path` (to show it). */
  reveal(path: string): Promise<void> {
    return Promise.all(parentDirs(path).map((d) => this.expand(d))).then(() => {});
  }

  private listOne(dir: string, first: boolean): Promise<void> {
    const gen = this.gen;
    if (first) this.set({ loading: [...this.s.loading, dir] });
    return this.load(dir).then(
      (l) => {
        if (gen !== this.gen) return;
        const errors = { ...this.s.errors };
        delete errors[dir];
        const same = JSON.stringify(this.s.listings[dir]) === JSON.stringify(l);
        this.set({
          listings: same ? this.s.listings : { ...this.s.listings, [dir]: l },
          errors,
          loading: this.s.loading.filter((d) => d !== dir),
        });
      },
      (e) => {
        if (gen !== this.gen) return;
        this.set({ errors: { ...this.s.errors, [dir]: String(e instanceof Error ? e.message : e) }, loading: this.s.loading.filter((d) => d !== dir) });
      },
    );
  }

  /** The agent's folder and every open folder, read again (`quiet`: no "refreshing…"). */
  refresh(quiet = false): Promise<void> {
    if (this.running) return this.running;
    const gen = this.gen;
    if (!quiet) this.set({ refreshing: true });
    const dirs = ["", ...this.s.expanded.filter((d) => d !== "")];
    const first = !this.s.listings[""];
    const p = Promise.all(dirs.map((d) => this.listOne(d, first && d === ""))).then(() => {
      if (gen !== this.gen) return;
      // Open folders that no longer exist close.
      const gone = this.s.expanded.filter((d) => this.s.errors[d] && /not found/i.test(this.s.errors[d]));
      const errors = { ...this.s.errors };
      gone.forEach((d) => delete errors[d]);
      this.set({ expanded: this.s.expanded.filter((d) => !gone.includes(d)), errors, updatedAt: this.now() });
    });
    const done = p.finally(() => {
      if (this.running === done) this.running = null;
      if (gen === this.gen && !quiet) this.set({ refreshing: false });
    });
    this.running = done;
    return done;
  }
}

// ── search results ───────────────────────────────────────────────────────

export interface MatchGroup {
  path: string;
  matches: SearchMatch[];
}

/** Matches grouped by file, in the order the files first appear. */
export function groupMatches(matches: readonly SearchMatch[]): MatchGroup[] {
  const by = new Map<string, SearchMatch[]>();
  for (const m of matches) {
    let list = by.get(m.path);
    if (!list) by.set(m.path, (list = []));
    list.push(m);
  }
  return [...by].map(([path, matches]) => ({ path, matches }));
}

/** A match's line as runs of plain and matched text. */
export function matchRuns(m: SearchMatch): { text: string; hit: boolean }[] {
  const out: { text: string; hit: boolean }[] = [];
  let at = 0;
  const ranges = [...m.ranges].sort((a, b) => a.start - b.start);
  for (const r of ranges) {
    if (r.start < at) continue;
    if (r.start > at) out.push({ text: m.text.slice(at, r.start), hit: false });
    out.push({ text: m.text.slice(r.start, r.end), hit: true });
    at = r.end;
  }
  if (at < m.text.length) out.push({ text: m.text.slice(at), hit: false });
  return out;
}
