// First-launch welcome screen and Settings → "Scan again" (docs/spec/onboarding.md).
// The scan is read-only; nothing changes until the user ticks a box and confirms.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, errorText, getApi } from "../../api";
import type {
  AgentView,
  RunningElsewhere,
  ScannedMachine,
  ScannedSession,
  ScanProgress,
  ScanResult,
  ScanStep,
  ScannedConversation,
  ScannedProject,
} from "../../types";
import { relTime } from "../../lib/time";
import { useActions } from "../../lib/actions";
import { canPickFolder, pickFolder } from "../../lib/pickFolder";
import { Icon } from "../Icon";
import { handOverSize } from "../../terminal/registry";
import {
  convKey,
  defaultConversations,
  defaultSelection,
  planAgents,
  rememberedShowUnder,
  sessionKey,
  startAll,
  type PlannedAgent,
} from "./projects";
import { FolderAccessStep } from "./FolderAccess";
import "./onboarding.css";

const STEPS: { step: ScanStep; label: string }[] = [
  { step: "agents", label: "Agents on your PATH" },
  { step: "projects", label: "Projects" },
  { step: "conversations", label: "Conversations you can continue" },
  { step: "running", label: "Running now" },
  { step: "rules", label: "Rule files" },
  { step: "hooks", label: "Codex hooks" },
];

const SOURCE_LABEL: Record<string, string> = {
  claude: "Claude",
  codex: "Codex",
  vscode: "VS Code",
  cursor: "Cursor",
  folder: "Folder",
};

const FIRST_PROJECTS = 8;
const FIRST_CONVERSATIONS = 6;

type StepState = Partial<Record<ScanStep, Omit<ScanProgress, "step">>>;

interface Props {
  mode: "welcome" | "rescan";
  /** Existing agents (unique names; projects that already have one). */
  agents: AgentView[];
  /** Closed without starting anything (Skip / Cancel / Esc). */
  onClose(): void;
  /** Start finished: hand the agents that started over to the main screen. */
  onFinished(created: AgentView[]): void;
}

const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;
/** "Show under project…" option that opens the inline Add folder form. */
const ADD_FOLDER = "\u0000add";

/**
 * The welcome screen starts with "Folder access" (FolderAccess.tsx) and only
 * then scans: the scan reads project folders, and on macOS the first read of
 * Desktop/Documents/Downloads is what makes macOS ask. "Scan again" skips it.
 */
export function Onboarding(props: Props) {
  const [accessDone, setAccessDone] = useState(props.mode !== "welcome");
  const done = useCallback(() => setAccessDone(true), []);
  return accessDone ? <ScanScreen {...props} /> : <FolderAccessStep onDone={done} />;
}

function ScanScreen({ mode, agents, onClose, onFinished }: Props) {
  const { run } = useActions();
  const [steps, setSteps] = useState<StepState>({});
  const [result, setResult] = useState<ScanResult | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [allProjects, setAllProjects] = useState(false);
  const [allConvs, setAllConvs] = useState(false);
  // What "Start" creates: ticked conversations (by kind:session), running
  // sessions to bring over (by pid), fresh agents (project → kind).
  const [convSel, setConvSel] = useState<Set<string>>(new Set());
  const [runSel, setRunSel] = useState<Set<number>>(new Set());
  const [fresh, setFresh] = useState<Record<string, string>>({});
  // Sessions on other machines (agw) to add to Pitwall, by `sessionKey`.
  const [adoptSel, setAdoptSel] = useState<Set<string>>(new Set());
  // "Show under project…" for conversations started outside a project (convKey → project path).
  const [showUnder, setShowUnder] = useState<Record<string, string>>({});
  /** Row whose picker asked for "Add folder…" (the form opens under it). */
  const [pickFor, setPickFor] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [hooks, setHooks] = useState(false);
  const [showHookChanges, setShowHookChanges] = useState(false);
  const [adding, setAdding] = useState(false);
  const [addPath, setAddPath] = useState("");
  const [addError, setAddError] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  const [finishError, setFinishError] = useState<string | null>(null);
  const scan = useRef<Promise<ScanResult> | null>(null);
  const agentsRef = useRef(agents);
  agentsRef.current = agents;

  // Listen first, then scan once (StrictMode re-mounts reuse the same promise).
  useEffect(() => {
    let alive = true;
    let off: (() => void) | null = null;
    getApi().then(async (a) => {
      const u = await a.onScanProgress((e) => {
        if (alive) setSteps((s) => ({ ...s, [e.step]: { status: e.status, summary: e.summary } }));
      });
      if (!alive) return u();
      off = u;
      scan.current ??= api.scanEnvironment();
      scan.current.then(
        (r) => {
          if (!alive) return;
          const sel = defaultSelection(r.projects);
          const ticked = new Set([...sel, ...r.projects.filter((p) => p.added).map((p) => p.path)]);
          const installed = new Set(r.agents.filter((a) => a.installed).map((a) => a.kind));
          setResult(r);
          setSelected(sel);
          setConvSel(defaultConversations(r.conversations, ticked, { agents: agentsRef.current, installed }));
          setShowUnder(rememberedShowUnder(r.conversations, r.running));
        },
        (e) => alive && setScanError(errorText(e)),
      );
    });
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  // Esc closes "Scan again" (the welcome screen needs an explicit choice).
  useEffect(() => {
    if (mode !== "rescan") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !(e.target instanceof HTMLInputElement)) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mode, onClose]);

  const doneCount = STEPS.filter((s) => {
    const st = steps[s.step]?.status;
    return st && st !== "running";
  }).length;
  const scanning = !result && !scanError;

  const installed = useMemo(() => new Set(result?.agents.filter((a) => a.installed).map((a) => a.kind) ?? []), [result]);
  // The backend reports Codex hooks only when an installed agent uses them.
  const codexInstalled = !!result?.codexHooks;
  const hooksInstalled = result?.codexHooks?.installed ?? false;

  const projects = result?.projects ?? [];
  const shownProjects = allProjects ? projects : projects.slice(0, FIRST_PROJECTS);
  const conversations = result?.conversations ?? [];
  const projectConvs = conversations.filter((c) => !c.outsideProject);
  const outsideConvs = conversations.filter((c) => c.outsideProject);
  const shownConvs = allConvs
    ? projectConvs
    : projectConvs.filter((c, i) => i < FIRST_CONVERSATIONS || convSel.has(convKey(c)));
  const running = result?.running ?? [];
  const places = result?.places ?? [];
  const placeMachines = places.flatMap((p) => (p.machines ?? []).map((m) => ({ place: p, machine: m })));
  const adoptable = placeMachines.flatMap(({ machine }) => machine.sessions).filter((x) => !x.inPitwall);
  const adopts = adoptable.filter((x) => adoptSel.has(sessionKey(x)));
  const newCount = [...selected].filter((p) => !projects.find((x) => x.path === p)?.added).length;
  const kindChoices = result?.agents.filter((a) => a.installed) ?? [];
  // The backend lists kinds in its preferred order.
  const defaultKind = kindChoices[0]?.kind;

  const plan: PlannedAgent[] = useMemo(
    () =>
      planAgents({
        conversations,
        convSel,
        running,
        runSel,
        fresh,
        taken: agents.map((a) => a.name),
        showUnder,
      }),
    [conversations, convSel, running, runSel, fresh, agents, showUnder],
  );
  const startCount = plan.length + adopts.length;
  const toggleAdopt = (x: ScannedSession) =>
    setAdoptSel((sel) => {
      const next = new Set(sel);
      if (!next.delete(sessionKey(x))) next.add(sessionKey(x));
      return next;
    });
  /** Projects that already get an agent from a ticked conversation / running session. */
  const covered = useMemo(() => new Set(plan.filter((p) => p.sessionId).map((p) => p.displayProject ?? p.projectPath)), [plan]);

  const toggle = (path: string) => {
    const off = selected.has(path);
    setSelected((s) => {
      const next = new Set(s);
      if (off) next.delete(path);
      else next.add(path);
      return next;
    });
    if (off) {
      // Unticking a project drops what would have started in it.
      setConvSel(
        (s) =>
          new Set(
            [...s].filter((k) => {
              const c = conversations.find((c) => convKey(c) === k);
              return c?.projectPath !== path && !(c?.outsideProject && showUnder[k] === path);
            }),
          ),
      );
      setFresh(({ [path]: _, ...rest }) => rest);
    }
  };

  const toggleConv = (c: ScannedConversation) => {
    const key = convKey(c);
    const on = !convSel.has(key);
    setConvSel((s) => {
      const next = new Set(s);
      if (on) next.add(key);
      else next.delete(key);
      return next;
    });
    // Continuing a conversation makes its folder (or the project it's shown under) a project.
    if (on) selectProject(c.outsideProject ? showUnder[key] : c.projectPath);
  };

  const selectProject = (path: string | undefined) => {
    const p = path && projects.find((x) => x.path === path);
    if (p && !p.added) setSelected((s) => new Set(s).add(p.path));
  };

  /** "Show under project…": `ticked` rows also tick the chosen project. */
  const chooseProject = (key: string, value: string, ticked: boolean) => {
    if (value === ADD_FOLDER) {
      openAdd(key);
      return;
    }
    setShowUnder(({ [key]: _, ...rest }) => (value ? { ...rest, [key]: value } : rest));
    if (ticked) selectProject(value);
  };

  const toggleRun = (pid: number) => {
    const on = !runSel.has(pid);
    setRunSel((s) => {
      const next = new Set(s);
      if (on) next.add(pid);
      else next.delete(pid);
      return next;
    });
    const r = running.find((x) => x.pid === pid);
    if (on && r?.sessionId) selectProject(r.outsideProject ? showUnder[convKey({ kind: r.kind, sessionId: r.sessionId })] : (r.cwd ?? undefined));
  };

  const toggleFresh = (path: string) =>
    setFresh(({ [path]: had, ...rest }) => (had || !defaultKind ? rest : { ...rest, [path]: defaultKind }));

  /** Opens the inline Add folder form (typed-path fallback) and, in the app, the native picker. */
  const openAdd = (forKey: string | null) => {
    setPickFor(forKey);
    setAdding(true);
    setAddError(null);
    if (canPickFolder) browse(forKey);
  };

  const browse = async (forKey: string | null) => {
    const picked = await pickFolder();
    if (picked) addFolder(picked, forKey);
  };

  const addFolder = async (typed?: string, forKey: string | null = pickFor) => {
    const path = (typed ?? addPath).trim();
    if (!path) return;
    setAddError(null);
    try {
      const list = await api.addProject(path);
      const norm = path.replace(/\/+$/, "");
      const added =
        list.find((p) => p.path === norm || p.display === norm) ??
        list.reduce<(typeof list)[number] | undefined>((a, b) => (!a || b.addedAt > a.addedAt ? b : a), undefined);
      setResult((r) => {
        if (!r || !added) return r;
        const exists = r.projects.some((p) => p.path === added.path);
        const row: ScannedProject = {
          path: added.path,
          display: added.display,
          isGit: added.isGit,
          lastUsed: null,
          sources: [],
          agentHistory: false,
          added: true,
          rules: { rulesync: false, claudeMd: false, agentsMd: false },
        };
        return {
          ...r,
          projects: exists ? r.projects.map((p) => (p.path === added.path ? { ...p, added: true } : p)) : [row, ...r.projects],
        };
      });
      if (forKey && added) setShowUnder((m) => ({ ...m, [forKey]: added.path }));
      setAddPath("");
      setAdding(false);
      setPickFor(null);
    } catch (e) {
      setAddError(errorText(e));
    }
  };

  const startOne = async (p: PlannedAgent) => {
    // Start at the tile size it gets on hand-over, so its first frames fit.
    const size = await handOverSize(plan.indexOf(p), startCount, agents.length);
    return p.sessionId
      ? api.continueConversation({
          kind: p.kind,
          sessionId: p.sessionId,
          projectPath: p.projectPath,
          name: p.name,
          ...(p.displayProject ? { displayProject: p.displayProject } : {}),
          ...size,
        })
      : api.createAgent({ name: p.name, kind: p.kind, projectPath: p.projectPath, worktree: false, ...size });
  };

  const closeAdd = () => {
    setAdding(false);
    setPickFor(null);
    setAddError(null);
  };

  const addForm = (
    <form
      className="onb-add"
      onSubmit={(e) => {
        e.preventDefault();
        addFolder();
      }}
    >
      <input
        className="input mono"
        placeholder="/Users/you/code/project"
        value={addPath}
        onChange={(e) => setAddPath(e.target.value)}
        spellCheck={false}
        autoFocus
      />
      <button type="submit" className="small-btn" disabled={!addPath.trim()}>
        Add
      </button>
      {canPickFolder && (
        <button type="button" className="small-btn" onClick={() => browse(pickFor)}>
          Browse…
        </button>
      )}
      <button type="button" className="link-btn" onClick={closeAdd}>
        Cancel
      </button>
      {addError && <span className="field-error">{addError}</span>}
    </form>
  );

  /** "Show under project…" for a conversation / running session started outside a project. */
  const picker = (key: string, where: string, ticked: boolean) => {
    const value = showUnder[key] ?? "";
    const options = projects.map((p) => ({ path: p.path, display: p.display }));
    if (value && !options.some((o) => o.path === value)) options.unshift({ path: value, display: value });
    return (
      <>
        <div className="onb-fresh onb-under" data-on={!!value}>
          <span className="muted-sm">Show under</span>
          <select
            className="input onb-fresh-kind onb-under-pick"
            aria-label="Show under project"
            value={value}
            onChange={(e) => chooseProject(key, e.target.value, ticked)}
          >
            <option value="">{where} (where it started)</option>
            {options.map((o) => (
              <option key={o.path} value={o.path}>
                {o.display}
              </option>
            ))}
            <option value={ADD_FOLDER}>Add folder…</option>
          </select>
          {value && <span className="muted-sm">runs in {where}</span>}
        </div>
        {adding && pickFor === key && <div className="onb-under-add">{addForm}</div>}
      </>
    );
  };

  const convRow = (c: ScannedConversation) => {
    const key = convKey(c);
    const canRun = installed.has(c.kind);
    const checked = !c.inPitwall && convSel.has(key);
    return (
      <li key={key}>
        <label
          className="onb-row onb-pick"
          data-checked={checked}
          data-disabled={c.inPitwall || !canRun}
          title={canRun ? undefined : `${c.kindName} isn't installed`}
        >
          <input type="checkbox" checked={checked} disabled={c.inPitwall || !canRun} onChange={() => toggleConv(c)} />
          <span className="chip chip-subtle onb-kind">{c.kindName}</span>
          <span className="onb-row-main">
            <span className="onb-conv-title" title={c.title}>
              {c.title}
            </span>
            <span className="muted-sm mono onb-conv-where">
              {c.projectDisplay} · {relTime(c.lastUsed)}
            </span>
          </span>
          {c.inPitwall ? (
            <span className="chip chip-ok">in Pitwall</span>
          ) : (
            c.runningElsewhere && <span className="chip chip-warn">open elsewhere</span>
          )}
        </label>
        {c.runningElsewhere && !c.inPitwall && checked && (
          <p className="hint onb-note onb-row-note" data-warn="true">
            This conversation is open in another terminal — close it first so two copies don't run.
          </p>
        )}
        {c.outsideProject && !c.inPitwall && canRun && picker(key, c.projectDisplay, checked)}
      </li>
    );
  };

  /** Rows a bulk "All" may tick: runnable, not already in Pitwall, and not open
   * in another terminal (those need a deliberate, single tick). */
  const bulkTickable = (c: ScannedConversation) => !c.inPitwall && !c.runningElsewhere && installed.has(c.kind);
  const setGroup = (group: ScannedConversation[], on: boolean) =>
    setConvSel((s) => {
      const next = new Set(s);
      for (const c of group) {
        if (!on) next.delete(convKey(c));
        else if (bulkTickable(c)) next.add(convKey(c));
      }
      return next;
    });

  const runKey = (r: RunningElsewhere) => (r.sessionId ? convKey({ kind: r.kind, sessionId: r.sessionId }) : null);
  const outsideTitle = outsideConvs.every((c) => c.projectDisplay === "~") ? "Started in ~" : "Started outside a project";

  const finish = async () => {
    setFinishing(true);
    setFinishError(null);
    const paths = [...selected].filter((p) => !projects.find((x) => x.path === p)?.added);
    const todo = plan;
    try {
      await api.completeOnboarding(paths, hooks && codexInstalled && !hooksInstalled);
    } catch (e) {
      const msg = errorText(e);
      if (!msg.startsWith("Projects were saved")) {
        setFinishError(msg);
        setFinishing(false);
        return;
      }
      // Projects are in; only the hooks step failed. Say so and move on.
      await run(Promise.reject(msg), "install Codex hooks");
    }
    // One by one; a failure is a toast and the rest still start.
    const total = todo.length + adopts.length;
    const created = await startAll(todo, startOne, {
      onProgress: (done) => setProgress({ done, total }),
      onError: (p, e) => void run(Promise.reject(e), `start ${p.name}`),
    });
    // Sessions on other machines: tracked and attached, nothing changes there.
    const added = await startAll(
      adopts,
      async (x) => {
        const size = await handOverSize(todo.length + adopts.indexOf(x), total, agents.length);
        return api.adoptSession({ provider: x.provider, machine: x.machine, native: x.native, ...size });
      },
      {
        onProgress: (done) => setProgress({ done: todo.length + done, total }),
        onError: (x, e) => void run(Promise.reject(e), `add ${x.name}`),
      },
    );
    onFinished([...created, ...added]);
  };

  const skip = async () => {
    if (mode === "rescan") return onClose();
    setFinishing(true);
    await run(api.completeOnboarding([], false), "finish setup");
    onClose();
  };

  return (
    <div className="onb" data-mode={mode} role="dialog" aria-modal="true" aria-label={mode === "welcome" ? "Welcome to Pitwall" : "Scan again"}>
      <div className="onb-inner">
        <header className="onb-head">
          <div className="onb-board" aria-hidden>
            <span>PIT</span>
            <span>{scanning ? `${doneCount}/${STEPS.length}` : "—"}</span>
            <span data-live={scanning}>{scanning ? "SCAN" : "BOX"}</span>
          </div>
          {mode === "welcome" ? (
            <>
              <h1 className="onb-title">Welcome to Pitwall</h1>
              <p className="muted onb-lede">
                Pitwall is looking around for your coding agents and projects. It only reads — nothing on your Mac changes
                unless you tick a box below.
              </p>
            </>
          ) : (
            <>
              <h1 className="onb-title">Scan again</h1>
              <p className="muted onb-lede">Read-only. Pick projects to add and conversations to continue.</p>
            </>
          )}
          {mode === "rescan" && (
            <button className="icon-btn onb-close" onClick={onClose} aria-label="Close" title="Close (esc)">
              <Icon name="x" />
            </button>
          )}
        </header>

        <div className="onb-grid">
          <aside className="onb-checklist" aria-live="polite">
            <div className="onb-lap" aria-hidden>
              <span style={{ width: `${(doneCount / STEPS.length) * 100}%` }} />
            </div>
            <ol>
              {STEPS.map(({ step, label }) => {
                const st = steps[step];
                const status = st?.status ?? (result ? "done" : "pending");
                return (
                  <li key={step} className="onb-step" data-status={status}>
                    <span className="onb-glyph" aria-hidden>
                      {status === "done" ? "✓" : status === "error" ? "✕" : status === "skipped" ? "–" : ""}
                    </span>
                    <span className="onb-step-text">
                      <span className="onb-step-label">{label}</span>
                      {st?.summary && <span className="onb-step-summary mono">{st.summary}</span>}
                      {status === "running" && <span className="onb-step-summary">looking…</span>}
                    </span>
                  </li>
                );
              })}
            </ol>
            {scanError && <p className="form-error">Scan failed: {scanError}</p>}
          </aside>

          <section className="onb-results">
            {!result && !scanError && (
              <div className="onb-waiting">
                <p className="muted-sm">Results appear here as soon as the scan finishes.</p>
              </div>
            )}

            {result && (
              <>
                <div className="onb-section">
                  <div className="onb-section-head">
                    <h2 className="label">Projects</h2>
                    <span className="label-count">{projects.length}</span>
                    <span className="spacer" />
                    <button className="link-btn" onClick={() => setSelected(new Set(projects.filter((p) => !p.added).map((p) => p.path)))}>
                      All
                    </button>
                    <button className="link-btn" onClick={() => setSelected(new Set())}>
                      None
                    </button>
                  </div>
                  <p className="hint">Projects appear in the sidebar even before they have agents. Recent first.</p>
                  {projects.length === 0 && <p className="muted-sm">No projects found. Add a folder below.</p>}
                  <ul className="onb-list">
                    {shownProjects.map((p) => {
                      const checked = p.added || selected.has(p.path);
                      const showFresh = checked && !covered.has(p.path) && !!defaultKind;
                      return (
                        <li key={p.path}>
                          <label className="onb-row onb-project" data-checked={checked} data-added={p.added}>
                            <input type="checkbox" checked={checked} disabled={p.added} onChange={() => toggle(p.path)} />
                            <span className="onb-row-main">
                              <span className="onb-path mono" title={p.path}>
                                {p.display}
                              </span>
                              <span className="onb-chips">
                                {p.added && <span className="chip chip-ok">in Pitwall</span>}
                                {p.sources.map((s) => (
                                  <span key={s} className="chip">
                                    {SOURCE_LABEL[s] ?? s}
                                  </span>
                                ))}
                                {!p.isGit && <span className="chip chip-subtle">no git</span>}
                                {p.rules.rulesync && <span className="chip chip-subtle">rulesync</span>}
                                {p.rules.claudeMd && <span className="chip chip-subtle">CLAUDE.md</span>}
                                {p.rules.agentsMd && <span className="chip chip-subtle">AGENTS.md</span>}
                              </span>
                            </span>
                            <span className="onb-when muted-sm">{p.lastUsed ? relTime(p.lastUsed) : "recently opened"}</span>
                          </label>
                          {showFresh && (
                            <div className="onb-fresh" data-on={p.path in fresh}>
                              <label className="check">
                                <input type="checkbox" checked={p.path in fresh} onChange={() => toggleFresh(p.path)} />
                                <span>Start a new agent</span>
                              </label>
                              {p.path in fresh && (
                                <select
                                  className="input onb-fresh-kind"
                                  aria-label={`Agent for ${p.display}`}
                                  value={fresh[p.path]}
                                  onChange={(e) => setFresh((f) => ({ ...f, [p.path]: e.target.value }))}
                                >
                                  {kindChoices.map((k) => (
                                    <option key={k.kind} value={k.kind}>
                                      {k.name}
                                    </option>
                                  ))}
                                </select>
                              )}
                            </div>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                  <div className="row gap onb-list-foot">
                    {projects.length > FIRST_PROJECTS && (
                      <button className="link-btn" onClick={() => setAllProjects((v) => !v)}>
                        {allProjects ? "Show fewer" : `Show all ${projects.length}`}
                      </button>
                    )}
                    <span className="spacer" />
                    {!(adding && !pickFor) && (
                      <button
                        className="small-btn"
                        onClick={() => openAdd(null)}
                      >
                        <Icon name="folder" size={14} /> Add folder…
                      </button>
                    )}
                  </div>
                  {adding && !pickFor && addForm}
                </div>

                {projectConvs.length > 0 && (
                  <div className="onb-section">
                    <div className="onb-section-head">
                      <h2 className="label">Conversations to continue</h2>
                      <span className="label-count">{projectConvs.length}</span>
                      <span className="spacer" />
                      {projectConvs.some((c) => convSel.has(convKey(c))) && (
                        <button className="link-btn" onClick={() => setGroup(projectConvs, false)}>
                          None
                        </button>
                      )}
                    </div>
                    <p className="hint">
                      Each ticked conversation starts an agent in its folder and resumes that session. The latest one of
                      each project from the last 3 days is ticked for you.
                    </p>
                    <ul className="onb-list">{shownConvs.map(convRow)}</ul>
                    {projectConvs.length > FIRST_CONVERSATIONS && (
                      <button className="link-btn onb-list-foot" onClick={() => setAllConvs((v) => !v)}>
                        {allConvs ? "Show fewer" : `Show all ${projectConvs.length}`}
                      </button>
                    )}
                  </div>
                )}

                {outsideConvs.length > 0 && (
                  <div className="onb-section">
                    <div className="onb-section-head">
                      <h2 className="label">{outsideTitle}</h2>
                      <span className="label-count">{outsideConvs.length}</span>
                      <span className="spacer" />
                      <button
                        className="link-btn"
                        onClick={() => setGroup(outsideConvs, true)}
                        title="Tick every conversation that isn't open in another terminal"
                      >
                        All
                      </button>
                      <button className="link-btn" onClick={() => setGroup(outsideConvs, false)}>
                        None
                      </button>
                    </div>
                    <p className="hint">
                      Conversations started outside a project folder. Pick a project to show one under — it still runs
                      in the folder it started in, because that's the only place it can be resumed.
                    </p>
                    <ul className="onb-list">{outsideConvs.map(convRow)}</ul>
                  </div>
                )}

                {(running.length > 0 || places.length > 0) && (
                  <div className="onb-section">
                    <div className="onb-section-head">
                      <h2 className="label">Running now</h2>
                      <span className="label-count">{running.length + placeMachines.reduce((n, { machine }) => n + machine.sessions.length, 0)}</span>
                    </div>
                    {running.length > 0 && (
                      <>
                        <div className="onb-machine">
                          <span className="onb-machine-name">This Mac</span>
                          <span className="muted-sm">running outside Pitwall</span>
                        </div>
                        <ul className="onb-list">
                          {running.map((r) => {
                            const can = !!r.sessionId && !!r.cwd && installed.has(r.kind) && !r.inPitwall;
                            const checked = can && runSel.has(r.pid);
                            const key = runKey(r);
                            return (
                              <li key={r.pid}>
                                <label className="onb-row onb-pick" data-checked={checked} data-disabled={!can}>
                                  <input type="checkbox" checked={checked} disabled={!can} onChange={() => toggleRun(r.pid)} />
                                  <span className="chip chip-subtle onb-kind">{r.kindName}</span>
                                  <span className="onb-row-main">
                                    <span className="mono onb-conv-title">{r.cwdDisplay ?? "unknown folder"}</span>
                                    <span className="muted-sm onb-conv-where" title={r.title ?? undefined}>
                                      {r.title ?? (r.sessionId ? `session ${r.sessionId.slice(0, 8)}` : "no conversation found")} ·
                                      pid {r.pid}
                                    </span>
                                  </span>
                                  {r.inPitwall ? (
                                    <span className="chip chip-ok">in Pitwall</span>
                                  ) : (
                                    <span className="muted-sm onb-when">{can ? "Bring into Pitwall" : "can't bring over"}</span>
                                  )}
                                </label>
                                {r.outsideProject && can && key && picker(key, r.cwdDisplay ?? "~", checked)}
                              </li>
                            );
                          })}
                          <li className="hint onb-note" data-warn={runSel.size > 0}>
                            Pitwall resumes the same conversation in its own terminal — close the old terminal first so two
                            copies don't run.
                          </li>
                        </ul>
                      </>
                    )}
                    {placeMachines.map(({ place, machine }) => (
                      <MachineGroup
                        key={`${place.provider}:${machine.id}`}
                        place={place.label}
                        machine={machine}
                        selected={adoptSel}
                        onToggle={toggleAdopt}
                      />
                    ))}
                    {places
                      .filter((p) => p.machines === null)
                      .map((p) => (
                        <p key={p.provider} className="hint">
                          {[p.label, p.version].filter(Boolean).join(" ")} is installed, but its machines and sessions couldn't
                          be listed.
                        </p>
                      ))}
                    {adoptable.length > 0 && (
                      <p className="hint">
                        Adding a session only starts tracking it: Pitwall attaches to its terminal and reads its status from the
                        screen. Nothing changes on the machine.
                      </p>
                    )}
                  </div>
                )}

                {codexInstalled && (
                  <div className="onb-section">
                    <div className="onb-section-head">
                      <h2 className="label">Codex status</h2>
                    </div>
                    {hooksInstalled ? (
                      <p className="muted-sm">
                        <span className="chip chip-ok">hooks installed</span> Codex status is exact.
                      </p>
                    ) : (
                      <>
                        <label className="check">
                          <input type="checkbox" checked={hooks} onChange={(e) => setHooks(e.target.checked)} />
                          <span>
                            Exact Codex status (install hooks)
                            <span className="hint block">
                              Without hooks Pitwall reads Codex's screen to tell working, blocked and done apart.{" "}
                              <button type="button" className="link-btn" onClick={() => setShowHookChanges((v) => !v)} aria-expanded={showHookChanges}>
                                {showHookChanges ? "Hide changes" : "Show changes"}
                              </button>
                            </span>
                          </span>
                        </label>
                        {showHookChanges && (
                          <div className="confirm-box">
                            <p>
                              This edits <span className="mono">{result.codexHooks?.path ?? "~/.codex/hooks.json"}</span>:
                            </p>
                            <ul>
                              <li>
                                A backup of the current file is made first (<span className="mono">hooks.json.pitwall-backup</span>).
                              </li>
                              <li>Pitwall entries are appended; your existing hooks stay.</li>
                              <li>Outside Pitwall the hook does nothing and exits silently.</li>
                            </ul>
                          </div>
                        )}
                      </>
                    )}
                  </div>
                )}
              </>
            )}
          </section>
        </div>

        <footer className="onb-foot">
          {finishError && <span className="form-error">{finishError}</span>}
          {!finishError && progress && (
            <span className="muted-sm onb-progress" aria-live="polite">
              Starting {plural(progress.total, "agent")}… {progress.done + 1}/{progress.total}
            </span>
          )}
          {!finishError && !finishing && result && startCount === 0 && (
            <span className="muted-sm">No agents will start — tick a conversation or “Start a new agent”.</span>
          )}
          <span className="spacer" />
          <button className="ghost-btn" onClick={skip} disabled={finishing}>
            {mode === "welcome" ? "Skip" : "Cancel"}
          </button>
          <button className="primary-btn primary-lg" onClick={finish} disabled={finishing || (!result && !scanError)} data-autofocus>
            {finishing
              ? progress
                ? "Starting…"
                : "Saving…"
              : mode === "welcome"
                ? startCount > 0
                  ? `Start Pitwall · ${plural(startCount, "agent")} →`
                  : "Start Pitwall →"
                : [newCount > 0 && `Add ${plural(newCount, "project")}`, startCount > 0 && `start ${plural(startCount, "agent")}`]
                    .filter(Boolean)
                    .join(" · ")
                    .replace(/^s/, "S") || "Done"}
          </button>
        </footer>
      </div>
    </div>
  );
}

const SESSION_STATUS: Record<ScannedSession["status"], string> = { running: "running", stopped: "stopped", unknown: "status unknown" };

/** One machine of another place (an agw VM) and its sessions, each with "Add to Pitwall". */
function MachineGroup({
  place,
  machine: m,
  selected,
  onToggle,
}: {
  place: string;
  machine: ScannedMachine;
  selected: Set<string>;
  onToggle(x: ScannedSession): void;
}) {
  return (
    <>
      <div className="onb-machine">
        <span className="onb-machine-name mono">{m.label}</span>
        <span className="muted-sm">{[place, m.detail].filter(Boolean).join(" · ")}</span>
      </div>
      <ul className="onb-list">
        {m.sessions.length === 0 && <li className="onb-row muted-sm">No sessions</li>}
        {m.sessions.map((x) => {
          const checked = !x.inPitwall && selected.has(sessionKey(x));
          return (
            <li key={x.native}>
              <label className="onb-row onb-pick" data-checked={checked} data-disabled={x.inPitwall}>
                <input type="checkbox" checked={checked} disabled={x.inPitwall} onChange={() => onToggle(x)} />
                <span className="chip chip-subtle onb-kind">{x.kindName}</span>
                <span className="onb-row-main">
                  <span className="mono onb-conv-title">{x.name}</span>
                  <span className="muted-sm onb-conv-where">
                    {[x.workspace && `workspace ${x.workspace}`, x.user ? `as ${x.user}` : null].filter(Boolean).join(" · ")}
                  </span>
                </span>
                <span className={`chip ${x.status === "running" ? "chip-ok" : "chip-subtle"}`}>{SESSION_STATUS[x.status]}</span>
                {x.inPitwall ? (
                  <span className="chip chip-ok">in Pitwall</span>
                ) : (
                  <span className="muted-sm onb-when">Add to Pitwall</span>
                )}
              </label>
            </li>
          );
        })}
      </ul>
    </>
  );
}
