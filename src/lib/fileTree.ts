import type { FileChange, FileStatus } from "../types";

/**
 * VS Code SCM letter. Uses the backend's `status` when present; otherwise
 * degrades to U (untracked) / M (everything else).
 */
export function fileStatus(f: FileChange): FileStatus {
  return f.status ?? (f.untracked ? "U" : "M");
}

export const STATUS_TITLE: Record<FileStatus, string> = {
  M: "Modified",
  A: "Added",
  D: "Deleted",
  R: "Renamed",
  U: "Untracked",
};

export function splitPath(p: string) {
  const i = p.lastIndexOf("/");
  return i < 0 ? { dir: "", base: p } : { dir: p.slice(0, i + 1), base: p.slice(i + 1) };
}

export type TreeNode =
  | { kind: "dir"; /** full path, the collapse key */ path: string; /** shown label, may be compact "a/b/c" */ name: string; children: TreeNode[] }
  | { kind: "file"; path: string; name: string; file: FileChange };

interface RawDir {
  dirs: Map<string, RawDir>;
  files: FileChange[];
}

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });

/**
 * Folder tree like VS Code's: folders first, then files, by name; a folder whose
 * only child is another folder is shown on one row (`src/components/review`).
 */
export function buildTree(files: FileChange[]): TreeNode[] {
  const root: RawDir = { dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split("/");
    let d = root;
    for (const seg of parts.slice(0, -1)) {
      let next = d.dirs.get(seg);
      if (!next) d.dirs.set(seg, (next = { dirs: new Map(), files: [] }));
      d = next;
    }
    d.files.push(f);
  }
  const walk = (d: RawDir, prefix: string): TreeNode[] => {
    const out: TreeNode[] = [];
    for (const seg of [...d.dirs.keys()].sort(byName)) {
      let sub = d.dirs.get(seg)!;
      let name = seg;
      while (sub.files.length === 0 && sub.dirs.size === 1) {
        const [k, v] = [...sub.dirs][0];
        name += "/" + k;
        sub = v;
      }
      const path = prefix + name;
      out.push({ kind: "dir", path, name, children: walk(sub, path + "/") });
    }
    for (const f of [...d.files].sort((a, b) => byName(splitPath(a.path).base, splitPath(b.path).base))) {
      out.push({ kind: "file", path: f.path, name: splitPath(f.path).base, file: f });
    }
    return out;
  };
  return walk(root, "");
}

/** Files in the order the tree shows them (all folders expanded). */
export function treeOrder(files: FileChange[]): FileChange[] {
  const out: FileChange[] = [];
  const visit = (ns: TreeNode[]) => ns.forEach((n) => (n.kind === "file" ? out.push(n.file) : visit(n.children)));
  visit(buildTree(files));
  return out;
}
