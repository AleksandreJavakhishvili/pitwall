import { useEffect, useMemo, useRef, useState } from "react";
import type { AgentView, FileChange } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { DiffStat } from "./DiffStat";
import { FileIcon, FileStats } from "./FileIcon";
import { fileStatus, splitPath } from "../lib/fileTree";
import { noteAccessError } from "../lib/permissions";
import { forceRefreshWorktrees, useWorktreeList, useWorktreeStatus } from "../lib/useWorktrees";
import { useFresh, useRefreshRequest } from "../lib/freshness";
import { FreshError, RefreshControl } from "./Freshness";
import { countLabel, worktreesByAgent } from "../lib/worktrees";
import { WorktreeRows } from "./WorktreeRows";
import { Icon } from "./Icon";

/** Changes are polled this often while the panel is shown and the window visible. */
export const CHANGES_POLL_MS = 5_000;

/** Changed files for an agent: a forced refresh when the panel opens for it (the backend's
 * polling pace is bypassed, so totals and branch are current too), then read when its totals
 * move and every 5 s while the window is visible; ↻ / ⌘⇧R force again. Nothing is fetched
 * when its changes can't be read (`caps.diff`). */
function useChanges(a: AgentView) {
  const fresh = useFresh<FileChange[]>(a.caps.diff ? a.id : null, () => ({
    load: () => api.getChanges(a.id),
    force: () => api.refreshChanges(a.id),
    everyMs: CHANGES_POLL_MS,
  }));
  const sig = `${a.added}:${a.removed}:${a.filesChanged}`;
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    void fresh.poll();
  }, [sig]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (fresh.error) noteAccessError(fresh.error);
  }, [fresh.error]);
  return fresh;
}

/** The agent's other worktrees (docs/spec/worktrees-view.md), collapsed until asked for;
 * expanding lists them again now. */
function AgentWorktrees({ agent: a }: { agent: AgentView }) {
  const projects = useWorktreeList();
  const refs = useMemo(() => worktreesByAgent(projects).get(a.id) ?? [], [projects, a.id]);
  const [open, setOpen] = useState(false);
  const [nonce, setNonce] = useState(0);
  const st = useWorktreeStatus();
  const projectIds = useMemo(() => [...new Set(refs.map((r) => r.projectId))], [refs]);
  const refresh = () => {
    setNonce((n) => n + 1);
    return Promise.all(projectIds.map((id) => forceRefreshWorktrees(id))).catch(() => {});
  };
  useRefreshRequest(() => open && void refresh());
  if (!refs.length) return null;
  return (
    <div className="panel-wts">
      <div className="panel-wts-head">
        <button
          className="wt-chip"
          aria-expanded={open}
          onClick={() => {
            if (!open) void refresh();
            setOpen(!open);
          }}
        >
          <span className="chev" data-open={open}>
            <Icon name="chevron" size={10} />
          </span>
          <Icon name="branch" size={11} />
          {countLabel(refs.length)}
        </button>
        {open && (
          <>
            <span className="spacer" />
            <RefreshControl refreshing={st.refreshing} updatedAt={st.updatedAt} onRefresh={() => void refresh()} label="Refresh worktrees" />
          </>
        )}
      </div>
      {open && st.error && <FreshError error={st.error} prefix="Couldn't list worktrees: " onRetry={() => void refresh()} />}
      {open && <WorktreeRows refs={refs} label={`Worktrees of ${a.name}`} nonce={nonce} />}
    </div>
  );
}

/** Branch and folder of the agent's checkout; click the path to copy it. */
function WhereLine({ agent: a }: { agent: AgentView }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1200);
    return () => clearTimeout(t);
  }, [copied]);
  return (
    <div className="where">
      <span className="where-branch mono" title={a.worktree ? "Works in its own git worktree" : "Current branch"}>
        <Icon name="branch" size={11} />
        <span className="where-text">{a.branch ?? "detached HEAD"}</span>
        {a.worktree && <span className="where-tag">worktree</span>}
      </span>
      <button
        className="where-path mono"
        title={copied ? "Copied" : `${a.cwd} — click to copy`}
        onClick={() => void navigator.clipboard?.writeText(a.cwd).then(() => setCopied(true), () => {})}
      >
        <Icon name="folder" size={11} />
        <span className="where-text">{copied ? "Copied" : a.cwdDisplay}</span>
      </button>
    </div>
  );
}

export function Changes({ agent: a }: { agent: AgentView }) {
  const { openDiff } = useActions();
  const { data: files, error, refreshing, updatedAt, refresh } = useChanges(a);
  if (!a.caps.diff) {
    return (
      <section className="panel-section panel-grow">
        <div className="section-head">
          <span className="label">Changes</span>
        </div>
        <p className="hint">Not a git repository — Pitwall can't track changes in this folder.</p>
      </section>
    );
  }
  return (
    <section className="panel-section panel-grow">
      <div className="section-head">
        <span className="label">Changes</span>
        {files && files.length > 0 && <span className="label-count">{files.length}</span>}
        <span className="spacer" />
        <RefreshControl refreshing={refreshing} updatedAt={updatedAt} onRefresh={() => void refresh()} label="Refresh changes" />
        <DiffStat added={a.added} removed={a.removed} />
      </div>
      <WhereLine agent={a} />
      {error === "not-a-git-repo" && (
        <p className="hint">Not a git repository — Pitwall can't track changes in this folder.</p>
      )}
      {error && error !== "not-a-git-repo" && <FreshError error={error} prefix="Couldn't read changes: " onRetry={() => void refresh()} />}
      {!error && files === null && <p className="hint">Reading git…</p>}
      {!error && files?.length === 0 && <p className="hint">No changes since the agent started.</p>}
      {files && files.length > 0 && (
        <ul className="file-list">
          {files.map((f) => {
            const { dir, base } = splitPath(f.path);
            return (
              <li key={f.path}>
                <button className="file-row" data-status={fileStatus(f)} onClick={() => openDiff(a.id, f)} title={f.path}>
                  <FileIcon path={f.path} />
                  <span className="file-path">
                    <span className="file-dir">{dir}</span>
                    <span className="file-base">{base}</span>
                  </span>
                  <FileStats file={f} />
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <AgentWorktrees agent={a} />
    </section>
  );
}
