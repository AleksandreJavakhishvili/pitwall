import { useEffect, useLayoutEffect, useRef, useState, type DragEvent } from "react";
import type { AgentView } from "../types";
import { dropZone, type PaneNode, type Side } from "../layout/tree";
import { useActions } from "../lib/actions";
import { draggedAgent, endAgentDrag } from "../lib/dnd";
import { cellPerPx, mountTerminal, setTileFont, unmountTerminal } from "../terminal/registry";
import { tileFont } from "../layout/density";
import type { TileFonts } from "./LayoutView";
import { PaneHeader } from "./PaneHeader";
import { StoppedOverlay } from "./StoppedOverlay";
import { StatusGlyph } from "./StatusGlyph";

interface Props {
  pane: PaneNode;
  agent: AgentView | null;
  focused: boolean;
  maximized: boolean;
  canClose: boolean;
  members: AgentView[];
  fonts: TileFonts;
}

export function PaneView({ pane, agent, focused, maximized, canClose, members, fonts }: Props) {
  const { focusPane, dropAgent } = useActions();
  const [zone, setZone] = useState<Side | "center" | null>(null);

  const onDragOver = (e: DragEvent) => {
    const id = draggedAgent(e);
    if (!id) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    const r = e.currentTarget.getBoundingClientRect();
    const z = agent ? dropZone((e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height) : "center";
    setZone((prev) => (prev === z ? prev : z));
  };
  const onDrop = (e: DragEvent) => {
    const id = draggedAgent(e);
    const z = zone;
    setZone(null);
    endAgentDrag();
    if (!id || !z) return;
    e.preventDefault();
    if (id === agent?.id) return;
    dropAgent(id, z === "center" ? { paneId: pane.id } : { paneId: pane.id, side: z });
  };

  return (
    <section
      className="pane"
      data-focused={focused}
      data-status={agent?.status ?? "empty"}
      onMouseDownCapture={() => !focused && focusPane(pane.id)}
      onDragOver={onDragOver}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setZone(null);
      }}
      onDrop={onDrop}
    >
      {agent ? (
        <>
          <PaneHeader agent={agent} paneId={pane.id} maximized={maximized} canClose={canClose} />
          <div className="pane-body">
            <TerminalSlot
              agentId={agent.id}
              running={agent.running}
              base={fonts.base}
              density={fonts.density}
              override={fonts.overrides[agent.id]}
            />
            {!agent.running && <StoppedOverlay agent={agent} />}
          </div>
        </>
      ) : (
        <div className="pane-empty">
          <p className="label">Empty pane</p>
          <p className="hint">Drag an agent here from the sidebar{members.length ? ", or pick one:" : "."}</p>
          {members.length > 0 && (
            <div className="pane-pick">
              {members.slice(0, 8).map((m) => (
                <button key={m.id} className="chip-btn" onClick={() => dropAgent(m.id, { paneId: pane.id })}>
                  <StatusGlyph status={m.status} size="sm" /> {m.name}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      {zone && <div className="drop-indicator" data-zone={zone} />}
    </section>
  );
}

/** Slot space the terminal can't use (xterm's scrollbar gutter). */
const SLOT_CHROME = { w: 14, h: 0 };

/**
 * Hosts the agent's long-lived xterm; re-parents it in and parks it on unmount.
 * Picks the tile's font (auto-shrink to keep the density minimum, or the
 * user's ⌘+/− choice) on every resize; the refit sends the real cols/rows.
 */
function TerminalSlot({
  agentId,
  running,
  base,
  density,
  override,
}: {
  agentId: string;
  running: boolean;
  base: number;
  density: TileFonts["density"];
  override: number | undefined;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const fontRef = useRef({ base, density, override });
  fontRef.current = { base, density, override };

  const refit = () => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const f = fontRef.current;
    const box = r.width >= 20 && r.height >= 20 ? { w: r.width, h: r.height } : null;
    setTileFont(agentId, tileFont(box, f.base, f.density, f.override, SLOT_CHROME, cellPerPx(agentId) ?? undefined));
  };

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    mountTerminal(agentId, el, running);
    return () => unmountTerminal(agentId, el);
  }, [agentId]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let raf = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(refit);
    });
    ro.observe(el);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [agentId]);

  // Base font, density or the tile's own size changed.
  useEffect(() => {
    refit();
  }, [agentId, base, density, override]); // eslint-disable-line react-hooks/exhaustive-deps

  return <div className="term-slot" ref={ref} />;
}
