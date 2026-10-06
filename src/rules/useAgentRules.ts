// One shared poller for per-agent rules state (stale flags, errors), so every
// pane header can read it without each one calling the backend.
import { useEffect, useState } from "react";
import { rulesApi, type AgentRules } from "./api";

type Map = Record<string, AgentRules>;
let current: Map = {};
const subs = new Set<(m: Map) => void>();
let timer: ReturnType<typeof setInterval> | null = null;
let inflight = false;

export async function refreshAgentRules(): Promise<void> {
  if (inflight) return;
  inflight = true;
  try {
    const list = await rulesApi.agentRules();
    current = Object.fromEntries(list.map((r) => [r.agentId, r]));
    subs.forEach((f) => f(current));
  } catch {
    // Older backend or transient error: keep the last state.
  } finally {
    inflight = false;
  }
}

const onFocus = () => void refreshAgentRules();

export function useAgentRules(agentId: string): AgentRules | undefined {
  const [map, setMap] = useState<Map>(current);
  useEffect(() => {
    subs.add(setMap);
    if (subs.size === 1) {
      void refreshAgentRules();
      // Library files are edited outside Pitwall; re-check now and then.
      timer = setInterval(refreshAgentRules, 20_000);
      window.addEventListener("focus", onFocus);
    }
    return () => {
      subs.delete(setMap);
      if (subs.size === 0) {
        if (timer) clearInterval(timer);
        timer = null;
        window.removeEventListener("focus", onFocus);
      }
    };
  }, []);
  return map[agentId];
}
