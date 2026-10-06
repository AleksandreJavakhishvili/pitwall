import { useEffect, useMemo, useState } from "react";
import type { AgentView, FileChange } from "../types";
import { api, errorText } from "../api";
import { parseUnifiedDiff } from "../lib/diff";
import { Modal } from "./Modal";
import { DiffStat } from "./DiffStat";
import { FileIcon, StatusLetter } from "./FileIcon";
import { splitPath } from "../lib/fileTree";

interface Props {
  agent: AgentView | null;
  file: FileChange;
  onClose(): void;
}

export function DiffView({ agent, file, onClose }: Props) {
  const [diff, setDiff] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!agent) return;
    let alive = true;
    api
      .getFileDiff(agent.id, file.path, file.untracked)
      .then((d) => alive && setDiff(d))
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [agent?.id, file.path, file.untracked]); // eslint-disable-line react-hooks/exhaustive-deps

  const lines = useMemo(() => (diff ? parseUnifiedDiff(diff) : []), [diff]);
  if (!agent) return null;

  return (
    <Modal onClose={onClose} variant="sheet" width={1080} className="diff-modal">
      <header className="diff-head">
        <FileIcon path={file.path} />
        <span className="diff-path mono" title={file.path}>
          <span className="file-base">{splitPath(file.path).base}</span>
          <span className="file-dir"> {splitPath(file.path).dir.replace(/\/$/, "")}</span>
        </span>
        <StatusLetter file={file} />
        {file.binary ? <span className="chip chip-subtle">bin</span> : <DiffStat added={file.added} removed={file.removed} />}
        <span className="spacer" />
        <span className="label">{agent.name}</span>
        <span className="hint">esc to close</span>
        <button className="icon-btn" onClick={onClose} aria-label="Close" data-autofocus>
          ✕
        </button>
      </header>
      <div className="diff-body">
        {error && <p className="hint hint-error pad">Couldn't load diff: {error}</p>}
        {!error && diff === null && <p className="hint pad">Loading diff…</p>}
        {!error && diff !== null && lines.length === 0 && <p className="hint pad">No textual changes.</p>}
        {lines.length > 0 && (
          <table className="diff-table">
            <tbody>
              {lines.map((l, i) =>
                l.kind === "hunk" || l.kind === "meta" ? (
                  <tr key={i} className={`dl dl-${l.kind}`}>
                    <td className="ln" />
                    <td className="ln" />
                    <td className="code">{l.text}</td>
                  </tr>
                ) : (
                  <tr key={i} className={`dl dl-${l.kind}`}>
                    <td className="ln">{l.old ?? ""}</td>
                    <td className="ln">{l.new ?? ""}</td>
                    <td className="code">
                      <span className="sign">{l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}</span>
                      {l.text}
                    </td>
                  </tr>
                ),
              )}
            </tbody>
          </table>
        )}
      </div>
    </Modal>
  );
}
