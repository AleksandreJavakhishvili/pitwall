import { useEffect, useMemo, useState } from "react";
import type { AgentView, FileChange } from "../types";
import { api, errorText } from "../api";
import { useActions } from "../lib/actions";
import { DiffStat } from "./DiffStat";
import { FileIcon, FileStats } from "./FileIcon";
import { fileStatus, splitPath } from "../lib/fileTree";
import { noteAccessError } from "../lib/permissions";
import { useWorktreeList } from "../lib/useWorktrees";
import { countLabel, worktreesByAgent } from "../lib/worktrees";
import { WorktreeRows } from "./WorktreeRows";
import { Icon } from "./Icon";

/** Changed files for an agent; refetched when its totals move, and every 5s.
 * Nothing is fetched when its changes can't be read (`caps.diff`). */
function useChanges(a: AgentView) {
  const canDiff = a.caps.diff;
  const [files, setFiles] = useState<FileChange[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const sig = `${a.id}:${a.added}:${a.removed}:${a.filesChanged}`;

  useEffect(() => setFiles(null), [a.id]);
  useEffect(() => {
    if (!canDiff) return;
    let alive = true;
    const load = () =>
      api
        .getChanges(a.id)
        .then((f) => {
          if (!alive) return;
          setFiles(f);
          setError(null);
        })
        .catch((e) => {
          if (!alive) return;
          setError(errorText(e));
          noteAccessError(errorText(e));
        });
    load();
    const t = setInterval(load, 5000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [sig, canDiff]); // eslint-disable-line react-hooks/exhaustive-deps

  return { files, error };
}

/** The agent's other worktrees (docs/spec/worktrees-view.md), collapsed until asked for. */
function AgentWorktrees({ agent: a }: { agent: AgentView }) {
  const projects = useWorktreeList();
  const refs = useMemo(() => worktreesByAgent(projects).get(a.id) ?? [], [projects, a.id]);
  const [open, setOpen] = useState(false);
  if (!refs.length) return null;
  return (
    <div className="panel-wts">
      <button className="wt-chip" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className="chev" data-open={open}>
          <Icon name="chevron" size={10} />
        </span>
        <Icon name="branch" size={11} />
        {countLabel(refs.length)}
      </button>
      {open && <WorktreeRows refs={refs} label={`Worktrees of ${a.name}`} />}
    </div>
  );
}

export function Changes({ agent: a }: { agent: AgentView }) {
  const { openDiff } = useActions();
  const { files, error } = useChanges(a);
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
        <DiffStat added={a.added} removed={a.removed} />
      </div>
      {error === "not-a-git-repo" && (
        <p className="hint">Not a git repository — Pitwall can't track changes in this folder.</p>
      )}
      {error && error !== "not-a-git-repo" && (
        <p className="hint hint-error" title={error}>
          Couldn't read changes: {error.split("\n")[0].slice(0, 160)}
        </p>
      )}
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
