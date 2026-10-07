import { createContext, useContext, useEffect, useState, useSyncExternalStore } from "react";
import { api, errorText } from "../api";
import type { AgentView, FileChange } from "../types";
import type { ProjectWorktrees } from "../worktreesApi";
import { agentsSignature } from "./worktrees";

/** While visible, the list is asked for this often (the backend lists a project at most this often too). */
export const LIST_EVERY_MS = 30_000;
/** After agents' numbers move, ask once things settle. */
const SETTLE_MS = 1_000;

// One list per window, shared by the sidebar, the Changes panel and Review.
let current: ProjectWorktrees[] = [];
let inflight = false;
let again = false;
let lastSig = "";
let users = 0;
let timer: ReturnType<typeof setInterval> | null = null;
const listeners = new Set<() => void>();

/** How fresh the list is: a forced refresh running, when it was last read, the last forced refresh's error. */
export interface WorktreeStatus {
  refreshing: boolean;
  updatedAt: number | null;
  error: string | null;
}
let status: WorktreeStatus = { refreshing: false, updatedAt: null, error: null };
const statusListeners = new Set<() => void>();
function setStatus(p: Partial<WorktreeStatus>) {
  status = { ...status, ...p };
  statusListeners.forEach((l) => l());
}

function take(list: ProjectWorktrees[]) {
  setStatus({ updatedAt: Date.now() });
  if (JSON.stringify(list) !== JSON.stringify(current)) {
    current = list;
    listeners.forEach((l) => l());
  }
}

function refresh() {
  if (inflight) {
    again = true;
    return;
  }
  inflight = true;
  api
    .listWorktrees()
    .then(take)
    .catch(() => {})
    .finally(() => {
      inflight = false;
      if (again) {
        again = false;
        refresh();
      }
    });
}

/** Ask again now (after commit, merge or remove). */
export const refreshWorktrees = refresh;

const forcing = new Map<string, Promise<ProjectWorktrees[]>>();

/**
 * List `projectId` (every project, when omitted) again now, bypassing the
 * backend's pace: a worktree list was expanded, or the user pressed ↻ / ⌘⇧R.
 * Asked again while one runs, the same one is awaited. Rejects with the error
 * (also kept in `useWorktreeStatus().error`).
 */
export function forceRefreshWorktrees(projectId?: string): Promise<ProjectWorktrees[]> {
  const key = projectId ?? "*";
  const running = forcing.get(key);
  if (running) return running;
  setStatus({ refreshing: true });
  const p = api
    .refreshWorktrees(projectId)
    .then((list) => {
      take(list);
      setStatus({ error: null });
      return list;
    })
    .catch((e) => {
      setStatus({ error: errorText(e) });
      throw e;
    })
    .finally(() => {
      forcing.delete(key);
      setStatus({ refreshing: forcing.size > 0 });
    });
  forcing.set(key, p);
  return p;
}

const subscribeStatus = (l: () => void) => {
  statusListeners.add(l);
  return () => {
    statusListeners.delete(l);
  };
};
const statusSnapshot = () => status;

/** The worktree list's freshness (for "refreshing…" / "updated 12 s ago" and errors). */
export function useWorktreeStatus(): WorktreeStatus {
  return useSyncExternalStore(subscribeStatus, statusSnapshot, statusSnapshot);
}

const onVisible = () => {
  if (!document.hidden) refresh();
};

function start() {
  if (users++ > 0) return;
  timer = setInterval(() => {
    if (!document.hidden) refresh();
  }, LIST_EVERY_MS);
  document.addEventListener("visibilitychange", onVisible);
}

function stop() {
  if (--users > 0) return;
  if (timer) clearInterval(timer);
  timer = null;
  document.removeEventListener("visibilitychange", onVisible);
}

const EMPTY: ProjectWorktrees[] = [];

/** The window's list (App calls `useWorktrees` once and provides it). */
export const WorktreesContext = createContext<ProjectWorktrees[]>(EMPTY);
export const useWorktreeList = () => useContext(WorktreesContext);
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
const snapshot = () => current;

/**
 * Every project's worktrees while any agent's can be listed (`caps.worktrees`):
 * asked on first use, every 30 s while the window is visible, and shortly
 * after agents' numbers move. Never per worktree: changes come from `useWorktreeFiles`.
 */
export function useWorktrees(agents: AgentView[]): ProjectWorktrees[] {
  const enabled = agents.some((a) => a.caps.worktrees);
  const list = useSyncExternalStore(subscribe, snapshot);
  const sig = agentsSignature(agents);

  useEffect(() => {
    if (!enabled) return;
    start();
    return stop;
  }, [enabled]);

  useEffect(() => {
    if (!enabled || sig === lastSig) return;
    const first = lastSig === "";
    const t = setTimeout(
      () => {
        lastSig = sig;
        refresh();
      },
      first ? 0 : SETTLE_MS,
    );
    return () => clearTimeout(t);
  }, [enabled, sig]);

  return enabled ? list : EMPTY;
}

/**
 * One worktree's changes, read only while it is shown (`enabled`): on show,
 * when its HEAD moves or `nonce` changes, and every `everyMs` after that while
 * the window is visible. `refresh` reads again now (Retry).
 */
export function useWorktreeFiles(projectId: string, path: string, head: string | null, enabled: boolean, nonce = 0, everyMs = 15_000) {
  const [files, setFiles] = useState<FileChange[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [again, setAgain] = useState(0);
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const load = () =>
      api
        .getWorktreeChanges(projectId, path)
        .then((f) => {
          if (!alive) return;
          setFiles((old) => (JSON.stringify(old) === JSON.stringify(f) ? old : f));
          setError(null);
        })
        .catch((e) => alive && setError(errorText(e)));
    load();
    const t = setInterval(() => !document.hidden && load(), everyMs);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [projectId, path, head, enabled, nonce, everyMs, again]);
  return { files, error, refresh: () => setAgain((n) => n + 1) };
}
