import { useEffect, useMemo, useState } from "react";
import type { RecentProject } from "../types";
import { api } from "../api";
import { canPickFolder, pickFolder } from "../lib/pickFolder";
import { Modal } from "./Modal";
import { Kbd } from "./Kbd";

interface Props {
  /** Prefilled path (the folder ⌘T would use). */
  initial?: string;
  onClose(): void;
  onOpen(path: string): void;
}

/** ⌘⇧T: a terminal in a folder of your choice (native picker, recent projects or a typed path). */
export function TerminalDialog({ initial, onClose, onOpen }: Props) {
  const [path, setPath] = useState(initial ?? "");
  const [folders, setFolders] = useState<RecentProject[]>([]);

  useEffect(() => {
    Promise.all([api.recentProjects().catch(() => [] as RecentProject[]), api.listProjects().catch(() => [])]).then(([recent, listed]) => {
      setFolders([
        ...recent,
        ...listed.filter((p) => !recent.some((r) => r.path === p.path)).map((p) => ({ path: p.path, display: p.display, lastUsed: 0 })),
      ]);
    });
  }, []);

  const shown = useMemo(() => {
    const q = path.trim().toLowerCase();
    const list = q ? folders.filter((f) => f.path.toLowerCase().includes(q) || f.display.toLowerCase().includes(q)) : folders;
    return list.slice(0, 8);
  }, [folders, path]);

  const valid = path.trim().startsWith("/") || path.trim().startsWith("~");
  const browse = () => pickFolder("Open a terminal in…", path).then((p) => p && onOpen(p));

  return (
    <Modal title="New terminal" onClose={onClose} width={500}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (valid) onOpen(path.trim());
        }}
      >
        <div className="modal-body form">
          <div className="field">
            <label className="field-label" htmlFor="term-path">
              Folder
            </label>
            <div className="row gap">
              <input
                id="term-path"
                className="input mono"
                style={{ flex: 1 }}
                placeholder="~/code/project"
                value={path}
                onChange={(e) => setPath(e.target.value)}
                spellCheck={false}
                autoComplete="off"
                data-autofocus
              />
              {canPickFolder && (
                <button type="button" className="small-btn" onClick={browse}>
                  Browse…
                </button>
              )}
            </div>
          </div>
          {shown.length > 0 && (
            <ul className="term-folders" aria-label="Recent projects">
              {shown.map((f) => (
                <li key={f.path}>
                  <button type="button" className="term-folder" onClick={() => onOpen(f.path)} title={f.path}>
                    <span className="mono">{f.display}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
        <footer className="modal-foot">
          <button type="button" className="ghost-btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="primary-btn" disabled={!valid}>
            Open terminal <Kbd>↵</Kbd>
          </button>
        </footer>
      </form>
    </Modal>
  );
}
