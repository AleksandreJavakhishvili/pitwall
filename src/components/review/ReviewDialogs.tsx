import { useEffect, useState } from "react";
import type { AgentView, FileChange } from "../../types";
import type { MergeResult, MergeStatus } from "../../reviewTypes";
import { api, errorText } from "../../api";
import { useActions } from "../../lib/actions";
import { Modal } from "../Modal";
import { conflictPrompt } from "./comments";

const busyStatus = (a: AgentView) => a.status === "working" || a.status === "blocked";

/**
 * An editable prompt, shown in full before anything is sent. Sent verbatim:
 * now via send_prompt, or into Next up when the agent is busy / not running.
 */
export function PromptDialog({
  agent: a,
  title,
  intro,
  initial,
  onClose,
  onSent,
}: {
  agent: AgentView;
  title: string;
  intro?: React.ReactNode;
  initial: string;
  onClose(): void;
  onSent?(): void;
}) {
  const { run, patch } = useActions();
  const [text, setText] = useState(initial);
  const [busy, setBusy] = useState(false);
  const queueFirst = !a.running || busyStatus(a);

  const send = async (how: "now" | "queue") => {
    setBusy(true);
    const ok =
      how === "now"
        ? await run(api.sendPrompt(a.id, text).then(() => true), `send to ${a.name}`)
        : await run(
            api.queueAdd(a.id, text).then((v) => {
              patch(v);
              return true;
            }),
            `add to ${a.name}'s Next up`,
          );
    setBusy(false);
    if (ok) {
      onSent?.();
      onClose();
    }
  };

  return (
    <Modal title={title} onClose={onClose} width={620}>
      <div className="modal-body">
        {intro}
        <textarea
          className="rv-prompt mono"
          value={text}
          rows={Math.min(16, Math.max(5, text.split("\n").length + 1))}
          spellCheck={false}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && text.trim()) {
              e.preventDefault();
              send(queueFirst ? "queue" : "now");
            }
          }}
          data-autofocus
        />
        <p className="hint">
          Sent to <strong>{a.name}</strong> exactly as written above.{" "}
          {queueFirst && (a.running ? `${a.name} is busy, so it goes to Next up.` : `${a.name} isn't running, so it goes to Next up.`)}
        </p>
      </div>
      <footer className="modal-foot">
        <button className="ghost-btn" onClick={onClose}>
          Cancel
        </button>
        {queueFirst ? (
          <>
            {a.running && (
              <button className="ghost-btn" onClick={() => send("now")} disabled={busy || !text.trim()}>
                Send now anyway
              </button>
            )}
            <button className="primary-btn" onClick={() => send("queue")} disabled={busy || !text.trim()}>
              Add to Next up
            </button>
          </>
        ) : (
          <>
            <button className="ghost-btn" onClick={() => send("queue")} disabled={busy || !text.trim()}>
              Add to Next up
            </button>
            <button className="primary-btn" onClick={() => send("now")} disabled={busy || !text.trim()}>
              Send now
            </button>
          </>
        )}
      </footer>
    </Modal>
  );
}

export function DiscardDialog({
  agent: a,
  file,
  scoped,
  onClose,
  onDone,
}: {
  agent: AgentView;
  file: FileChange;
  scoped: boolean;
  onClose(): void;
  onDone(msg: string): void;
}) {
  const { run } = useActions();
  const [busy, setBusy] = useState(false);
  const discard = async () => {
    setBusy(true);
    const ok = await run(api.discardFile(a.id, file.path).then(() => true), `discard ${file.path}`);
    setBusy(false);
    if (ok) {
      onDone(`Discarded ${file.path}`);
      onClose();
    }
  };
  return (
    <Modal title="Discard file" onClose={onClose} width={480}>
      <div className="modal-body">
        <p>
          <span className="mono">{file.path}</span>
        </p>
        <p className="muted">
          {file.untracked
            ? `This file is new; it will be deleted from ${a.cwdDisplay}.`
            : `Restores the file to how it was when ${a.name} started (its base commit), in ${a.cwdDisplay}.`}
          {scoped && " This undoes all of the agent's changes to this file, not only this task's."}
        </p>
        <p className="hint hint-error">This can't be undone.</p>
      </div>
      <footer className="modal-foot">
        <button className="ghost-btn" onClick={onClose} data-autofocus>
          Cancel
        </button>
        <button className="danger-btn" onClick={discard} disabled={busy}>
          {busy ? "Discarding…" : "Discard"}
        </button>
      </footer>
    </Modal>
  );
}

/** What is committed and merged: an agent's folder, or one worktree. */
export interface CommitTarget {
  /** Shown in the title ("api-fix", "worktree agent-a1f3"). */
  name: string;
  /** Where the commit runs. */
  where: string;
  /** The main checkout merges go into. */
  projectDisplay: string;
  status(): Promise<MergeStatus>;
  commit(message: string): Promise<string>;
  merge(): Promise<MergeResult>;
}

/** An agent's folder as a commit target. */
export function agentTarget(a: AgentView): CommitTarget {
  return {
    name: a.name,
    where: a.cwdDisplay,
    projectDisplay: a.projectDisplay,
    status: () => api.getMergeStatus(a.id),
    commit: (m) => api.commitAgent(a.id, m),
    merge: () => api.mergeAgent(a.id),
  };
}

/** Commit (and for worktrees, merge) — each step spelled out and confirmed. */
export function CommitMergeDialog({
  target: t,
  onClose,
  onDone,
  onConflict,
}: {
  target: CommitTarget;
  onClose(): void;
  onDone(msg: string): void;
  onConflict(r: MergeResult): void;
}) {
  const [st, setSt] = useState<MergeStatus | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [alsoMerge, setAlsoMerge] = useState(true);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    t.status()
      .then(setSt)
      .catch((e) => setLoadErr(errorText(e)));
  }, [t.name, t.where]); // eslint-disable-line react-hooks/exhaustive-deps

  const needCommit = !!st && st.uncommitted > 0;
  const canMerge = !!st && st.worktree && !!st.target && !!st.branch;
  const willMerge = canMerge && (needCommit ? alsoMerge : st!.ahead > 0) && !st!.targetDirty;
  const nothing = !!st && !needCommit && !(canMerge && st.ahead > 0);

  const go = async () => {
    if (!st) return;
    setBusy(true);
    setErr(null);
    try {
      let done = "";
      if (needCommit) {
        const id = await t.commit(message);
        done = `Committed ${id} on ${st.branch ?? "the current branch"}.`;
      }
      if (willMerge) {
        const r = await t.merge();
        if (r.conflict) {
          onConflict(r);
          return;
        }
        if (!r.merged) {
          setErr((done ? done + " " : "") + r.message);
          setSt(await t.status().catch(() => st));
          return;
        }
        done = (done ? done + " " : "") + r.message;
      }
      onDone(done);
      onClose();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const title = st?.worktree ? "Commit & merge" : "Commit";
  const primary = !st ? "…" : needCommit ? (willMerge ? "Commit & merge" : "Commit") : "Merge";

  return (
    <Modal title={`${title} · ${t.name}`} onClose={onClose} width={540}>
      <div className="modal-body">
        {loadErr && <p className="form-error">Couldn't read git status: {loadErr}</p>}
        {!st && !loadErr && <p className="hint">Reading git…</p>}
        {st && (
          <>
            {needCommit ? (
              <div className="field">
                <label className="field-label" htmlFor="rv-commit-msg">
                  Commit message
                </label>
                <textarea
                  id="rv-commit-msg"
                  rows={3}
                  value={message}
                  onChange={(e) => setMessage(e.target.value)}
                  placeholder="Describe the change"
                  data-autofocus
                />
                <span className="hint">
                  Runs <span className="mono">git add -A && git commit</span> in{" "}
                  <span className="mono">{t.where}</span>
                  {st.branch && (
                    <>
                      {" "}
                      on <span className="mono">{st.branch}</span>
                    </>
                  )}{" "}
                  — {st.uncommitted} uncommitted {st.uncommitted === 1 ? "entry" : "entries"}.
                  {!st.worktree && " This is the main checkout: everything in it is committed, including changes not made by this agent."}
                </span>
              </div>
            ) : (
              <p className="muted">Nothing uncommitted in {t.where}.</p>
            )}

            {st.worktree && (
              <div className="confirm-box">
                {needCommit ? (
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={alsoMerge && canMerge && !st.targetDirty}
                      disabled={st.targetDirty || !canMerge}
                      onChange={(e) => setAlsoMerge(e.target.checked)}
                    />
                    <span>
                      Then merge <span className="mono">{st.branch}</span> into{" "}
                      <span className="mono">{st.target ?? "?"}</span>
                    </span>
                  </label>
                ) : (
                  <span>
                    Merge <span className="mono">{st.branch}</span> into <span className="mono">{st.target ?? "?"}</span>
                    {st.ahead > 0 ? ` (${st.ahead} commit${st.ahead === 1 ? "" : "s"})` : " — nothing to merge"}
                  </span>
                )}
                <span className="hint">
                  <span className="mono">git merge --no-ff</span> in the main checkout{" "}
                  <span className="mono">{t.projectDisplay}</span>. On conflicts the merge is aborted and nothing changes.
                </span>
                {st.targetDirty && (
                  <span className="hint hint-error">
                    The main checkout has uncommitted changes, so merging is refused. Commit or stash them first.
                  </span>
                )}
                {!st.target && <span className="hint hint-error">The main checkout is on a detached HEAD.</span>}
                {!st.branch && (
                  <span className="hint hint-error">
                    The worktree isn't on a branch (detached HEAD). Create a branch there to merge it.
                  </span>
                )}
              </div>
            )}
            {err && <p className="form-error">{err}</p>}
          </>
        )}
      </div>
      <footer className="modal-foot">
        <button className="ghost-btn" onClick={onClose}>
          Cancel
        </button>
        <button
          className="primary-btn"
          onClick={go}
          disabled={!st || busy || nothing || (needCommit && !message.trim()) || (!needCommit && !willMerge)}
        >
          {busy ? "Working…" : primary}
        </button>
      </footer>
    </Modal>
  );
}

export function ConflictDialog({ agent, result, onClose }: { agent: AgentView; result: MergeResult; onClose(): void }) {
  return (
    <PromptDialog
      agent={agent}
      title="Merge conflict"
      intro={<p className="form-error">{result.message}</p>}
      initial={conflictPrompt(result.branch)}
      onClose={onClose}
    />
  );
}
