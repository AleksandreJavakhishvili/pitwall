import { useState } from "react";
import type { AgentView } from "../types";
import { useActions } from "../lib/actions";
import { Icon } from "./Icon";

export function StoppedOverlay({ agent: a }: { agent: AgentView }) {
  const { restart, openRemove } = useActions();
  const [busy, setBusy] = useState(false);
  const word = a.status === "stopped" ? "is stopped" : "has exited";
  // The backend resumes the session only if a prompt was sent before.
  const verb = a.lastSent ? "Resume" : "Restart";
  return (
    <div className="stopped-overlay">
      <div className="stopped-card">
        <div className="label-lg">
          {a.name} {word}
        </div>
        <p className="muted">
          {a.status === "stopped" ? "Not running since Pitwall restarted." : "The process ended."}
          {a.lastSent ? " Resume picks up the previous session." : ""}
        </p>
        <div className="row gap">
          <button
            className="primary-btn"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              await restart(a.id);
              setBusy(false);
            }}
          >
            <Icon name="restart" size={14} /> {busy ? `${verb === "Resume" ? "Resuming" : "Restarting"}…` : verb}
          </button>
          <button className="ghost-btn" onClick={() => openRemove(a.id)}>
            <Icon name="trash" size={14} /> Remove
          </button>
        </div>
      </div>
    </div>
  );
}
