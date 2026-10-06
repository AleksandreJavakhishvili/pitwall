import { Suspense, lazy, useCallback, useEffect, useMemo, useState } from "react";
import type { AgentView, FileChange } from "../../types";
import type { MergeResult, Task } from "../../reviewTypes";
import { api, errorText } from "../../api";
import { groupByProject } from "../../lib/groups";
import { usePersistentFlag } from "../../lib/usePersistentFlag";
import { StatusGlyph } from "../StatusGlyph";
import { DiffStat } from "../DiffStat";
import { Icon } from "../Icon";
import { Kbd } from "../Kbd";
import { commentStore, composePrompt, sortComments, useComments } from "./comments";
import { CommitMergeDialog, ConflictDialog, DiscardDialog, PromptDialog } from "./ReviewDialogs";
import "./review.css";

// Monaco is big: load it only when a file is opened.
const ReviewDiff = lazy(() => import("./ReviewDiff").then((m) => ({ default: m.ReviewDiff })));

type Dialog =
  | null
  | { type: "discard" }
  | { type: "send" }
  | { type: "commit" }
  | { type: "conflict"; result: MergeResult };

interface Sel {
  agentId: string;
  path: string;
}

function splitPath(p: string) {
  const i = p.lastIndexOf("/");
  return i < 0 ? { dir: "", base: p } : { dir: p.slice(0, i + 1), base: p.slice(i + 1) };
}

function clock(ms: number) {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function taskLabel(t: Task, n: number) {
  const prompt = t.prompt.trim().replace(/\s+/g, " ") || "(typed in the terminal)";
  const short = prompt.length > 70 ? prompt.slice(0, 69) + "…" : prompt;
  return `Task ${n} · ${clock(t.startedAt)}${t.endedAt ? "" : " · running"} · ${short}`;
}

/** Files per agent for its current scope; refetched when totals move and every 5s. */
function useAgentFiles(agents: AgentView[], scope: Record<string, string | null>, nonce: number) {
  const [files, setFiles] = useState<Record<string, FileChange[]>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const sig = agents
    .map((a) => `${a.id}:${a.added}:${a.removed}:${a.filesChanged}:${a.currentTaskId ?? ""}:${scope[a.id] ?? ""}`)
    .join("|");

  useEffect(() => {
    let alive = true;
    const load = () =>
      agents.forEach((a) => {
        const s = scope[a.id] ?? null;
        api
          .getTaskChanges(a.id, s)
          .then((f) => {
            if (!alive) return;
            setFiles((m) => (JSON.stringify(m[a.id]) === JSON.stringify(f) ? m : { ...m, [a.id]: f }));
            setErrors((m) => {
              if (!(a.id in m)) return m;
              const n = { ...m };
              delete n[a.id];
              return n;
            });
          })
          .catch((e) => alive && setErrors((m) => ({ ...m, [a.id]: errorText(e) })));
      });
    load();
    const t = setInterval(load, 5000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [sig, nonce]); // eslint-disable-line react-hooks/exhaustive-deps

  return { files, errors };
}

export default function Review({ agents, onExit }: { agents: AgentView[]; onExit(): void }) {
  const ordered = useMemo(() => groupByProject(agents).flatMap((g) => g.agents), [agents]);
  const [scope, setScope] = useState<Record<string, string | null>>({});
  const [nonce, setNonce] = useState(0);
  const { files, errors } = useAgentFiles(ordered, scope, nonce);
  const [sel, setSel] = useState<Sel | null>(null);
  const [selAgentId, setSelAgentId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [tasks, setTasks] = useState<Task[]>([]);
  const [sideBySide, setSideBySide] = usePersistentFlag("pitwall.review.sideBySide", true);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [reveal, setReveal] = useState<{ line: number; nonce: number } | null>(null);
  const comments = useComments();

  const agent = ordered.find((a) => a.id === (sel?.agentId ?? selAgentId)) ?? null;
  const agentFiles = agent ? files[agent.id] : undefined;
  const file = sel && agentFiles ? (agentFiles.find((f) => f.path === sel.path) ?? null) : null;
  const taskId = agent ? (scope[agent.id] ?? null) : null;
  const agentComments = agent ? sortComments(comments[agent.id] ?? []) : [];

  // Default selection: the first agent with changes, its first file.
  useEffect(() => {
    if (sel || selAgentId) return;
    const first = ordered.find((a) => (files[a.id]?.length ?? 0) > 0);
    if (first) setSel({ agentId: first.id, path: files[first.id][0].path });
  }, [files, ordered, sel, selAgentId]);

  // The selected file went away (discarded, merged, scope changed): pick a neighbour.
  useEffect(() => {
    if (!sel || !agentFiles || file) return;
    setSel(agentFiles[0] ? { agentId: sel.agentId, path: agentFiles[0].path } : null);
    setSelAgentId(sel.agentId);
  }, [sel, agentFiles, file]);

  // Removed agent.
  useEffect(() => {
    if (sel && !agents.some((a) => a.id === sel.agentId)) setSel(null);
  }, [agents, sel]);

  // Tasks of the selected agent (for the scope picker).
  const agentId = agent?.id ?? null;
  const currentTask = agent?.currentTaskId ?? null;
  useEffect(() => {
    if (!agentId) return setTasks([]);
    let alive = true;
    api
      .listTasks(agentId)
      .then((t) => alive && setTasks(t))
      .catch(() => alive && setTasks([]));
    return () => {
      alive = false;
    };
  }, [agentId, currentTask, nonce]);

  // Esc leaves the review (unless a dialog, the editor or a text field has it).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (document.querySelector(".backdrop")) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.(".monaco-editor, textarea, input, select, .rv-composer")) return;
      e.preventDefault();
      onExit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onExit]);

  const select = (agentId: string, path: string) => {
    setSel({ agentId, path });
    setSelAgentId(agentId);
  };

  const done = useCallback((msg: string) => {
    setNotice(msg);
    setNonce((n) => n + 1);
  }, []);

  const versionSig = agent ? `${agent.added}:${agent.removed}:${agent.filesChanged}:${nonce}:${file?.added}:${file?.removed}` : "";

  return (
    <div className="review">
      <div className="review-bar">
        <span className="label">Review</span>
        <span className="muted-sm">what your agents changed · click a line number to comment</span>
        <span className="spacer" />
        <div className="rv-seg" role="group" aria-label="Diff layout">
          <button aria-pressed={sideBySide} onClick={() => setSideBySide(true)}>
            Side by side
          </button>
          <button aria-pressed={!sideBySide} onClick={() => setSideBySide(false)}>
            Inline
          </button>
        </div>
        <button className="small-btn" onClick={onExit}>
          Back <Kbd>esc</Kbd>
        </button>
      </div>

      <div className="review-body">
        <aside className="rv-list" aria-label="Changes by agent">
          {ordered.length === 0 && <p className="hint pad">No agents yet.</p>}
          {ordered.map((a) => {
            const list = files[a.id];
            const err = errors[a.id];
            const isOpen = !collapsed[a.id];
            const cs = sortComments(comments[a.id] ?? []);
            const scoped = !!scope[a.id];
            const added = list?.reduce((s, f) => s + f.added, 0) ?? 0;
            const removed = list?.reduce((s, f) => s + f.removed, 0) ?? 0;
            return (
              <section key={a.id} className="rv-agent" data-current={agent?.id === a.id}>
                <div className="rv-agent-head">
                  <button
                    className="rv-agent-toggle"
                    onClick={() => setCollapsed((c) => ({ ...c, [a.id]: isOpen }))}
                    aria-expanded={isOpen}
                    aria-label={isOpen ? "Collapse" : "Expand"}
                  >
                    <span className="chev" data-open={isOpen}>
                      <Icon name="chevron" size={12} />
                    </span>
                  </button>
                  <button
                    className="rv-agent-name"
                    onClick={() => {
                      setSelAgentId(a.id);
                      if (list?.[0]) select(a.id, list[0].path);
                      else setSel(null);
                    }}
                    title={`${a.name} · ${a.cwdDisplay}`}
                  >
                    <StatusGlyph status={a.status} size="sm" />
                    <span className="rv-agent-label">{a.name}</span>
                    {a.branch && <span className="rv-branch mono">{a.branch}</span>}
                  </button>
                  {list && list.length > 0 ? <DiffStat added={added} removed={removed} /> : null}
                </div>
                {isOpen && (
                  <>
                    {scoped && <div className="rv-scope-note">this task only</div>}
                    {err && <p className="hint hint-error rv-pad">{err}</p>}
                    {!err && !list && <p className="hint rv-pad">Reading git…</p>}
                    {!err && list?.length === 0 && <p className="hint rv-pad">{scoped ? "No changes in this task." : "No changes."}</p>}
                    {list && list.length > 0 && (
                      <ul className="file-list">
                        {list.map((f) => {
                          const { dir, base } = splitPath(f.path);
                          const n = cs.filter((c) => c.path === f.path).length;
                          return (
                            <li key={f.path}>
                              <button
                                className="file-row"
                                aria-current={sel?.agentId === a.id && sel.path === f.path}
                                onClick={() => select(a.id, f.path)}
                                title={f.path}
                              >
                                <span className="file-path">
                                  <span className="file-dir">{dir}</span>
                                  <span className="file-base">{base}</span>
                                </span>
                                {n > 0 && <span className="rv-ccount">{n}</span>}
                                {f.untracked && <span className="chip chip-new">new</span>}
                                {f.binary ? <span className="chip chip-subtle">bin</span> : <DiffStat added={f.added} removed={f.removed} />}
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    )}
                    {cs.length > 0 && (
                      <ul className="rv-comments" aria-label={`Comments for ${a.name}`}>
                        {cs.map((c) => (
                          <li key={c.id} className="rv-comment">
                            <button
                              className="rv-comment-jump"
                              onClick={() => {
                                select(a.id, c.path);
                                setReveal({ line: c.line, nonce: Date.now() });
                              }}
                              title="Show in diff"
                            >
                              <span className="mono rv-comment-where">
                                {splitPath(c.path).base}:{c.line}
                              </span>
                              <span className="rv-comment-text">{c.text}</span>
                            </button>
                            <button className="icon-btn icon-btn-sm" onClick={() => commentStore.remove(a.id, c.id)} aria-label="Delete comment" title="Delete comment">
                              ✕
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </>
                )}
              </section>
            );
          })}
        </aside>

        <section className="rv-main">
          {agent ? (
            <>
              <div className="rv-head">
                <select
                  className="input rv-scope"
                  value={taskId ?? ""}
                  onChange={(e) => {
                    setScope((s) => ({ ...s, [agent.id]: e.target.value || null }));
                    setSelAgentId(agent.id);
                  }}
                  aria-label="Which changes"
                  title="All changes since the agent started, or one task (prompt)"
                >
                  <option value="">All changes · {agent.name}</option>
                  {[...tasks]
                    .map((t, i) => ({ t, n: i + 1 }))
                    .reverse()
                    .map(({ t, n }) => (
                      <option key={t.id} value={t.id} disabled={!t.startTree}>
                        {taskLabel(t, n)}
                        {!t.startTree ? " (snapshot pending)" : ""}
                      </option>
                    ))}
                </select>
                {file && (
                  <>
                    <span className="rv-path mono" title={file.path}>
                      <span className="file-dir">{splitPath(file.path).dir}</span>
                      <span className="file-base">{splitPath(file.path).base}</span>
                    </span>
                    {file.untracked && <span className="chip chip-new">new</span>}
                    {!file.binary && <DiffStat added={file.added} removed={file.removed} />}
                  </>
                )}
              </div>
              {taskId && (
                <p className="rv-task-prompt" title="The prompt of this task, as sent">
                  {tasks.find((t) => t.id === taskId)?.prompt || "(typed in the terminal)"}
                </p>
              )}
              {file ? (
                <Suspense fallback={<p className="hint pad">Loading editor…</p>}>
                  <ReviewDiff
                    agentId={agent.id}
                    file={file}
                    taskId={taskId}
                    sideBySide={sideBySide}
                    version={versionSig}
                    comments={agentComments.filter((c) => c.path === file.path)}
                    onAddComment={(line, text) => commentStore.add(agent.id, { path: file.path, line, text })}
                    reveal={reveal}
                  />
                </Suspense>
              ) : (
                <div className="rv-empty">{agentFiles?.length === 0 ? "Nothing to review here." : "Pick a file on the left."}</div>
              )}
              <footer className="rv-actions">
                <StatusGlyph status={agent.status} size="sm" />
                <span className="rv-actions-name">{agent.name}</span>
                {notice && (
                  <span className="muted-sm rv-notice" title={notice}>
                    {notice}
                  </span>
                )}
                <span className="spacer" />
                <button className="ghost-btn" disabled={!file} onClick={() => setDialog({ type: "discard" })} title="Restore this file from where the agent started">
                  Discard file
                </button>
                <button className="ghost-btn" disabled={agentComments.length === 0} onClick={() => setDialog({ type: "send" })}>
                  Send comments{agentComments.length > 0 && <span className="rv-ccount">{agentComments.length}</span>}
                </button>
                <button className="primary-btn" onClick={() => setDialog({ type: "commit" })}>
                  {agent.worktree ? "Commit & merge" : "Commit"}
                </button>
              </footer>
            </>
          ) : (
            <div className="rv-empty">{ordered.length ? "Nothing selected." : "Start an agent to review its changes."}</div>
          )}
        </section>
      </div>

      {agent && dialog?.type === "discard" && file && (
        <DiscardDialog agent={agent} file={file} scoped={!!taskId} onClose={() => setDialog(null)} onDone={done} />
      )}
      {agent && dialog?.type === "send" && (
        <PromptDialog
          agent={agent}
          title={`Send comments · ${agent.name}`}
          intro={<p className="muted">Edit freely — this exact text is what {agent.name} receives.</p>}
          initial={composePrompt(agentComments)}
          onClose={() => setDialog(null)}
          onSent={() => {
            commentStore.clear(agent.id);
            setNotice("Comments sent");
          }}
        />
      )}
      {agent && dialog?.type === "commit" && (
        <CommitMergeDialog
          agent={agent}
          onClose={() => setDialog(null)}
          onDone={done}
          onConflict={(result) => {
            setNonce((n) => n + 1);
            setDialog({ type: "conflict", result });
          }}
        />
      )}
      {agent && dialog?.type === "conflict" && <ConflictDialog agent={agent} result={dialog.result} onClose={() => setDialog(null)} />}
    </div>
  );
}
