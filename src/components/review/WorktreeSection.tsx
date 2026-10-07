import { useEffect } from "react";
import type { FileChange } from "../../types";
import { forceRefreshWorktrees, useWorktreeFiles, useWorktreeStatus } from "../../lib/useWorktrees";
import { FreshError, RefreshControl } from "../Freshness";
import { refKey, type WorktreeRef } from "../../lib/worktrees";
import { treeOrder } from "../../lib/fileTree";
import { DiffStat } from "../DiffStat";
import { Icon } from "../Icon";
import { FileTree } from "./FileTree";

interface Props {
  r: WorktreeRef;
  open: boolean;
  /** It is what the main pane shows. */
  current: boolean;
  selected: string | null;
  nonce: number;
  onToggle(): void;
  /** Show it (its first file, if any). */
  onShow(first: string | null): void;
  onSelect(path: string): void;
  isClosed(dir: string): boolean;
  onToggleDir(dir: string): void;
  onFiles(key: string, files: FileChange[] | null, error: string | null): void;
}

/** One worktree in Review's list: its changes against the merge-base, read only while open or shown. */
export function WorktreeSection({ r, open, current, selected, nonce, onToggle, onShow, onSelect, isClosed, onToggleDir, onFiles }: Props) {
  const { wt } = r;
  const key = refKey(r);
  const { files, error, refresh } = useWorktreeFiles(r.projectId, wt.path, wt.head, (open || current) && wt.caps.diff, nonce, 10_000);
  const st = useWorktreeStatus();
  const again = () => {
    void forceRefreshWorktrees(r.projectId).catch(() => {});
    refresh();
  };
  useEffect(() => onFiles(key, files, error), [key, files, error]); // eslint-disable-line react-hooks/exhaustive-deps
  const added = files?.reduce((s, f) => s + f.added, 0) ?? 0;
  const removed = files?.reduce((s, f) => s + f.removed, 0) ?? 0;
  return (
    <section className="rv-agent rv-wt" data-current={current}>
      <div className="rv-agent-head">
        <button className="rv-agent-toggle" onClick={onToggle} aria-expanded={open} aria-label={open ? "Collapse" : "Expand"}>
          <span className="chev" data-open={open}>
            <Icon name="chevron" size={12} />
          </span>
        </button>
        <button
          className="rv-agent-name"
          onClick={() => onShow(files && files.length ? treeOrder(files)[0].path : null)}
          title={`${wt.pathDisplay}${wt.lockReason ? `\nLocked: ${wt.lockReason}` : wt.locked ? "\nLocked" : ""}`}
        >
          <Icon name="branch" size={12} />
          <span className="rv-agent-label">{wt.name}</span>
          <span className="rv-branch mono">{wt.branch ?? "detached"}</span>
          {wt.locked && <span className="chip chip-warn">locked</span>}
        </button>
        {files && files.length > 0 ? <DiffStat added={added} removed={removed} /> : null}
        {open && <RefreshControl note={false} refreshing={st.refreshing} updatedAt={st.updatedAt} onRefresh={again} label={`Refresh ${wt.name}`} />}
      </div>
      {open && (
        <>
          {!wt.caps.diff && <p className="hint rv-pad">Its folder is gone.</p>}
          {error && (
            <div className="rv-pad">
              <FreshError error={error} onRetry={again} />
            </div>
          )}
          {wt.caps.diff && !error && !files && <p className="hint rv-pad">Reading git…</p>}
          {!error && files?.length === 0 && <p className="hint rv-pad">No changes against {r.target ?? "the project's branch"}.</p>}
          {files && files.length > 0 && (
            <FileTree
              files={files}
              selected={selected}
              commentCounts={{}}
              isClosed={isClosed}
              onToggle={onToggleDir}
              onSelect={onSelect}
            />
          )}
        </>
      )}
    </section>
  );
}
