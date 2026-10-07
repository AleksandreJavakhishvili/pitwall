// Worktrees in source control (docs/spec/worktrees-view.md). Shapes from
// pitwall-proto (src/gen); per-worktree calls take the project's `id` and the
// worktree's path from `listWorktrees`.
import type { FileChange } from "./types";
import type { ProjectWorktrees } from "./gen/ProjectWorktrees";
import type { FileVersions, MergeResult, MergeStatus } from "./reviewTypes";

export type { ProjectWorktrees } from "./gen/ProjectWorktrees";
export type { WorktreeView } from "./gen/WorktreeView";
export type { WorktreeCaps } from "./gen/WorktreeCaps";
export type { WorktreeVia } from "./gen/WorktreeVia";

export interface WorktreesApi {
  /** Every project's worktrees (projects agents with `caps.worktrees` work in). The backend
   * lists a project again only when something there changed or its list is ~30 s old. */
  listWorktrees(): Promise<ProjectWorktrees[]>;
  /** Every project's worktrees, with `projectId` (all, when omitted) listed again now whatever
   * its age or its machine's pace; one already running is awaited. */
  refreshWorktrees(projectId?: string): Promise<ProjectWorktrees[]>;
  /** Changes against the merge-base with the project's current branch (committed, uncommitted, untracked). */
  getWorktreeChanges(projectId: string, path: string): Promise<FileChange[]>;
  getWorktreeFileVersions(projectId: string, path: string, file: string): Promise<FileVersions>;
  getWorktreeMergeStatus(projectId: string, path: string): Promise<MergeStatus>;
  /** `git add -A && git commit -m` in the worktree; the short commit id. */
  commitWorktree(projectId: string, path: string, message: string): Promise<string>;
  /** Into the project's current branch, with the Review merge rules. */
  mergeWorktree(projectId: string, path: string): Promise<MergeResult>;
  /** `git worktree remove` (never --force; locked worktrees are refused). The branch is kept. */
  removeWorktree(projectId: string, path: string): Promise<void>;
}

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

export function tauriWorktreesApi(invoke: Invoke): WorktreesApi {
  return {
    listWorktrees: () => invoke("list_worktrees"),
    refreshWorktrees: (projectId) => invoke("refresh_worktrees", { projectId: projectId ?? null }),
    getWorktreeChanges: (projectId, path) => invoke("get_worktree_changes", { projectId, path }),
    getWorktreeFileVersions: (projectId, path, file) => invoke("get_worktree_file_versions", { projectId, path, file }),
    getWorktreeMergeStatus: (projectId, path) => invoke("get_worktree_merge_status", { projectId, path }),
    commitWorktree: (projectId, path, message) => invoke("commit_worktree", { projectId, path, message }),
    mergeWorktree: (projectId, path) => invoke("merge_worktree", { projectId, path }),
    removeWorktree: (projectId, path) => invoke("remove_worktree", { projectId, path }),
  };
}
