import { useState } from "react";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { refreshWorktrees } from "../lib/useWorktrees";
import type { WorktreeRef } from "../lib/worktrees";
import { Modal } from "./Modal";

/** Confirm `git worktree remove` (never --force; locked worktrees can't be removed from Pitwall). */
export function RemoveWorktreeDialog({ target, onClose, onDone }: { target: WorktreeRef | null; onClose(): void; onDone?(msg: string): void }) {
  const { run } = useActions();
  const [busy, setBusy] = useState(false);
  if (!target) return null;
  const { wt } = target;

  const remove = async () => {
    setBusy(true);
    const ok = await run(api.removeWorktree(target.projectId, wt.path).then(() => true), `remove worktree ${wt.name}`);
    setBusy(false);
    refreshWorktrees();
    if (ok) {
      onDone?.(`Removed worktree ${wt.name}`);
      onClose();
    }
  };

  return (
    <Modal title={`Remove worktree ${wt.name}`} onClose={onClose} width={460}>
      <div className="modal-body">
        <p className="mono">{wt.pathDisplay}</p>
        {wt.caps.remove ? (
          <>
            <p className="muted">
              Runs <span className="mono">git worktree remove</span> (without <span className="mono">--force</span>): git refuses if
              it has uncommitted or untracked changes, and nothing is lost.{" "}
              {wt.branch ? (
                <>
                  Branch <span className="mono">{wt.branch}</span> is kept.
                </>
              ) : (
                "It is on a detached HEAD; commits not on a branch become hard to find."
              )}
            </p>
            <p className="form-error">The worktree folder will be deleted from disk.</p>
          </>
        ) : (
          <p className="hint hint-error">
            {wt.locked
              ? `It is locked${wt.lockReason ? ` (${wt.lockReason})` : ""}. Pitwall doesn't remove locked worktrees.`
              : wt.via === "own"
                ? "An agent works there: remove the agent (with its worktree) instead."
                : "This worktree can't be removed from Pitwall."}
          </p>
        )}
      </div>
      <footer className="modal-foot">
        <button className="ghost-btn" onClick={onClose} data-autofocus>
          Cancel
        </button>
        {wt.caps.remove && (
          <button className="danger-btn" onClick={remove} disabled={busy}>
            {busy ? "Removing…" : "Remove worktree"}
          </button>
        )}
      </footer>
    </Modal>
  );
}
