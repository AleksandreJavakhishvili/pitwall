import { useState } from "react";
import type { AgentView, RunningElsewhere } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { bringInRequest } from "../lib/terminals";
import { newAgentSize } from "../terminal/registry";
import { Modal } from "./Modal";

/** "Bring into Pitwall": resume an outside agent's conversation here (it keeps running where it is). */
export function BringInDialog({ row, agents, onClose, onCreated }: { row: RunningElsewhere; agents: AgentView[]; onClose(): void; onCreated(a: AgentView): void }) {
  const { run } = useActions();
  const [busy, setBusy] = useState(false);
  const req = bringInRequest(row, agents);

  const go = async () => {
    if (!req) return;
    setBusy(true);
    const a = await run(api.continueConversation({ ...req, ...(await newAgentSize()) }), "bring the agent into Pitwall");
    setBusy(false);
    if (a) onCreated(a);
  };

  return (
    <Modal title="Bring into Pitwall" onClose={onClose} width={460}>
      <div className="modal-body">
        <p>
          Resumes this {row.kindName} conversation in a new Pitwall agent
          {row.cwdDisplay ? (
            <>
              {" "}in <span className="mono">{row.cwdDisplay}</span>
            </>
          ) : null}
          .
        </p>
        {row.title && <p className="hint">“{row.title}”</p>}
        <div className="confirm-box">
          <p>
            <strong>Close the original first</strong> (in the other terminal, pid <span className="mono">{row.pid}</span>), so two
            copies don't work on the same conversation. Pitwall never stops it for you.
          </p>
        </div>
      </div>
      <footer className="modal-foot">
        <button className="ghost-btn" onClick={onClose}>
          Cancel
        </button>
        <button className="primary-btn" onClick={go} disabled={busy || !req} data-autofocus>
          {busy ? "Starting…" : "I closed it, resume here"}
        </button>
      </footer>
    </Modal>
  );
}
