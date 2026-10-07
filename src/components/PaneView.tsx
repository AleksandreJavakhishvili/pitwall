import { memo, useEffect, useLayoutEffect, useRef } from "react";
import type { AgentView } from "../types";
import type { PaneNode } from "../layout/tree";
import { useActions } from "../lib/actions";
import { usePaneDropZone } from "../lib/dnd";
import { cellPerPx, mountTerminal, setTileFont, unmountTerminal } from "../terminal/registry";
import { foldMin, tileFont } from "../layout/density";
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
  spaceId: string;
}

/** Re-render only when something it shows changed (members: element-wise). */
function sameProps(a: Props, b: Props): boolean {
  return (
    a.pane === b.pane &&
    a.agent === b.agent &&
    a.focused === b.focused &&
    a.maximized === b.maximized &&
    a.canClose === b.canClose &&
    a.fonts === b.fonts &&
    a.spaceId === b.spaceId &&
    a.members.length === b.members.length &&
    a.members.every((m, i) => m === b.members[i])
  );
}

export const PaneView = memo(PaneViewImpl, sameProps);

function PaneViewImpl({ pane, agent, focused, maximized, canClose, members, fonts, spaceId }: Props) {
  const { focusPane, dropAgent } = useActions();
  // Live drop preview (lib/dnd hit-tests the data-drop attributes below).
  const zone = usePaneDropZone(pane.id);
  const ref = useRef<HTMLElement>(null);
  // A split that would leave a half below the density minimum lands as a chip instead (workspace dropOnPane).
  const noRoom = (() => {
    if (!zone || zone === "center" || !agent || !ref.current) return false;
    const r = ref.current.getBoundingClientRect();
    const min = foldMin(fonts.density, fonts.base);
    return zone === "left" || zone === "right" ? r.width / 2 < min.minW : r.height / 2 < min.minH;
  })();

  return (
    <section
      ref={ref}
      className="pane"
      data-focused={focused}
      data-status={agent?.status ?? "empty"}
      onMouseDownCapture={() => !focused && focusPane(pane.id)}
      data-drop="pane"
      data-space-id={spaceId}
      data-pane-id={pane.id}
      data-agent-id={agent?.id}
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
                <button key={m.id} className="chip-btn" onClick={() => dropAgent(m.id, { kind: "pane", spaceId, paneId: pane.id, zone: "center" })}>
                  <StatusGlyph status={m.status} size="sm" /> {m.name}
                </button>
              ))}
            </div>
          )}
        </div>
      )}
      {zone && (
        <div className="drop-indicator" data-zone={zone} data-empty={!agent} data-full={noRoom}>
          <span className="drop-label">
            {zone === "center" ? (agent ? "Swap" : "Show here") : noRoom ? "No room · adds as a chip" : `Split ${zone}`}
          </span>
        </div>
      )}
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
