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

function refresh() {
  if (inflight) {
    again = true;
    return;
  }
  inflight = true;
  api
    .listWorktrees()
    .then((list) => {
      if (JSON.stringify(list) !== JSON.stringify(current)) {
        current = list;
        listeners.forEach((l) => l());
      }
    })
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
 * when its HEAD moves or `nonce` changes, and every `everyMs` after that.
 */
export function useWorktreeFiles(projectId: string, path: string, head: string | null, enabled: boolean, nonce = 0, everyMs = 15_000) {
  const [files, setFiles] = useState<FileChange[] | null>(null);
  const [error, setError] = useState<string | null>(null);
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
  }, [projectId, path, head, enabled, nonce, everyMs]);
  return { files, error };
}
