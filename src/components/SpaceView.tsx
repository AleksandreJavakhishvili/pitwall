import { useEffect, useMemo, useRef, useState } from "react";
import type { AgentView } from "../types";
import {
  agentsIn,
  availablePresets,
  buildPreset,
  findPane,
  fitLayout,
  paneRects,
  PRESET_LABEL,
  type LayoutNode,
  type Preset,
} from "../layout/tree";
import { DENSITIES, DENSITY_CELLS, DENSITY_LABEL, foldMin, isDensity } from "../layout/density";
import { densityOf, spaceMembers, type Space, type UiState } from "../state/workspace";
import { setHiddenPanes } from "../state/hidden";
import { useActions } from "../lib/actions";
import { LayoutView } from "./LayoutView";
import { ChipStrip } from "./ChipStrip";
import { Icon } from "./Icon";

interface Props {
  space: Space;
  agents: AgentView[];
  ui: UiState;
  me: string;
  maxPerRow: number;
  fontSize: number;
  onMoveToWindow?: () => void;
}


export function SpaceView({ space, agents, ui, me, maxPerRow, fontSize, onMoveToWindow }: Props) {
  const { applyPreset, toggleMaximize, setDensity } = useActions();
  const density = densityOf(ui, space);
  const min = foldMin(density, fontSize);
  const areaRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);

  useEffect(() => {
    const el = areaRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize((s) => (s && Math.abs(s.w - width) < 1 && Math.abs(s.h - height) < 1 ? s : { w: width, h: height }));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const maximized = space.maximizedPaneId ? findPane(space.layout, space.maximizedPaneId) : null;
  const base: LayoutNode = maximized ?? space.layout;
  const { tree, hidden } = useMemo(() => {
    if (!size) return { tree: base, hidden: [] };
    // Density, not the breakpoint, limits how many panes fit (maxPerRow only seeds Auto grid).
    return fitLayout(base, size, { ...min, keep: space.focusedPaneId });
  }, [base, size, min.minW, min.minH, space.focusedPaneId]); // eslint-disable-line react-hooks/exhaustive-deps
  const presets = useMemo(() => availablePresets(size, min), [size, min.minW, min.minH]); // eslint-disable-line react-hooks/exhaustive-deps
  const fonts = useMemo(() => ({ base: fontSize, density, overrides: ui.tileFont }), [fontSize, density, ui.tileFont]);

  const hiddenKey = hidden.map((p) => p.id).join(",");
  useEffect(() => setHiddenPanes(space.id, hiddenKey ? hiddenKey.split(",") : []), [space.id, hiddenKey]);

  const visible = new Set(agentsIn(tree));
  const byId = new Map(agents.map((a) => [a.id, a]));
  const chips = spaceMembers(space, agents).filter((a) => !visible.has(a.id));
  const paneCount = agentsIn(space.layout).length;

  return (
    <div className="space">
      <div className="space-bar">
        <span className="label">{space.name}</span>
        <span className="muted-sm">
          {paneCount} shown
          {hidden.length > 0 && ` · ${hidden.length} folded (too small at this density)`}
        </span>
        <span className="spacer" />
        {maximized && (
          <button className="small-btn" onClick={() => toggleMaximize(maximized.id)} title="Restore (⌘⏎)">
            <Icon name="restore" size={12} /> Restore layout
          </button>
        )}
        <select
          className="density-select"
          value={space.density ?? ""}
          onChange={(e) => setDensity(isDensity(e.target.value) ? e.target.value : null, space.id)}
          title="Tile density for this space (smallest tile before it folds into a chip)"
          aria-label="Tile density for this space"
        >
          <option value="">Default ({DENSITY_LABEL[ui.density]})</option>
          {DENSITIES.map((d) => (
            <option key={d} value={d}>
              {DENSITY_LABEL[d]} · {DENSITY_CELLS[d].cols}×{DENSITY_CELLS[d].rows}
            </option>
          ))}
        </select>
        <div className="presets" role="group" aria-label="Layout presets">
          {presets.map((p) => (
            <button
              key={p}
              className="preset-btn"
              onClick={() => applyPreset(p, size ? { area: size, min, maxPerRow } : undefined)}
              title={p === "auto" ? "Auto grid: fit all agents evenly, the rest as chips" : `Tile ${PRESET_LABEL[p]}`}
            >
              <PresetIcon preset={p} />
            </button>
          ))}
        </div>
        {onMoveToWindow && (
          <button className="icon-btn icon-btn-sm" onClick={onMoveToWindow} title="Move to new window (⌘⇧N)">
            <Icon name="window" size={14} />
          </button>
        )}
      </div>
      <div className="space-area" ref={areaRef} data-drop="space" data-space-id={space.id}>
        {tree && (
          <LayoutView
            node={tree}
            agents={byId}
            focusedPaneId={space.focusedPaneId}
            maximized={!!maximized}
            paneCount={paneCount}
            spaceId={space.id}
            members={chips}
            fonts={fonts}
          />
        )}
      </div>
      {chips.length > 0 && <ChipStrip agents={chips} ui={ui} me={me} spaceId={space.id} />}
    </div>
  );
}

function PresetIcon({ preset }: { preset: Preset }) {
  if (preset === "auto") {
    return (
      <svg width="18" height="14" viewBox="0 0 18 14" fill="currentColor" aria-hidden>
        {[1, 5.5, 10].map((x) => [1, 5].map((y) => <rect key={`${x}-${y}`} x={x} y={y} width={3.5} height={3} rx={0.6} />))}
        <rect x={1} y={9} width={16} height={4} rx={1} opacity={0.45} />
      </svg>
    );
  }
  let n = 0;
  const rects = [...paneRects(buildPreset(preset, [], (p) => `${p}${n++}`), { x: 0, y: 0, w: 17, h: 13 }).values()];
  return (
    <svg width="18" height="14" viewBox="0 0 18 14" fill="currentColor" aria-hidden>
      {rects.map((r, i) => (
        <rect key={i} x={r.x + 1} y={r.y + 1} width={Math.max(0.5, r.w - 1)} height={Math.max(0.5, r.h - 1)} rx={preset.length > 2 || preset === "3" ? 0.6 : 1} />
      ))}
    </svg>
  );
}
