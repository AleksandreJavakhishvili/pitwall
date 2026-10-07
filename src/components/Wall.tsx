import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { AgentView } from "../types";
import type { ProjectGroup } from "../lib/groups";
import { machineHeading, summarize } from "../lib/groups";
import type { UiState } from "../state/workspace";
import { useActions } from "../lib/actions";
import { STATUS_WORD } from "../lib/status";
import { agentDragSource } from "../lib/dnd";
import { disposeWallViewers, mountWallView } from "../terminal/registry";
import { StatusGlyph } from "./StatusGlyph";
import { DiffStat } from "./DiffStat";
import { Icon } from "./Icon";
import { Kbd } from "./Kbd";

/** Never scale below this; wider terminals get cropped on the right instead. */
const MIN_SCALE = 0.42;

/**
 * The pit wall: every agent's live terminal, grouped by project. View-only —
 * it never resizes a PTY or sends input. Click a tile to go to that agent.
 */
export function Wall({ groups, ui, onExit }: { groups: ProjectGroup[]; ui: UiState; me: string; onExit(): void }) {
  const { toggleCollapsed } = useActions();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector(".backdrop")) {
        e.preventDefault();
        onExit();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onExit]);

  // Leaving the Wall drops the view-only instances and their channels.
  useEffect(() => () => disposeWallViewers(), []);

  return (
    <div className="wall">
      <div className="wall-bar">
        <span className="label">Wall</span>
        <span className="muted-sm">every agent, live · view only · click a tile to take over</span>
        <span className="spacer" />
        <button className="small-btn" onClick={onExit}>
          Back <Kbd>esc</Kbd>
        </button>
      </div>
      <div className="wall-scroll">
        {groups.map((g, gi) => {
          const collapsed = ui.wallCollapsed.includes(g.key);
          const heading = machineHeading(groups, gi);
          return (
            <section key={g.key} className="wall-section" data-blocked={g.blocked > 0}>
              {heading && <div className="machine-head">{heading}</div>}
              <button className="wall-section-head" onClick={() => toggleCollapsed(g.key, "wall")} aria-expanded={!collapsed}>
                <span className="chev" data-open={!collapsed}>
                  <Icon name="chevron" size={12} />
                </span>
                <span className="wall-project">{g.display}</span>
                {g.blocked > 0 && <span className="project-blocked">▲ {g.blocked}</span>}
                <span className="muted-sm">{collapsed ? summarize(g.agents) : `${g.agents.length} agent${g.agents.length === 1 ? "" : "s"}`}</span>
              </button>
              {!collapsed && (
                <div className="wall-grid">
                  {g.agents.map((a) => (
                    <WallTile key={a.id} agent={a} fontSize={ui.fontSize} />
                  ))}
                </div>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}

const WallTile = memo(WallTileImpl);

function WallTileImpl({ agent: a, fontSize }: { agent: AgentView; fontSize: number }) {
  const { showAgent } = useActions();
  const bodyRef = useRef<HTMLDivElement>(null);
  const scaleRef = useRef<HTMLDivElement>(null);
  const [hasView, setHasView] = useState(true);
  const cols = a.cols || 80;
  const rows = a.rows || 24;

  // Mount the terminal (own instance if sizes match, else a view-only one).
  useLayoutEffect(() => {
    const el = scaleRef.current;
    if (!el) return;
    const v = mountWallView(a.id, el, { cols, rows, running: a.running });
    setHasView(!!v);
    return () => v?.release();
  }, [a.id, cols, rows, a.running]);

  // Scale to the tile width; anchored bottom-left so the bottom rows stay visible.
  useEffect(() => {
    const body = bodyRef.current;
    const el = scaleRef.current;
    if (!body || !el) return;
    const apply = () => {
      const screen = el.querySelector<HTMLElement>(".xterm-screen");
      const natW = screen?.offsetWidth ?? 0;
      if (!natW) return;
      const s = Math.min(1, Math.max(MIN_SCALE, body.clientWidth / natW));
      el.style.transform = `scale(${s})`;
    };
    const ro = new ResizeObserver(apply);
    ro.observe(body);
    ro.observe(el);
    const t = window.setInterval(apply, 1000); // catches late renderer measurements
    apply();
    return () => {
      ro.disconnect();
      window.clearInterval(t);
    };
  }, [a.id, cols, rows, fontSize]);

  return (
    <div
      className="wall-tile"
      role="button"
      tabIndex={0}
      data-status={a.status}
      onClick={() => showAgent(a.id)}
      {...agentDragSource(a.id, a.name)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          showAgent(a.id);
        }
      }}
      title={`Go to ${a.name} · drag onto a space tab to move it there`}
    >
      <div className="wall-tile-head">
        <StatusGlyph status={a.status} />
        <span className="pane-name">{a.name}</span>
        <span className="status-word" data-status={a.status}>
          {STATUS_WORD[a.status]}
        </span>
        {a.status === "blocked" && a.statusDetail && <span className="pane-detail">{a.statusDetail}</span>}
        <span className="spacer" />
        <DiffStat added={a.added} removed={a.removed} />
      </div>
      <div className="wall-tile-body" ref={bodyRef}>
        <div className="wall-scale" ref={scaleRef} />
        {!hasView && <span className="wall-off">{a.status === "stopped" ? "stopped" : "not running"}</span>}
      </div>
    </div>
  );
}
