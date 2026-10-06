import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, errorText, getApi } from "./api";
import type { AgentView, FileChange } from "./types";
import { groupByProject } from "./lib/groups";
import { useAgents } from "./lib/useAgents";
import { useToasts } from "./lib/useToasts";
import { ActionsContext, type Actions } from "./lib/actions";
import { useShortcuts } from "./lib/useShortcuts";
import { useBreakpoint } from "./lib/useBreakpoint";
import { usePersistentFlag } from "./lib/usePersistentFlag";
import { useUiState } from "./state/useUiState";
import { isPaneHidden } from "./state/hidden";
import * as W from "./state/workspace";
import { clampFont, stepTileFont } from "./layout/density";
import { autoRows, measureTiling, presetsFor } from "./lib/tiling";
import { setAgentDropHandler, type AgentDropTarget } from "./lib/dnd";
import { findPane, setSplitSizes } from "./layout/tree";
import {
  disposeTerminal,
  focusTerminal,
  hasTerminal,
  knownTerminals,
  fittedSize,
  reattachTerminal,
  setTerminalFontSize,
  tileFontOf,
} from "./terminal/registry";
import { TopBar } from "./components/TopBar";
import { Sidebar } from "./components/Sidebar";
import { SpaceView } from "./components/SpaceView";
import { RightPanel } from "./components/RightPanel";
import { AttentionBar } from "./components/AttentionBar";
import { Toasts } from "./components/Toasts";
import { DiffView } from "./components/DiffView";
import { NewAgentDialog } from "./components/NewAgentDialog";
import { CommandPalette } from "./components/CommandPalette";
import { SettingsDialog } from "./components/SettingsDialog";
import { RemoveDialog } from "./components/RemoveDialog";
import { EmptyState } from "./components/EmptyState";
import { Wall } from "./components/Wall";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Suspense, lazy } from "react";
import { Onboarding } from "./components/onboarding/Onboarding";
import { useProjects } from "./components/onboarding/useProjects";
import { withProjects } from "./components/onboarding/projects";

type ModalState =
  | null
  | { type: "new"; projectPath?: string }
  | { type: "scan" }
  | { type: "palette" }
  | { type: "settings" }
  | { type: "remove"; agentId: string }
  | { type: "diff"; agentId: string; file: FileChange };

// Review screen (Monaco) loads on first use.
const Review = lazy(() => import("./components/review/Review"));

const urlSpace = new URLSearchParams(window.location.search).get("space");
const nextFrame = (fn: () => void) => requestAnimationFrame(() => requestAnimationFrame(fn));

export default function App() {
  const { agents, loaded, isMock, label, patch } = useAgents();
  const { projects, onboarded, setOnboarded } = useProjects();
  const { ui, ready, update, flush, current } = useUiState(label);
  const resp = useBreakpoint();
  const me = label ?? W.MAIN;

  const [activeSpaceId, setActiveSpaceId] = useState<string>(urlSpace ?? W.ALL_SPACE);
  const [sidebarCollapsed, setSidebarCollapsed] = usePersistentFlag("pitwall.sidebarCollapsed", false);
  const [rightPinned, setRightPinned] = usePersistentFlag("pitwall.rightOpen", true);
  const [overlaySidebar, setOverlaySidebar] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [modal, setModal] = useState<ModalState>(null);
  const [reviewOn, setReviewOn] = useState(false);
  const { toasts, push, dismiss } = useToasts();

  const groups = useMemo(() => groupByProject(agents), [agents]);
  const ordered = useMemo(() => groups.flatMap((g) => g.agents), [groups]);
  const mySpaces = W.spacesOf(ui, me);
  const activeSpace = mySpaces.find((s) => s.id === activeSpaceId) ?? mySpaces[0] ?? null;
  const focusedAgentId = activeSpace
    ? (findPane(activeSpace.layout, activeSpace.focusedPaneId ?? "")?.agentId ?? null)
    : null;
  const selected = agents.find((a) => a.id === focusedAgentId) ?? null;
  const wallOn = ui.wall.includes(me);

  const agentsRef = useRef(agents);
  agentsRef.current = agents;
  const activeRef = useRef<string | null>(null);
  activeRef.current = activeSpace?.id ?? null;
  const focusedRef = useRef(focusedAgentId);
  focusedRef.current = focusedAgentId;

  // ── window bootstrapping ────────────────────────────────────────────────
  // A window opened with ?space=<id> claims that space.
  useEffect(() => {
    if (!ready || !urlSpace || !label) return;
    if (W.getSpace(current.current, urlSpace)) {
      update((s) => W.moveSpaceToWindow(s, urlSpace, label));
      setActiveSpaceId(urlSpace);
    }
  }, [ready, label, update, current]);

  // Main window: reclaim spaces of windows that closed (event) or no longer exist (startup).
  useEffect(() => {
    if (!ready || me !== W.MAIN) return;
    let off: (() => void) | null = null;
    let alive = true;
    getApi().then(async (a) => {
      const u = await a.onWindowClosed((e) => update((s) => W.reclaimWindow(s, e.label)));
      if (alive) off = u;
      else u();
    });
    const t = setTimeout(async () => {
      const labels = new Set(await api.listWindows().catch(() => [W.MAIN]));
      update((s) => {
        let next = s;
        for (const w of new Set([...Object.values(s.windowOf), ...s.wall])) {
          if (!labels.has(w)) next = W.reclaimWindow(next, w);
        }
        return next;
      });
    }, 1500);
    return () => {
      alive = false;
      off?.();
      clearTimeout(t);
    };
  }, [ready, me, update]);

  // Keep the shared blob free of removed agents.
  useEffect(() => {
    if (!ready || !loaded) return;
    update((s) => {
      const next = W.pruneAgents(s, new Set(agents.map((a) => a.id)));
      return JSON.stringify(next) === JSON.stringify(s) ? s : next;
    });
  }, [ready, loaded, agents, update]);

  // First run: put the first agent in the All space so the screen isn't empty.
  const hasAgents = ordered.length > 0;
  useEffect(() => {
    if (!ready || !loaded || !hasAgents || me !== W.MAIN) return;
    const s = current.current;
    if (!agentsRef.current.some((a) => W.locate(s, a.id))) {
      const first = groupByProject(agentsRef.current)[0]?.agents[0];
      if (first) update((x) => W.placeAgent(x, W.ALL_SPACE, first.id));
    }
  }, [ready, loaded, hasAgents, me, update, current]);

  // Terminal font size is global.
  useEffect(() => setTerminalFontSize(ui.fontSize), [ui.fontSize]);

  // Terminals: drop instances of removed agents or agents now shown in another window;
  // re-attach after a restart (not running → running).
  const prevRunning = useRef(new Map<string, boolean>());
  useEffect(() => {
    const ids = new Set(agents.map((a) => a.id));
    for (const id of knownTerminals()) {
      const loc = W.locate(ui, id);
      if (!ids.has(id) || (loc && loc.window !== me)) disposeTerminal(id);
    }
    for (const a of agents) {
      const was = prevRunning.current.get(a.id);
      if (was === false && a.running && hasTerminal(a.id)) reattachTerminal(a.id);
      prevRunning.current.set(a.id, a.running);
    }
  }, [agents, ui, me]);

  // ── actions ─────────────────────────────────────────────────────────────
  const run = useCallback(
    async <T,>(p: Promise<T>, what: string): Promise<T | undefined> => {
      try {
        return await p;
      } catch (e) {
        push({ tone: "error", title: `Couldn't ${what}`, detail: errorText(e) });
        return undefined;
      }
    },
    [push],
  );

  const focusLocal = useCallback(
    (spaceId: string, paneId: string, agentId: string) => {
      setActiveSpaceId(spaceId);
      update((s) => {
        const sp = W.getSpace(s, spaceId);
        let next = s;
        if (sp?.maximizedPaneId && sp.maximizedPaneId !== paneId) next = W.toggleMaximize(next, spaceId, sp.maximizedPaneId);
        if (isPaneHidden(spaceId, paneId) && sp?.focusedPaneId) {
          // Folded away for lack of room: swap it into the focused pane.
          return W.placeAgent(next, spaceId, agentId, { paneId: sp.focusedPaneId });
        }
        return W.focusPane(next, spaceId, paneId);
      });
      nextFrame(() => focusTerminal(agentId));
    },
    [update],
  );

  const showAgent = useCallback(
    (agentId: string) => {
      const s = current.current;
      if (s.wall.includes(me)) update((x) => W.setWall(x, me, false));
      setReviewOn(false);
      setOverlaySidebar(false);
      const loc = W.locate(s, agentId);
      if (loc && loc.window === me) {
        focusLocal(loc.space.id, loc.paneId, agentId);
      } else if (loc) {
        update((x) => ({ ...x, focusRequest: { agentId, window: loc.window, nonce: Date.now() } }));
        flush();
        api.focusWindow(loc.window).catch(() => {});
      } else {
        const spaceId = activeRef.current ?? W.spacesOf(s, me)[0]?.id;
        if (!spaceId) return;
        setActiveSpaceId(spaceId);
        update((x) => W.placeAgent(x, spaceId, agentId));
        nextFrame(() => focusTerminal(agentId));
      }
      if (agentsRef.current.find((x) => x.id === agentId)?.status === "done") api.markSeen(agentId).catch(() => {});
    },
    [me, update, flush, focusLocal, current],
  );

  // Another window asked us to focus an agent we hold.
  const lastNonce = useRef(0);
  useEffect(() => {
    const r = ui.focusRequest;
    if (!r || r.window !== me || r.nonce === lastNonce.current) return;
    lastNonce.current = r.nonce;
    const loc = W.locate(ui, r.agentId);
    if (loc && loc.window === me) focusLocal(loc.space.id, loc.paneId, r.agentId);
  }, [ui, me, focusLocal]);

  const nextBlocked = useCallback(() => {
    const blocked = groupByProject(agentsRef.current)
      .flatMap((g) => g.agents)
      .filter((a) => a.status === "blocked");
    if (!blocked.length) return;
    const i = blocked.findIndex((a) => a.id === focusedRef.current);
    showAgent(blocked[(i + 1) % blocked.length].id);
  }, [showAgent]);

  const restart = useCallback(
    async (id: string) => {
      // Start at the size it's shown at, so the replayed first frames fit.
      const a = await run(api.restartAgent(id, fittedSize(id)), "restart");
      if (a) patch(a); // the running transition re-attaches the terminal
    },
    [run, patch],
  );

  const withActive = (fn: (s: W.UiState, spaceId: string) => W.UiState) => {
    const id = activeRef.current;
    if (id) update((s) => fn(s, id));
  };

  const setWall = (on: boolean | null) => update((s) => W.setWall(s, me, on ?? !s.wall.includes(me)));
  const toggleReview = () => {
    setReviewOn((v) => !v);
    if (wallOn) setWall(false);
  };

  const moveSpaceToNewWindow = async (spaceId: string) => {
    if (spaceId === W.ALL_SPACE) {
      push({ tone: "info", title: "The All space stays in the main window" });
      return;
    }
    const newLabel = await run(api.openWindow(spaceId), "open a window");
    if (!newLabel) return;
    update((s) => W.moveSpaceToWindow(s, spaceId, newLabel));
    flush();
    if (isMock) push({ tone: "info", title: `Moved to window ${newLabel}`, detail: "The browser mock can't open real windows." });
  };

  // Drag & drop (lib/dnd): one rule set for every source and target (workspace dropOnPane / dropOnSpace).
  const dropAgentOn = (agentId: string, t: AgentDropTarget) => {
    const s = current.current;
    let spaceId: string;
    let r: W.DropResult;
    if (t.kind === "new-space") {
      const created = W.createCustomSpace(s, me);
      spaceId = created.spaceId;
      r = W.dropOnSpace(created.state, spaceId, agentId);
    } else {
      spaceId = t.spaceId;
      const fit = measureTiling(s, spaceId, resp.maxPerRow) ?? undefined;
      r =
        t.kind === "pane"
          ? W.dropOnPane(s, spaceId, agentId, t.paneId, t.zone, fit)
          : W.dropOnSpace(s, spaceId, agentId, fit);
    }
    if (r.state !== s) update(() => (t.kind === "pane" ? r.state : W.setWall(r.state, me, false)));
    if (t.kind !== "pane" && W.windowOfSpace(r.state, spaceId) === me) setActiveSpaceId(spaceId);
    if (r.outcome === "chip") {
      const name = agentsRef.current.find((a) => a.id === agentId)?.name ?? "Agent";
      push({
        tone: "info",
        title: `${name} added to the chip strip`,
        detail: "No room for another tile at this density. Click the chip to swap it in, or pick a denser layout.",
      });
    } else if (r.paneId && r.outcome !== "noop") {
      nextFrame(() => focusTerminal(agentId));
    }
  };
  const dropRef = useRef(dropAgentOn);
  dropRef.current = dropAgentOn;
  useEffect(() => setAgentDropHandler((agentId, t) => dropRef.current(agentId, t)), []);

  const actions: Actions = {
    showAgent,
    run,
    patch,
    restart,
    openDiff: (agentId, file) => setModal({ type: "diff", agentId, file }),
    openRemove: (agentId) => setModal({ type: "remove", agentId }),
    openNewAgent: (p) => setModal({ type: "new", projectPath: typeof p === "string" ? p : undefined }),
    dropAgent: (agentId, t) => dropAgentOn(agentId, t),
    focusPane: (paneId) => withActive((s, id) => W.focusPane(s, id, paneId)),
    closePane: (paneId) => withActive((s, id) => W.closePane(s, id, paneId)),
    toggleMaximize: (paneId) => withActive((s, id) => W.toggleMaximize(s, id, paneId)),
    setSplitSizes: (splitId, sizes) =>
      withActive((s, id) => {
        const sp = W.getSpace(s, id);
        return sp ? W.setLayout(s, id, setSplitSizes(sp.layout, splitId, sizes)) : s;
      }),
    applyPreset: (preset, ctx) =>
      withActive((s, id) =>
        W.applyPreset(s, id, preset, agentsRef.current, autoRows(ctx ?? measureTiling(s, id, resp.maxPerRow))),
      ),
    setDensity: (density, spaceId) => update((s) => W.setDensity(s, density, spaceId)),
    setBaseFont: (size) => update((s) => ({ ...s, fontSize: size === null ? W.DEFAULT_FONT : clampFont(size) })),
    layoutPrefs: { density: ui.density, fontSize: ui.fontSize },
    openProjectSpace: (project, display) => {
      const { state, spaceId } = W.openProjectSpace(current.current, project, display, agentsRef.current, me);
      update(() => W.setWall(state, me, false));
      const owner = W.windowOfSpace(state, spaceId);
      if (owner === me) setActiveSpaceId(spaceId);
      else api.focusWindow(owner).catch(() => {});
      setOverlaySidebar(false);
    },
    toggleCollapsed: (project, where) =>
      update((s) =>
        where === "sidebar"
          ? { ...s, collapsed: W.toggleIn(s.collapsed, project) }
          : { ...s, wallCollapsed: W.toggleIn(s.wallCollapsed, project) },
      ),
  };

  const toggleSidebar = () => {
    if (resp.sidebar === "full") setSidebarCollapsed((v) => !v);
    else setOverlaySidebar((v) => !v);
  };
  const toggleRight = () => {
    if (resp.rightDocked) setRightPinned((v) => !v);
    else setDrawerOpen((v) => !v);
  };
  // ⌘+ / ⌘− / ⌘0 act on the focused tile (its own size); the base size lives in Settings.
  const changeFont = (d: number | null) => {
    const id = wallOn ? null : focusedRef.current;
    const current = id ? tileFontOf(id) : undefined;
    if (id && current !== undefined) update((s) => ({ ...s, tileFont: stepTileFont(s.tileFont, id, current, d) }));
    else update((s) => ({ ...s, fontSize: d === null ? W.DEFAULT_FONT : clampFont(s.fontSize + d) }));
  };

  useShortcuts({
    palette: () => setModal((m) => (m?.type === "palette" ? null : { type: "palette" })),
    newAgent: () => setModal({ type: "new" }),
    nextBlocked,
    toggleSidebar,
    toggleRight,
    toggleWall: () => setWall(null),
    toggleReview,
    toggleMaximize: () => actions.toggleMaximize(),
    moveSpaceToWindow: () => {
      if (activeRef.current) moveSpaceToNewWindow(activeRef.current);
    },
    fontSize: changeFont,
    selectIndex: (i) => ordered[i] && showAgent(ordered[i].id),
    settings: () => setModal({ type: "settings" }),
  });

  // Native menu "Pitwall → Settings… ⌘," (sent to the focused window).
  useEffect(() => {
    let off: (() => void) | null = null;
    let alive = true;
    getApi().then(async (a) => {
      const u = await a.onOpenSettings(() => setModal({ type: "settings" }));
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, []);

  // If the agent you're looking at finishes while the window has focus, you've seen it.
  useEffect(() => {
    if (selected?.status !== "done" || !document.hasFocus() || wallOn) return;
    const id = selected.id;
    const t = setTimeout(() => document.hasFocus() && api.markSeen(id).catch(() => {}), 1500);
    return () => clearTimeout(t);
  }, [selected?.id, selected?.status, wallOn]);

  // Attention events → in-app toasts.
  useEffect(() => {
    let off: (() => void) | null = null;
    let alive = true;
    getApi().then(async (a) => {
      const u = await a.onAttention((e) => {
        if (e.agentId === focusedRef.current && e.reason === "done" && document.hasFocus()) return;
        push({
          tone: e.reason,
          title: e.reason === "blocked" ? `${e.name} needs you` : `${e.name} is done`,
          detail: e.detail,
          agentId: e.agentId,
        });
      });
      if (alive) off = u;
      else u();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [push]);

  // Drawers close when the window grows past their breakpoint.
  useEffect(() => {
    if (resp.rightDocked) setDrawerOpen(false);
    if (resp.sidebar === "full") setOverlaySidebar(false);
  }, [resp.rightDocked, resp.sidebar]);

  const closeModal = useCallback(() => setModal(null), []);
  const blocked = ordered.filter((a) => a.status === "blocked");
  const sidebarMode: "full" | "rail" | "hidden" =
    resp.sidebar === "full" ? (sidebarCollapsed ? "hidden" : "full") : resp.sidebar;
  const rightMode: "docked" | "drawer" | "hidden" =
    !selected || wallOn || reviewOn ? "hidden" : resp.rightDocked ? (rightPinned ? "docked" : "hidden") : drawerOpen ? "drawer" : "hidden";

  const sidebarGroups = useMemo(() => withProjects(groups, projects), [groups, projects]);
  const sidebarProps = { groups: sidebarGroups, ui, me, focusedAgentId };

  return (
    <ActionsContext.Provider value={actions}>
      <div
        className="app"
        data-bp={resp.bp}
        data-sidebar={sidebarMode}
        data-right={rightMode}
        data-attention={blocked.length ? "on" : "off"}
      >
        <TopBar
          agents={agents}
          isMock={isMock}
          spaces={mySpaces}
          activeSpaceId={activeSpace?.id ?? null}
          wallOn={wallOn}
          rightOpen={rightMode !== "hidden"}
          sidebarOpen={sidebarMode === "full" || overlaySidebar}
          compact={resp.bp === "xs" || resp.bp === "sm"}
          onSelectSpace={(id) => {
            setActiveSpaceId(id);
            if (wallOn) setWall(false);
          }}
          onNewSpace={() => {
            const { state, spaceId } = W.createCustomSpace(current.current, me);
            update(() => W.setWall(state, me, false));
            setActiveSpaceId(spaceId);
          }}
          onCloseSpace={(id) => update((s) => W.closeSpace(s, id))}
          onRenameSpace={(id, name) => update((s) => W.renameSpace(s, id, name))}
          onMoveSpace={moveSpaceToNewWindow}
          onToggleSidebar={toggleSidebar}
          onToggleRight={toggleRight}
          onToggleWall={() => setWall(null)}
          reviewOn={reviewOn && !wallOn}
          onToggleReview={toggleReview}
          onPalette={() => setModal({ type: "palette" })}
          onSettings={() => setModal({ type: "settings" })}
        />

        {sidebarMode !== "hidden" && <Sidebar mode={sidebarMode} {...sidebarProps} />}
        {overlaySidebar && sidebarMode !== "full" && (
          <>
            <div className="scrim" onClick={() => setOverlaySidebar(false)} />
            <Sidebar mode="overlay" {...sidebarProps} />
          </>
        )}

        <main className="center">
          {loaded && agents.length === 0 ? (
            <EmptyState />
          ) : wallOn ? (
            <ErrorBoundary key="wall" where="Wall" onClose={() => setWall(false)}>
              <Wall groups={groups} ui={ui} me={me} onExit={() => setWall(false)} />
            </ErrorBoundary>
          ) : reviewOn ? (
            <ErrorBoundary key="review" where="Review" onClose={() => setReviewOn(false)}>
              <Suspense fallback={<p className="hint pad">Loading review…</p>}>
                <Review agents={agents} onExit={() => setReviewOn(false)} />
              </Suspense>
            </ErrorBoundary>
          ) : activeSpace ? (
            <SpaceView
              key={activeSpace.id}
              space={activeSpace}
              agents={agents}
              ui={ui}
              me={me}
              maxPerRow={resp.maxPerRow}
              fontSize={ui.fontSize}
              onMoveToWindow={activeSpace.id === W.ALL_SPACE ? undefined : () => moveSpaceToNewWindow(activeSpace.id)}
            />
          ) : (
            <div className="empty">
              <h1 className="empty-title">No spaces in this window</h1>
              <button
                className="primary-btn"
                onClick={() => {
                  const { state, spaceId } = W.createCustomSpace(current.current, me);
                  update(() => state);
                  setActiveSpaceId(spaceId);
                }}
              >
                New space
              </button>
            </div>
          )}
        </main>

        {selected && rightMode === "docked" && <RightPanel agent={selected} />}
        {selected && rightMode === "drawer" && (
          <>
            <div className="scrim scrim-right" onClick={() => setDrawerOpen(false)} />
            <RightPanel agent={selected} drawer onClose={() => setDrawerOpen(false)} />
          </>
        )}

        {blocked.length > 0 && <AttentionBar blocked={blocked} onJump={nextBlocked} />}
      </div>

      <Toasts toasts={toasts} onDismiss={dismiss} onJump={showAgent} />

      {modal?.type === "new" && (
        <NewAgentDialog
          existing={agents}
          initialProject={modal.projectPath}
          onClose={closeModal}
          onCreated={(a: AgentView) => {
            patch(a);
            closeModal();
            setTimeout(() => showAgent(a.id), 0);
          }}
        />
      )}
      {modal?.type === "palette" && (
        <CommandPalette
          agents={ordered}
          selectedId={focusedAgentId}
          onClose={closeModal}
          commands={{
            newAgent: () => setModal({ type: "new" }),
            nextBlocked,
            toggleSidebar,
            toggleRight,
            toggleWall: () => setWall(null),
            toggleReview,
            moveToWindow: () => {
              if (activeRef.current) moveSpaceToNewWindow(activeRef.current);
            },
            preset: (p) => actions.applyPreset(p),
            presets: activeRef.current ? presetsFor(measureTiling(ui, activeRef.current, resp.maxPerRow)) : undefined,
            density: (d, scope) => actions.setDensity(d, scope === "space" ? (activeRef.current ?? undefined) : undefined),
            settings: () => setModal({ type: "settings" }),
          }}
        />
      )}
      {modal?.type === "settings" && (
        <ErrorBoundary where="Settings" variant="modal" onClose={closeModal}>
          <SettingsDialog onClose={closeModal} onScanAgain={() => setModal({ type: "scan" })} />
        </ErrorBoundary>
      )}
      {modal?.type === "remove" && (
        <RemoveDialog agent={agents.find((a) => a.id === modal.agentId) ?? null} onClose={closeModal} />
      )}
      {modal?.type === "diff" && (
        <DiffView agent={agents.find((a) => a.id === modal.agentId) ?? null} file={modal.file} onClose={closeModal} />
      )}
      {((onboarded === false && me === W.MAIN) || modal?.type === "scan") && (
        <ErrorBoundary
          where={modal?.type === "scan" ? "Scan again" : "onboarding"}
          variant="modal"
          onClose={() => {
            setOnboarded(true);
            if (modal?.type === "scan") closeModal();
          }}
        >
          <Onboarding
            mode={modal?.type === "scan" ? "rescan" : "welcome"}
            agents={agents}
            onClose={() => {
              setOnboarded(true);
              if (modal?.type === "scan") closeModal();
            }}
            onFinished={(created) => {
              setOnboarded(true);
              if (modal?.type === "scan") closeModal();
              if (!created.length) return;
              // Hand over: the new agents tiled in the All space (overflow as chips), first one focused.
              created.forEach(patch);
              setReviewOn(false);
              setOverlaySidebar(false);
              setActiveSpaceId(W.ALL_SPACE);
              const ids = created.map((a) => a.id);
              update((s) => W.setWall(W.tileAgents(s, W.ALL_SPACE, ids, Math.max(2, resp.maxPerRow * 2)), me, false));
              flush();
              nextFrame(() => focusTerminal(ids[0]));
            }}
          />
        </ErrorBoundary>
      )}
    </ActionsContext.Provider>
  );
}
