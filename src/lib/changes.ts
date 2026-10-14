// The Changes list's sections (VS Code's Source Control): a repository's
// staged and not-staged changes apart, and each repository inside the
// agent's folder under its own header.
import type { FileChange, FilePart } from "../types";

export type Stage = "staged" | "unstaged";

/** Which part of a file a row (and its diff) is: staged, not, or both (null). */
export function stageOf(f: FileChange): Stage | null {
  const p = f.parts;
  if (!p) return null;
  if (p.staged && !p.unstaged) return "staged";
  if (!p.staged && p.unstaged) return "unstaged";
  return null;
}

/**
 * A repository's files as "Staged Changes" then "Changes" when anything is
 * staged (a file both staged and changed again is in both, each row its
 * part); else one list without headers (`stage: null`).
 */
export function stageGroups(files: FileChange[]): { stage: Stage | null; files: FileChange[] }[] {
  if (!files.some((f) => f.parts?.staged)) return [{ stage: null, files: files.map(({ parts: _p, ...f }) => f) }];
  const part = (f: FileChange, stage: Stage): FileChange | null => {
    // (No parts read: all of it counts as not staged.)
    const p: FilePart | null =
      stage === "staged" ? (f.parts?.staged ?? null) : f.parts ? f.parts.unstaged : { added: f.added, removed: f.removed, status: f.status };
    if (!p) return null;
    return {
      ...f,
      added: p.added,
      removed: p.removed,
      status: p.status ?? f.status,
      untracked: f.untracked && stage === "unstaged",
      parts: stage === "staged" ? { staged: p, unstaged: null } : { staged: null, unstaged: p },
    };
  };
  return (["staged", "unstaged"] as Stage[])
    .map((stage) => ({ stage, files: files.map((f) => part(f, stage)).filter((f): f is FileChange => !!f) }))
    .filter((g) => g.files.length > 0);
}

/** The changed files by repository: the agent's own first (`repo: null`), then those inside its folder by name. */
export function changeGroups(files: FileChange[]): { repo: string | null; files: FileChange[] }[] {
  const out: { repo: string | null; files: FileChange[] }[] = [];
  for (const f of files) {
    const repo = f.repo ?? null;
    const g = out.find((x) => x.repo === repo);
    if (g) g.files.push(f);
    else out.push({ repo, files: [f] });
  }
  return out.sort((a, b) => (a.repo === null ? -1 : b.repo === null ? 1 : a.repo.localeCompare(b.repo)));
}

/** A nested repository's file as its row shows it: relative to that repository. */
export const shownPath = (f: FileChange) => (f.repo && f.path.startsWith(`${f.repo}/`) ? f.path.slice(f.repo.length + 1) : f.path);
