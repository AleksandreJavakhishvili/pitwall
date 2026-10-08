import { useEffect, useMemo, useRef, useState } from "react";
import type { AgentView } from "../../types";
import { api, errorText } from "../../api";
import type { FileIndex } from "../../explorerApi";
import { highlightRuns, quickOpen } from "../../lib/explorer";
import { splitPath } from "../../lib/fileTree";
import { Modal } from "../Modal";
import { FileIcon } from "../FileIcon";
import { Kbd } from "../Kbd";

// Files opened from the viewer, newest first, per agent (an empty ⌘P lists them first).
const recents = new Map<string, string[]>();

export function noteRecent(agentId: string, path: string): void {
  const list = (recents.get(agentId) ?? []).filter((p) => p !== path);
  recents.set(agentId, [path, ...list].slice(0, 20));
}

function Runs({ text, positions, offset }: { text: string; positions: number[]; offset: number }) {
  return (
    <>
      {highlightRuns(text, positions, offset).map((r, i) => (r.hit ? <mark key={i}>{r.text}</mark> : <span key={i}>{r.text}</span>))}
    </>
  );
}

/** ⌘P: a fuzzy file picker over the agent's files (ignore files respected), filtered here. */
export function QuickOpen({ agent, onPick, onClose }: { agent: AgentView; onPick(path: string): void; onClose(): void }) {
  const [index, setIndex] = useState<FileIndex | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);

  useEffect(() => {
    let alive = true;
    api.listAllFiles(agent.id).then(
      (i) => alive && setIndex(i),
      (e) => alive && setError(errorText(e)),
    );
    return () => {
      alive = false;
    };
  }, [agent.id]);

  const hits = useMemo(() => (index ? quickOpen(index.files, q, { recent: recents.get(agent.id) }) : []), [index, q, agent.id]);
  const idx = Math.min(active, Math.max(0, hits.length - 1));
  const listRef = useRef<HTMLUListElement>(null);

  const pick = (path: string | undefined) => {
    if (!path) return;
    onClose();
    onPick(path);
  };

  return (
    <Modal onClose={onClose} variant="palette" width={600}>
      <div className="palette">
        <input
          className="palette-input"
          placeholder={`Go to a file in ${agent.name}`}
          value={q}
          spellCheck={false}
          data-autofocus
          aria-label={`Go to a file in ${agent.name}`}
          onChange={(e) => {
            setQ(e.target.value);
            setActive(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setActive((i) => Math.min(hits.length - 1, i + 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setActive((i) => Math.max(0, i - 1));
            } else if (e.key === "Enter") {
              e.preventDefault();
              pick(hits[idx]?.path);
            }
          }}
        />
        <ul className="palette-list ex-quick" ref={listRef} role="listbox" aria-label="Files">
          {error && <li className="palette-empty hint-error">Couldn't list files: {error.split("\n")[0]}</li>}
          {!error && !index && <li className="palette-empty">Reading files…</li>}
          {index && hits.length === 0 && <li className="palette-empty">No matching files.</li>}
          {hits.map((h, i) => {
            const { dir, base } = splitPath(h.path);
            return (
              <li
                key={h.path}
                role="option"
                aria-selected={i === idx}
                className="palette-item"
                onMouseMove={() => setActive(i)}
                onClick={() => pick(h.path)}
                ref={(el) => {
                  if (el && i === idx) el.scrollIntoView({ block: "nearest" });
                }}
              >
                <span className="pal-lead">
                  <FileIcon path={h.path} />
                </span>
                <span className="pal-label ex-quick-label">
                  <span className="ex-quick-base">
                    <Runs text={base} positions={h.positions} offset={dir.length} />
                  </span>
                  {dir && (
                    <span className="pal-sub ex-quick-dir">
                      <Runs text={dir.replace(/\/$/, "")} positions={h.positions} offset={0} />
                    </span>
                  )}
                </span>
              </li>
            );
          })}
        </ul>
        {index?.truncated && <p className="hint ex-quick-foot">Only the first 100 000 files are listed.</p>}
        <p className="hint ex-quick-foot">
          Read-only · <Kbd>↵</Kbd> opens in the viewer · {agent.cwdDisplay}
        </p>
      </div>
    </Modal>
  );
}
