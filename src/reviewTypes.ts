// Review screen shapes (docs/spec/review.md). Keep in sync with pitwall-core's review.rs + tasks.rs.
import type { FileChange } from "./types";

/** One prompt delivered to an agent, until it went done/idle. */
export interface Task {
  id: string;
  /** Verbatim prompt; empty when typed in the terminal and no hook reported it. */
  prompt: string;
  startedAt: number;
  endedAt: number | null;
  /** null until the start snapshot has been taken. */
  startTree: string | null;
  endTree: string | null;
}

export interface FileVersions {
  original: string | null;
  modified: string | null;
  /** Binary or too large: both sides null. */
  binary: boolean;
}

export interface MergeResult {
  merged: boolean;
  conflict: boolean;
  message: string;
  /** Branch of the main checkout (merge target). */
  branch: string | null;
}

export interface MergeStatus {
  worktree: boolean;
  /** Agent's branch (worktree agents: the worktree's current branch; null = detached). */
  branch: string | null;
  /** Main checkout branch (worktree agents only). */
  target: string | null;
  targetDirty: boolean;
  /** Uncommitted entries in the agent's checkout. */
  uncommitted: number;
  /** Commits on the agent's branch not yet in target. */
  ahead: number;
}

export interface ReviewApi {
  listTasks(agentId: string): Promise<Task[]>;
  /** taskId null = all changes since the agent started. Paths are repo-root relative. */
  getTaskChanges(agentId: string, taskId: string | null): Promise<FileChange[]>;
  getFileVersions(agentId: string, path: string, taskId: string | null): Promise<FileVersions>;
  discardFile(agentId: string, path: string): Promise<void>;
  /** Returns the short id of the new commit. */
  commitAgent(agentId: string, message: string): Promise<string>;
  mergeAgent(agentId: string): Promise<MergeResult>;
  getMergeStatus(agentId: string): Promise<MergeStatus>;
}
