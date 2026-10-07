import { useEffect, useState } from "react";
import { api, getApi } from "../api";
import type { AgentView } from "../types";

/**
 * `next`, reusing `prev`'s objects for agents that did not change (and `prev`
 * itself when nothing did), so memoised rows/panes/tiles skip re-rendering
 * and an `agents-changed` without visible changes renders nothing at all.
 */
export function shareAgents(prev: AgentView[], next: AgentView[]): AgentView[] {
  const old = new Map(prev.map((a) => [a.id, a]));
  let same = prev.length === next.length;
  const out = next.map((a, i) => {
    const o = old.get(a.id);
    const keep = o && (o === a || JSON.stringify(o) === JSON.stringify(a)) ? o : a;
    if (keep !== prev[i]) same = false;
    return keep;
  });
  return same ? prev : out;
}

/** Live agent list: initial list_agents, then every agents-changed event. */
export function useAgents() {
  const [agents, setAgents] = useState<AgentView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [isMock, setIsMock] = useState(false);
  const [label, setLabel] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | null = null;
    getApi().then(async (a) => {
      if (!alive) return;
      setIsMock(a.isMock);
      setLabel(a.windowLabel());
      unlisten = await a.onAgentsChanged((list) => alive && setAgents((prev) => shareAgents(prev, list)));
      if (!alive) return unlisten();
      const list = await api.listAgents().catch(() => [] as AgentView[]);
      if (alive) {
        setAgents((prev) => shareAgents(prev, list));
        setLoaded(true);
      }
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  /** Apply an AgentView returned by a command right away (events may lag). */
  const patch = (a: AgentView) =>
    setAgents((list) =>
      shareAgents(list, list.some((x) => x.id === a.id) ? list.map((x) => (x.id === a.id ? a : x)) : [...list, a]),
    );

  return { agents, loaded, isMock, label, patch };
}
