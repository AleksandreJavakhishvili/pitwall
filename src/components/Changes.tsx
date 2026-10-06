import { useEffect, useState } from "react";
import type { AgentView, FileChange } from "../types";
import { api, errorText } from "../api";
import { useActions } from "../lib/actions";
import { DiffStat } from "./DiffStat";

/** Changed files for an agent; refetched when its totals move, and every 5s. */
function useChanges(a: AgentView) {
  const [files, setFiles] = useState<FileChange[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const sig = `${a.id}:${a.added}:${a.removed}:${a.filesChanged}`;

  useEffect(() => setFiles(null), [a.id]);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .getChanges(a.id)
        .then((f) => {
          if (!alive) return;
          setFiles(f);
          setError(null);
        })
        .catch((e) => alive && setError(errorText(e)));
    load();
    const t = setInterval(load, 5000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [sig]); // eslint-disable-line react-hooks/exhaustive-deps

  return { files, error };
}

function splitPath(p: string) {
  const i = p.lastIndexOf("/");
  return i < 0 ? { dir: "", base: p } : { dir: p.slice(0, i + 1), base: p.slice(i + 1) };
}

export function Changes({ agent: a }: { agent: AgentView }) {
  const { openDiff } = useActions();
  const { files, error } = useChanges(a);
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
                <button className="file-row" onClick={() => openDiff(a.id, f)} title={f.path}>
                  <span className="file-path">
                    <span className="file-dir">{dir}</span>
                    <span className="file-base">{base}</span>
                  </span>
                  {f.untracked && <span className="chip chip-new">new</span>}
                  {f.binary ? <span className="chip chip-subtle">bin</span> : <DiffStat added={f.added} removed={f.removed} />}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
