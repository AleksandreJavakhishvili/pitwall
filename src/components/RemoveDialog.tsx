import { useState } from "react";
import type { AgentView } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { Modal } from "./Modal";

export function RemoveDialog({ agent: a, onClose }: { agent: AgentView | null; onClose(): void }) {
  const { run } = useActions();
  const [deleteWorktree, setDeleteWorktree] = useState(false);
  const [busy, setBusy] = useState(false);
  if (!a) return null;

  const remove = async () => {
    setBusy(true);
    const ok = await run(api.removeAgent(a.id, deleteWorktree).then(() => true), `remove ${a.name}`);
    setBusy(false);
    if (ok) onClose();
  };

  return (
    <Modal title={`Remove ${a.name}`} onClose={onClose} width={440}>
      <div className="modal-body">
        {a.caps.removeKeepsSession ? (
          <p>
            Pitwall stops tracking {a.name}. Its session on {a.machine?.label ?? "its machine"} is left as it is
            {a.running ? " and keeps running" : ""}; you can add it again from Settings → Scan again.
          </p>
        ) : (
          <p>
            {a.running ? "The agent process will be stopped and " : "The agent "}
            will be removed from Pitwall. Its terminal history goes with it.
          </p>
        )}
        {a.caps.removeWorktree ? (
          <label className="check">
            <input type="checkbox" checked={deleteWorktree} onChange={(e) => setDeleteWorktree(e.target.checked)} />
            <span>
              Also remove worktree
              <span className="hint block mono">{a.cwdDisplay}</span>
              <span className="hint block">
                Runs <span className="mono">git worktree remove</span>, which refuses if it has uncommitted changes.{" "}
                {a.branch ? <>Branch <span className="mono">{a.branch}</span> is kept.</> : "Its branch is kept."}
              </span>
            </span>
          </label>
        ) : (
          a.worktreePending && <p className="hint">Its worktree (if the agent made one) is left as it is.</p>
        )}
        {deleteWorktree && a.caps.removeWorktree && (
          <p className="form-error">The worktree folder will be deleted from disk.</p>
        )}
      </div>
      <footer className="modal-foot">
        <button className="ghost-btn" onClick={onClose}>
          Cancel
        </button>
        <button className="danger-btn" onClick={remove} disabled={busy} data-autofocus>
          {busy ? "Removing…" : deleteWorktree && a.caps.removeWorktree ? "Remove agent & worktree" : "Remove"}
        </button>
      </footer>
    </Modal>
  );
}
