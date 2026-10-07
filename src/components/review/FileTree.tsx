import { useMemo } from "react";
import type { FileChange } from "../../types";
import { buildTree, fileStatus, type TreeNode } from "../../lib/fileTree";
import { FileIcon, FileStats, FolderIcon } from "../FileIcon";
import { Icon } from "../Icon";

interface Props {
  files: FileChange[];
  selected: string | null;
  commentCounts: Record<string, number>;
  isClosed(dir: string): boolean;
  onToggle(dir: string): void;
  onSelect(path: string): void;
}

const indent = (depth: number) => ({ paddingLeft: 6 + depth * 12 });

/** Collapsible, compact-folder file tree for one agent (VS Code SCM tree view). */
export function FileTree({ files, selected, commentCounts, isClosed, onToggle, onSelect }: Props) {
  const tree = useMemo(() => buildTree(files), [files]);

  const render = (nodes: TreeNode[], depth: number): React.ReactNode =>
    nodes.map((n) => {
      if (n.kind === "dir") {
        const open = !isClosed(n.path);
        return (
          <li key={`d:${n.path}`}>
            <button className="file-row rv-dir-row" style={indent(depth)} onClick={() => onToggle(n.path)} aria-expanded={open} title={n.path}>
              <span className="chev" data-open={open}>
                <Icon name="chevron" size={10} />
              </span>
              <FolderIcon path={n.path} open={open} />
              <span className="file-path rv-dir-name">{n.name}</span>
            </button>
            {open && <ul className="rv-tree-group">{render(n.children, depth + 1)}</ul>}
          </li>
        );
      }
      const c = commentCounts[n.path] ?? 0;
      return (
        <li key={n.path}>
          <button
            className="file-row"
            data-file-row={n.path}
            data-status={fileStatus(n.file)}
            style={indent(depth)}
            aria-current={selected === n.path}
            onClick={() => onSelect(n.path)}
            title={n.path}
          >
            <span className="rv-chev-space" />
            <FileIcon path={n.path} />
            <span className="file-path">
              <span className="file-base">{n.name}</span>
            </span>
            {c > 0 && <span className="rv-ccount">{c}</span>}
            <FileStats file={n.file} />
          </button>
        </li>
      );
    });

  return <ul className="file-list rv-tree">{render(tree, 0)}</ul>;
}
