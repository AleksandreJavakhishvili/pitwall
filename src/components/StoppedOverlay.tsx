import { useState } from "react";
import type { AgentView } from "../types";
import { useActions } from "../lib/actions";
import { restartAction } from "../lib/terminals";
import { Icon } from "./Icon";

export function StoppedOverlay({ agent: a }: { agent: AgentView }) {
  const { restart, openRemove } = useActions();
  const [busy, setBusy] = useState(false);
  const word = a.status === "stopped" ? "is stopped" : "has exited";
  // A restart continues the conversation when the agent can (caps.resume); a
  // terminal restarts as the agent last started in it (restartAs).
  const action = restartAction(a);
  return (
    <div className="stopped-overlay">
      <div className="stopped-card">
        <div className="label-lg">
          {a.name} {word}
        </div>
        <p className="muted">
          {a.status === "stopped"
            ? a.caps.removeKeepsSession
              ? `Its session on ${a.machine?.label ?? "its machine"} isn't running.`
              : "Not running since Pitwall restarted."
            : "The process ended."}
          {action.note}
        </p>
        <div className="row gap">
          {a.caps.restart && (
            <button
              className="primary-btn"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                await restart(a.id);
                setBusy(false);
              }}
            >
              <Icon name="restart" size={14} /> {busy ? action.busy : action.label}
            </button>
          )}
          <button className="ghost-btn" onClick={() => openRemove(a.id)}>
            <Icon name="trash" size={14} /> Remove
          </button>
        </div>
      </div>
    </div>
  );
}
