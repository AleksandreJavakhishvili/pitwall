import { useEffect, useState } from "react";
import type { ApprovalView } from "../types";
import { api, errorText } from "../api";
import { useNow } from "../lib/useNow";
import { Modal } from "./Modal";

/** "1:54" until the request is denied for lack of an answer. */
export function timeLeft(expiresAt: number, now: number): string {
  const s = Math.max(0, Math.round((expiresAt - now) / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Who asked, as Pitwall established it from the calling process. */
export function requesterLine(a: ApprovalView): string {
  const r = a.requester;
  const who = r.kind === "agent" ? `agent “${r.name}” in Pitwall` : "a process outside Pitwall's agents";
  const proc = [r.process, r.pid != null ? `pid ${r.pid}` : null].filter(Boolean).join(", ");
  return proc ? `${who} (${proc})` : who;
}

/**
 * Pitwall's approval dialog: a request from the `pitwall` CLI (an agent, a
 * script, a terminal) that changes something outside Pitwall waits here.
 * Only this window can answer it; closing it denies.
 */
export function ApprovalDialog() {
  const [list, setList] = useState<ApprovalView[]>([]);
  const [remember, setRemember] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const now = useNow(1000);

  useEffect(() => {
    let live = true;
    const off = api.onApprovalsChanged((l) => live && setList(l));
    api.listApprovals().then((l) => live && setList(l)).catch(() => {});
    return () => {
      live = false;
      off.then((f) => f()).catch(() => {});
    };
  }, []);

  const current = list[0];
  useEffect(() => {
    setRemember(false);
    setError(null);
  }, [current?.id]);
  if (!current) return null;

  const answer = (allow: boolean) => {
    api.answerApproval(current.id, allow, allow && remember && current.rememberable).catch((e) => setError(errorText(e)));
  };

  return (
    <Modal title="Approval needed" onClose={() => answer(false)} width={500} className="approval-modal">
      <div className="modal-body approval">
        <p className="approval-ask">
          <strong>{current.requester.name}</strong> wants to {current.summary}.
        </p>
        {current.details.length > 0 && (
          <ul className="approval-details">
            {current.details.map((d) => (
              <li key={d}>{d}</li>
            ))}
          </ul>
        )}
        <p className="hint">Asked by {requesterLine(current)}. Pitwall decides who asked; the caller can't approve this itself.</p>
        {current.rememberable && (
          <label className="check">
            <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
            <span>
              Allow {current.requester.kind === "agent" ? current.requester.name : "processes outside Pitwall"} to do this
              again without asking
              <span className="hint block">Until Pitwall quits.</span>
            </span>
          </label>
        )}
        {error && <p className="hint-error">{error}</p>}
        <div className="row gap approval-actions">
          <span className="muted-sm">
            Denied in {timeLeft(current.expiresAt, now)}
            {list.length > 1 ? ` · ${list.length - 1} more waiting` : ""}
          </span>
          <span className="spacer" />
          <button className="ghost-btn" onClick={() => answer(false)}>
            Deny
          </button>
          <button className="primary-btn approval-allow" onClick={() => answer(true)}>
            Allow
          </button>
        </div>
      </div>
    </Modal>
  );
}
