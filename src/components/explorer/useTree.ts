import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";
import type { AgentView } from "../../types";
import { api } from "../../api";
import { TreeModel, type TreeState } from "../../lib/explorer";
import { useRefreshRequest } from "../../lib/freshness";
import { usePersistentFlag } from "../../lib/usePersistentFlag";

/** Open folders are read again this often while the tree is shown and the window visible. */
export const TREE_POLL_MS = 10_000;

// One tree per agent for the window's lifetime: the Files tab and the viewer
// show the same open folders.
const models = new Map<string, { model: TreeModel; ignored: boolean }>();

function entry(agentId: string, ignored: boolean) {
  let m = models.get(agentId);
  if (!m) {
    m = { model: new TreeModel((dir) => api.listFiles(agentId, dir, ignored)), ignored };
    models.set(agentId, m);
  }
  return m;
}

export function treeModel(agentId: string, ignored: boolean): TreeModel {
  return entry(agentId, ignored).model;
}

/** "Show ignored files" in the tree (off by default, like VS Code's git-ignored files hidden). */
export function useShowIgnored() {
  return usePersistentFlag("pitwall.explorer.showIgnored", false);
}

/**
 * The agent's file tree while shown: read again on open, on ↻ / ⌘⇧R, when the
 * agent's change totals move, and every 10 s while the window is visible.
 */
export function useTree(agent: AgentView, ignored: boolean): { state: TreeState; model: TreeModel } {
  const model = treeModel(agent.id, ignored);
  const subscribe = useCallback((fn: () => void) => model.subscribe(fn), [model]);
  const state = useSyncExternalStore(subscribe, () => model.state);
  // "Show ignored files" toggled: list everything again that way.
  useEffect(() => {
    const m = entry(agent.id, ignored);
    if (m.ignored === ignored) return;
    m.ignored = ignored;
    void m.model.setLoader((dir) => api.listFiles(agent.id, dir, ignored));
  }, [agent.id, ignored]);
  useEffect(() => {
    void model.refresh();
    const t = setInterval(() => document.visibilityState !== "hidden" && void model.refresh(true), TREE_POLL_MS);
    const onVisible = () => document.visibilityState !== "hidden" && void model.refresh(true);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      clearInterval(t);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [model]);
  const sig = `${agent.added}:${agent.removed}:${agent.filesChanged}`;
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    void model.refresh(true);
  }, [sig]); // eslint-disable-line react-hooks/exhaustive-deps
  useRefreshRequest(() => void model.refresh());
  return { state, model };
}
