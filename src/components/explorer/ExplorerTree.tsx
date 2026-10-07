import { useMemo, useRef } from "react";
import type { FileEntry } from "../../explorerApi";
import { treeRows, type TreeModel, type TreeState } from "../../lib/explorer";
import { STATUS_TITLE } from "../../lib/fileTree";
import { FileIcon, FolderIcon } from "../FileIcon";
import { Icon } from "../Icon";

interface Props {
  state: TreeState;
  model: TreeModel;
  /** The file shown in the viewer. */
  selected?: string | null;
  onOpen(path: string): void;
  label: string;
}

const indent = (depth: number) => ({ paddingLeft: 6 + depth * 12 });

/** The change letter (as in Changes), or a dot on a folder with changes below it. */
function Mark({ e }: { e: FileEntry }) {
  if (e.status) {
    return (
      <span className="status-letter" data-status={e.status} title={STATUS_TITLE[e.status]} aria-label={STATUS_TITLE[e.status]}>
        {e.status}
      </span>
    );
  }
  if (e.kind === "dir" && e.changes > 0) {
    const n = e.changes === 1 ? "1 change" : `${e.changes} changes`;
    return <span className="ex-dot" title={`${n} inside`} aria-label={`${n} inside`} />;
  }
  return null;
}

/**
 * An agent's folder as VS Code's Explorer shows it: lazy folders, file icons,
 * change letters; ↑/↓ move, → / ← open and close folders, ↵ opens a file.
 */
export function ExplorerTree({ state, model, selected = null, onOpen, label }: Props) {
  const rows = useMemo(() => treeRows(state), [state]);
  const ref = useRef<HTMLUListElement>(null);

  const rowEls = () => [...(ref.current?.querySelectorAll<HTMLButtonElement>("[data-ex-row]") ?? [])];
  const onKey = (ev: React.KeyboardEvent) => {
    if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
    const els = rowEls();
    const cur = (ev.target as HTMLElement).closest<HTMLButtonElement>("[data-ex-row]");
    const i = cur ? els.indexOf(cur) : -1;
    const go = (j: number) => {
      const el = els[Math.max(0, Math.min(els.length - 1, j))];
      el?.focus();
      el?.scrollIntoView({ block: "nearest" });
    };
    const path = cur?.dataset.exRow ?? "";
    const isDir = cur?.dataset.kind === "dir";
    if (ev.key === "ArrowDown") go(i + 1);
    else if (ev.key === "ArrowUp") go(i - 1);
    else if (ev.key === "ArrowRight" && isDir) {
      if (!model.isOpen(path)) void model.expand(path);
      else go(i + 1);
    } else if (ev.key === "ArrowLeft" && cur) {
      if (isDir && model.isOpen(path)) model.collapse(path);
      else {
        const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : null;
        if (parent !== null) go(els.findIndex((e) => e.dataset.exRow === parent));
      }
    } else return;
    ev.preventDefault();
  };

  return (
    <ul className="file-list ex-tree" ref={ref} role="tree" aria-label={label} onKeyDown={onKey}>
      {rows.map((r) => {
        if (r.kind === "loading") {
          return (
            <li key={`l:${r.dir}`} className="ex-note" style={indent(r.depth)}>
              Reading…
            </li>
          );
        }
        if (r.kind === "error") {
          return (
            <li key={`e:${r.dir}`} className="ex-note ex-note-error" style={indent(r.depth)} title={r.error}>
              {r.error.split("\n")[0]}
            </li>
          );
        }
        if (r.kind === "truncated") {
          return (
            <li key={`t:${r.dir}`} className="ex-note" style={indent(r.depth)}>
              Only the first 5 000 entries are shown
            </li>
          );
        }
        const e = r.entry;
        const dir = e.kind === "dir";
        return (
          <li key={e.path} role="treeitem" aria-expanded={dir ? r.open : undefined} aria-selected={selected === e.path}>
            <button
              className="file-row ex-row"
              data-ex-row={e.path}
              data-kind={e.kind}
              data-status={e.status ?? undefined}
              data-ignored={e.ignored || undefined}
              aria-current={selected === e.path}
              style={indent(r.depth)}
              title={e.kind === "symlink" ? `${e.path} (symbolic link)` : e.path}
              onClick={() => (dir ? void model.toggle(e.path) : onOpen(e.path))}
            >
              {dir ? (
                <span className="chev" data-open={r.open}>
                  <Icon name="chevron" size={10} />
                </span>
              ) : (
                <span className="rv-chev-space" />
              )}
              {dir ? <FolderIcon path={e.path} open={r.open} /> : <FileIcon path={e.path} />}
              <span className="file-path">
                <span className={dir ? "ex-dir-name" : "file-base"}>{e.name}</span>
                {e.kind === "symlink" && <span className="ex-link" aria-label="symbolic link"> ↗</span>}
              </span>
              <Mark e={e} />
            </button>
          </li>
        );
      })}
    </ul>
  );
}
