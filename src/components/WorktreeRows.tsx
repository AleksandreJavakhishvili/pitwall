import { useState, type MouseEvent } from "react";
import { useActions } from "../lib/actions";
import { useWorktreeFiles } from "../lib/useWorktrees";
import type { WorktreeRef } from "../lib/worktrees";
import { DiffStat } from "./DiffStat";
import { Icon } from "./Icon";
import { Menu, type MenuItem } from "./Menu";

/** The actions a worktree offers (by its caps): Review, a terminal there, remove. */
export function useWorktreeMenu() {
  const { openReview, openTerminal, openRemoveWorktree } = useActions();
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; r: WorktreeRef } | null>(null);
  const items = (r: WorktreeRef): MenuItem[] => [
    { id: "review", icon: <Icon name="review" size={14} />, label: "Review changes", run: () => openReview({ projectId: r.projectId, path: r.wt.path }) },
    ...(r.wt.caps.terminal
      ? [{ id: "terminal", icon: <Icon name="terminal" size={14} />, label: "Open terminal here", run: () => openTerminal(r.wt.path) }]
      : []),
    ...(r.wt.caps.remove
      ? [{ id: "remove", icon: <Icon name="trash" size={14} />, label: "Remove worktree…", run: () => openRemoveWorktree(r.projectId, r.wt.path) }]
      : []),
  ];
  const open = (e: MouseEvent, r: WorktreeRef) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ at: { x: e.clientX, y: e.clientY }, r });
  };
  const node = menu && <Menu at={menu.at} label={menu.r.wt.name} items={items(menu.r)} onClose={() => setMenu(null)} />;
  return { open, node };
}

function WorktreeRow({ r, nonce, onMenu }: { r: WorktreeRef; nonce: number; onMenu(e: MouseEvent, r: WorktreeRef): void }) {
  const { openReview } = useActions();
  const { wt } = r;
  // Read only while shown (the list is expanded).
  const { files, error } = useWorktreeFiles(r.projectId, wt.path, wt.head, wt.caps.diff, nonce);
  const added = files?.reduce((s, f) => s + f.added, 0) ?? 0;
  const removed = files?.reduce((s, f) => s + f.removed, 0) ?? 0;
  return (
    <li>
      <button
        className="wt-row"
        onClick={() => openReview({ projectId: r.projectId, path: wt.path })}
        onContextMenu={(e) => onMenu(e, r)}
        title={`${wt.pathDisplay}${wt.via === "process" ? "\nA process of this agent works here" : ""}${error ? `\n${error}` : ""}`}
      >
        <Icon name="branch" size={12} />
        <span className="wt-name mono">{wt.branch ?? `${wt.name} (detached)`}</span>
        {wt.locked && (
          <span className="chip chip-warn" title={wt.lockReason ?? "locked"}>
            locked
          </span>
        )}
        {wt.prunable && <span className="chip chip-subtle" title="Its folder is gone">gone</span>}
        {files && <DiffStat added={added} removed={removed} />}
      </button>
    </li>
  );
}

/** Worktrees as compact rows (branch, +/−, locked); their changes are read only while shown
 * (and again when `nonce` changes: a refresh). */
export function WorktreeRows({ refs, label, nonce = 0 }: { refs: WorktreeRef[]; label: string; nonce?: number }) {
  const menu = useWorktreeMenu();
  return (
    <>
      <ul className="wt-list" aria-label={label}>
        {refs.map((r) => (
          <WorktreeRow key={r.wt.path} r={r} nonce={nonce} onMenu={menu.open} />
        ))}
      </ul>
      {menu.node}
    </>
  );
}
