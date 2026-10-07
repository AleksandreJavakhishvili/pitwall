import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { AgentView, FileChange } from "../../types";
import type { MergeResult, Task } from "../../reviewTypes";
import { api, errorText } from "../../api";
import { groupByProject } from "../../lib/groups";
import { usePersistentFlag } from "../../lib/usePersistentFlag";
import { StatusGlyph } from "../StatusGlyph";
import { DiffStat } from "../DiffStat";
import { Icon } from "../Icon";
import { Kbd } from "../Kbd";
import { FileIcon, StatusLetter } from "../FileIcon";
import { FileTree } from "./FileTree";
import { splitPath, treeOrder } from "../../lib/fileTree";
import { commentStore, composePrompt, sortComments, useComments } from "./comments";
import { agentTarget, CommitMergeDialog, ConflictDialog, DiscardDialog, PromptDialog, type CommitTarget } from "./ReviewDialogs";
import { WorktreeSection } from "./WorktreeSection";
import { forceRefreshWorktrees, refreshWorktrees, useWorktreeList } from "../../lib/useWorktrees";
import { useFresh } from "../../lib/freshness";
import { FreshError, RefreshControl } from "../Freshness";
import { otherWorktrees, refKey, worktreesByAgent, type WorktreeRef } from "../../lib/worktrees";
import { useActions } from "../../lib/actions";
import { releaseMonaco } from "./monacoLifecycle";
import "./review.css";

// Monaco is big: load it only when a file is opened.
const ReviewDiff = lazy(() => import("./ReviewDiff").then((m) => ({ default: m.ReviewDiff })));

type Dialog =
  | null
  | { type: "discard" }
  | { type: "send" }
  | { type: "commit" }
  | { type: "commitWorktree" }
  | { type: "conflict"; result: MergeResult };

interface Sel {
  agentId: string;
  path: string;
}

function clock(ms: number) {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function taskLabel(t: Task, n: number) {
  const prompt = t.prompt.trim().replace(/\s+/g, " ") || "(typed in the terminal)";
  const short = prompt.length > 70 ? prompt.slice(0, 69) + "…" : prompt;
  return `Task ${n} · ${clock(t.startedAt)}${t.endedAt ? "" : " · running"} · ${short}`;
}

/** Files per agent for its current scope; refetched when totals move, when `nonce` changes,
 * and every 5 s while the window is visible. */
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
    const t = setInterval(() => document.visibilityState !== "hidden" && load(), 5000);
    const onVisible = () => document.visibilityState !== "hidden" && load();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      alive = false;
      clearInterval(t);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [sig, nonce]); // eslint-disable-line react-hooks/exhaustive-deps

  return { files, errors };
}

/** What Review shows first when opened from a worktree. */
export interface ReviewFocus {
  projectId: string;
  path: string;
  nonce: number;
}

export default function Review({ agents, focus = null, onExit }: { agents: AgentView[]; focus?: ReviewFocus | null; onExit(): void }) {
  // Only agents whose changes can be read (not outside a git repository).
  const reviewable = useMemo(() => agents.filter((a) => a.caps.review), [agents]);
  const hidden = agents.length - reviewable.length;
  const groups = useMemo(() => groupByProject(reviewable), [reviewable]);
  const ordered = useMemo(() => groups.flatMap((g) => g.agents), [groups]);
  const { openRemoveWorktree, openTerminal } = useActions();

  // Worktrees (docs/spec/worktrees-view.md): each agent's own ones under it, the rest per project.
  const projects = useWorktreeList();
  const byAgent = useMemo(() => worktreesByAgent(projects), [projects]);
  const othersOf = useMemo(() => new Map(groups.map((g) => [g.key, otherWorktrees(projects, g.agents.map((a) => a.id))])), [groups, projects]);
  const wtRefs = useMemo(() => {
    const m = new Map<string, WorktreeRef>();
    for (const list of [...byAgent.values(), ...othersOf.values()]) for (const r of list) m.set(refKey(r), r);
    return m;
  }, [byAgent, othersOf]);
  /** The worktree the main pane shows (instead of an agent), and its file. */
  const [wtSel, setWtSel] = useState<{ key: string; path: string | null } | null>(null);
  const [wtOpen, setWtOpen] = useState<Record<string, boolean>>({});
  const [wtFiles, setWtFiles] = useState<Record<string, FileChange[] | null>>({});
  const wt = wtSel ? (wtRefs.get(wtSel.key) ?? null) : null;
  const wtList = wtSel ? wtFiles[wtSel.key] : undefined;
  const wtFile = wtSel?.path && wtList ? (wtList.find((f) => f.path === wtSel.path) ?? null) : null;
  const [scope, setScope] = useState<Record<string, string | null>>({});
  const [nonce, setNonce] = useState(0);
  const { files, errors } = useAgentFiles(ordered, scope, nonce);
  const [sel, setSel] = useState<Sel | null>(null);
  const [selAgentId, setSelAgentId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [closedDirs, setClosedDirs] = useState<Record<string, boolean>>({});
  const listRef = useRef<HTMLElement>(null);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [sideBySide, setSideBySide] = usePersistentFlag("pitwall.review.sideBySide", true);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [reveal, setReveal] = useState<{ line: number; nonce: number } | null>(null);
  const comments = useComments();

  // Freshness: opening Review reads every agent's changes now (bypassing the backend's
  // polling pace) and lists worktrees again; so do ↻ and ⌘⇧R. Picking an agent or a
  // worktree refreshes just that one. Each ends by reading the lists again (`nonce`).
  const latest = useRef(reviewable);
  latest.current = reviewable;
  const refreshAll = async (): Promise<number> => {
    const list = latest.current;
    const results = await Promise.allSettled([forceRefreshWorktrees(), ...list.map((a) => api.refreshChanges(a.id))]);
    setNonce((n) => n + 1);
    const failed = results
      .map((r, i) => (r.status === "rejected" ? `${i === 0 ? "worktrees" : list[i - 1].name}: ${errorText(r.reason)}` : null))
      .filter((x): x is string => x !== null);
    if (failed.length) throw new Error(failed.length > 1 ? `${failed[0]} (and ${failed.length - 1} more)` : failed[0]);
    return Date.now();
  };
  const fresh = useFresh<number>("review", () => ({ load: async () => Date.now(), force: refreshAll }));
  const refreshOne = (run: () => Promise<unknown>) =>
    void fresh.refresh(async () => {
      try {
        await run();
      } finally {
        setNonce((n) => n + 1);
      }
      return Date.now();
    });

  const agent = wtSel ? null : (ordered.find((a) => a.id === (sel?.agentId ?? selAgentId)) ?? null);
  const agentFiles = agent ? files[agent.id] : undefined;
  const file = sel && agentFiles ? (agentFiles.find((f) => f.path === sel.path) ?? null) : null;
  const taskId = agent ? (scope[agent.id] ?? null) : null;
  const agentComments = agent ? sortComments(comments[agent.id] ?? []) : [];

  // Default selection: the first agent with changes, its first file.
  useEffect(() => {
    if (sel || selAgentId || wtSel) return;
    const first = ordered.find((a) => (files[a.id]?.length ?? 0) > 0);
    if (first) setSel({ agentId: first.id, path: treeOrder(files[first.id])[0].path });
  }, [files, ordered, sel, selAgentId]);

  // The selected file went away (discarded, merged, scope changed): pick a neighbour.
  useEffect(() => {
    if (!sel || !agentFiles || file) return;
    const next = treeOrder(agentFiles)[0];
    setSel(next ? { agentId: sel.agentId, path: next.path } : null);
    setSelAgentId(sel.agentId);
  }, [sel, agentFiles, file]);

  // Opened from a worktree: show it.
  useEffect(() => {
    if (!focus) return;
    const key = refKey(focus);
    setWtSel({ key, path: null });
    setWtOpen((o) => ({ ...o, [key]: true }));
    setSel(null);
    setSelAgentId(null);
  }, [focus?.nonce]); // eslint-disable-line react-hooks/exhaustive-deps

  // A shown worktree: its first file once read; gone from the list (removed): nothing.
  useEffect(() => {
    if (!wtSel) return;
    if (projects.length && !wt) {
      setWtSel(null);
      return;
    }
    if (wtList && (!wtSel.path || !wtList.some((f) => f.path === wtSel.path))) {
      const first = treeOrder(wtList)[0]?.path ?? null;
      if (first !== wtSel.path) setWtSel({ key: wtSel.key, path: first });
    }
  }, [wtSel, wt, wtList, projects.length]);

  // Removed agent.
  useEffect(() => {
    if (sel && !agents.some((a) => a.id === sel.agentId)) setSel(null);
  }, [agents, sel]);

  // Tasks of the selected agent (for the scope picker).
  const agentId = agent?.id ?? null;

  // Picking an agent or a worktree: that one, now (joins a refresh already running).
  const wtProject = wt?.projectId ?? null;
  const picked = useRef<string | null>(null);
  useEffect(() => {
    const key = agentId ? `a:${agentId}` : wtSel && wtProject ? `w:${wtSel.key}` : null;
    if (key === picked.current) return;
    const first = picked.current === null;
    picked.current = key;
    // The first pick comes with opening Review, which already refreshes everything.
    if (!key || first) return;
    if (agentId) refreshOne(() => api.refreshChanges(agentId));
    else if (wtProject) refreshOne(() => forceRefreshWorktrees(wtProject));
  }, [agentId, wtSel?.key, wtProject]); // eslint-disable-line react-hooks/exhaustive-deps
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

  // Closing Review frees Monaco's editors, models and worker (after the diff
  // editor's own unmount has run).
  useEffect(() => () => void setTimeout(releaseMonaco, 0), []);

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

  // ↑/↓ move through the visible file rows (focus follows), Enter opens the focused one.
  const moveFocus = (delta: number) => {
    const rows = [...(listRef.current?.querySelectorAll<HTMLButtonElement>("[data-file-row]") ?? [])];
    if (!rows.length) return;
    const active = document.activeElement?.closest?.("[data-file-row]");
    let i = rows.findIndex((r) => r === active);
    if (i < 0) i = rows.findIndex((r) => r.getAttribute("aria-current") === "true");
    const next = rows[i < 0 ? 0 : Math.max(0, Math.min(rows.length - 1, i + delta))];
    next.focus();
    next.scrollIntoView({ block: "nearest" });
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.key !== "ArrowDown" && e.key !== "ArrowUp") || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
      if (document.querySelector(".backdrop")) return;
      const t = e.target as HTMLElement | null;
      const inList = !!t?.closest?.(".rv-list");
      // Elsewhere (editor, fields, other buttons) arrows keep their own meaning.
      if (!inList && t && t !== document.body) return;
      e.preventDefault();
      moveFocus(e.key === "ArrowDown" ? 1 : -1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const select = (agentId: string, path: string) => {
    setSel({ agentId, path });
    setSelAgentId(agentId);
    setWtSel(null);
  };
  const showWorktree = (key: string, path: string | null) => {
    setWtSel({ key, path });
    setSel(null);
    setSelAgentId(null);
  };
  const onWtFiles = useCallback(
    (key: string, f: FileChange[] | null) => setWtFiles((m) => (m[key] === f ? m : { ...m, [key]: f })),
    [],
  );
  const worktreeSection = (r: WorktreeRef) => {
    const key = refKey(r);
    return (
      <WorktreeSection
        key={key}
        r={r}
        open={!!wtOpen[key]}
        current={wtSel?.key === key}
        selected={wtSel?.key === key ? wtSel.path : null}
        nonce={nonce}
        onToggle={() => {
          // Expanding lists its project's worktrees again now.
          if (!wtOpen[key]) void forceRefreshWorktrees(r.projectId).catch(() => {});
          setWtOpen((o) => ({ ...o, [key]: !o[key] }));
        }}
        onShow={(first) => {
          showWorktree(key, first);
          setWtOpen((o) => ({ ...o, [key]: true }));
        }}
        onSelect={(path) => showWorktree(key, path)}
        isClosed={(dir) => !!closedDirs[`${key}\0${dir}`]}
        onToggleDir={(dir) => setClosedDirs((m) => ({ ...m, [`${key}\0${dir}`]: !m[`${key}\0${dir}`] }))}
        onFiles={onWtFiles}
      />
    );
  };
  const wtTarget = (r: WorktreeRef): CommitTarget => ({
    name: `worktree ${r.wt.name}`,
    where: r.wt.pathDisplay,
    projectDisplay: r.repoDisplay,
    status: () => api.getWorktreeMergeStatus(r.projectId, r.wt.path),
    commit: (m) => api.commitWorktree(r.projectId, r.wt.path, m),
    merge: () => api.mergeWorktree(r.projectId, r.wt.path),
  });

  const done = useCallback((msg: string) => {
    setNotice(msg);
    setNonce((n) => n + 1);
    refreshWorktrees();
  }, []);

  const versionSig = agent ? `${agent.added}:${agent.removed}:${agent.filesChanged}:${nonce}:${file?.added}:${file?.removed}` : "";

  return (
    <div className="review">
      <div className="review-bar">
        <span className="label">Review</span>
        <span className="muted-sm">what your agents changed · click a line number to comment</span>
        <span className="spacer" />
        <RefreshControl refreshing={fresh.refreshing} updatedAt={fresh.updatedAt} onRefresh={() => void fresh.refresh()} label="Refresh changes" />
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

      {fresh.error && (
        <div className="review-fresh-error">
          <FreshError error={fresh.error} prefix="Couldn't refresh: " onRetry={() => void fresh.refresh()} />
        </div>
      )}
      <div className="review-body">
        <aside className="rv-list" aria-label="Changes by agent" ref={listRef}>
          {ordered.length === 0 && <p className="hint pad">{hidden ? "No agents with changes to review." : "No agents yet."}</p>}
          {groups.map((g) => (
            <div key={g.key} className="rv-project">
              {g.agents.map((a) => {
                const list = files[a.id];
                const err = errors[a.id];
                const isOpen = !collapsed[a.id];
                const cs = sortComments(comments[a.id] ?? []);
                const scoped = !!scope[a.id];
                const added = list?.reduce((s, f) => s + f.added, 0) ?? 0;
                const removed = list?.reduce((s, f) => s + f.removed, 0) ?? 0;
                return (
                  <section key={a.id} className="rv-agent" data-current={!wtSel && agent?.id === a.id}>
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
                          const first = list && treeOrder(list)[0];
                          if (first) select(a.id, first.path);
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
                        {err && (
                          <div className="rv-pad">
                            <FreshError error={err} onRetry={() => refreshOne(() => api.refreshChanges(a.id))} />
                          </div>
                        )}
                        {!err && !list && <p className="hint rv-pad">Reading git…</p>}
                        {!err && list?.length === 0 && <p className="hint rv-pad">{scoped ? "No changes in this task." : "No changes."}</p>}
                        {list && list.length > 0 && (
                          <FileTree
                            files={list}
                            selected={sel?.agentId === a.id ? sel.path : null}
                            commentCounts={cs.reduce<Record<string, number>>((m, c) => ((m[c.path] = (m[c.path] ?? 0) + 1), m), {})}
                            isClosed={(dir) => !!closedDirs[`${a.id}\0${dir}`]}
                            onToggle={(dir) => setClosedDirs((m) => ({ ...m, [`${a.id}\0${dir}`]: !m[`${a.id}\0${dir}`] }))}
                            onSelect={(path) => select(a.id, path)}
                          />
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
                        {(byAgent.get(a.id)?.length ?? 0) > 0 && (
                          <div className="rv-wts" aria-label={`Worktrees of ${a.name}`}>
                            {byAgent.get(a.id)!.map(worktreeSection)}
                          </div>
                        )}
                      </>
                    )}
                  </section>
                );
              })}
              {(othersOf.get(g.key)?.length ?? 0) > 0 && (
                <div className="rv-others" aria-label={`Other worktrees of ${g.display}`}>
                  <div className="rv-others-head label">Other worktrees · {g.display}</div>
                  {othersOf.get(g.key)!.map(worktreeSection)}
                </div>
              )}
            </div>
          ))}
          {hidden > 0 && ordered.length > 0 && (
            <p className="hint pad">
              {hidden === 1 ? "1 agent works" : `${hidden} agents work`} outside a git repository — no changes to review.
            </p>
          )}
        </aside>

        <section className="rv-main">
          {wt ? (
            <>
              <div className="rv-head">
                <span className="rv-scope rv-scope-static" title={wt.wt.pathDisplay}>
                  Worktree <span className="mono">{wt.wt.name}</span> · {wt.wt.branch ?? "detached"} · against{" "}
                  {wt.target ?? "its HEAD"}
                </span>
                {wtFile && (
                  <>
                    <FileIcon path={wtFile.path} />
                    <span className="rv-path mono" title={wtFile.path}>
                      <span className="file-base">{splitPath(wtFile.path).base}</span>
                      <span className="file-dir"> {splitPath(wtFile.path).dir.replace(/\/$/, "")}</span>
                    </span>
                    <StatusLetter file={wtFile} />
                    {wtFile.binary ? <span className="chip chip-subtle">bin</span> : <DiffStat added={wtFile.added} removed={wtFile.removed} />}
                  </>
                )}
              </div>
              {wtFile ? (
                <Suspense fallback={<p className="hint pad">Loading editor…</p>}>
                  <ReviewDiff
                    sourceKey={wtSel!.key}
                    load={(path) => api.getWorktreeFileVersions(wt.projectId, wt.wt.path, path)}
                    file={wtFile}
                    taskId={null}
                    sideBySide={sideBySide}
                    version={`${wt.wt.head}:${nonce}:${wtFile.added}:${wtFile.removed}`}
                    comments={[]}
                    reveal={null}
                  />
                </Suspense>
              ) : (
                <div className="rv-empty">{wtList?.length === 0 ? "Nothing to review here." : wtList ? "Pick a file on the left." : "Reading git…"}</div>
              )}
              <footer className="rv-actions">
                <Icon name="branch" size={12} />
                <span className="rv-actions-name">{wt.wt.name}</span>
                {wt.wt.locked && <span className="chip chip-warn" title={wt.wt.lockReason ?? undefined}>locked</span>}
                {notice && (
                  <span className="muted-sm rv-notice" title={notice}>
                    {notice}
                  </span>
                )}
                <span className="spacer" />
                {wt.wt.caps.terminal && (
                  <button className="ghost-btn" onClick={() => openTerminal(wt.wt.path)} title="A shell in this worktree">
                    Open terminal
                  </button>
                )}
                {wt.wt.caps.remove && (
                  <button className="ghost-btn" onClick={() => openRemoveWorktree(wt.projectId, wt.wt.path)} title="git worktree remove (asks first)">
                    Remove worktree…
                  </button>
                )}
                {wt.wt.caps.commit && (
                  <button className="primary-btn" onClick={() => setDialog({ type: "commitWorktree" })}>
                    {wt.wt.caps.merge ? "Commit & merge" : "Commit"}
                  </button>
                )}
              </footer>
            </>
          ) : agent ? (
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
                  title="Everything since the agent started (committed or not), or one task (prompt). The Changes panel shows only uncommitted work, like git status."
                >
                  <option value="">Since the agent started · {agent.name}</option>
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
                    <FileIcon path={file.path} />
                    <span className="rv-path mono" title={file.path}>
                      <span className="file-base">{splitPath(file.path).base}</span>
                      <span className="file-dir"> {splitPath(file.path).dir.replace(/\/$/, "")}</span>
                    </span>
                    <StatusLetter file={file} />
                    {file.binary ? <span className="chip chip-subtle">bin</span> : <DiffStat added={file.added} removed={file.removed} />}
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
                    sourceKey={agent.id}
                    load={(path) => api.getFileVersions(agent.id, path, taskId)}
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
          target={agentTarget(agent)}
          onClose={() => setDialog(null)}
          onDone={done}
          onConflict={(result) => {
            setNonce((n) => n + 1);
            setDialog({ type: "conflict", result });
          }}
        />
      )}
      {wt && dialog?.type === "commitWorktree" && (
        <CommitMergeDialog
          target={wtTarget(wt)}
          onClose={() => setDialog(null)}
          onDone={done}
          onConflict={(result) => {
            setDialog(null);
            done(result.message);
          }}
        />
      )}
      {agent && dialog?.type === "conflict" && <ConflictDialog agent={agent} result={dialog.result} onClose={() => setDialog(null)} />}
    </div>
  );
}
