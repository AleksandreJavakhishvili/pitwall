import { Fragment, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import type { AgentView } from "../types";
import type { LayoutNode, SplitNode } from "../layout/tree";
import { useActions } from "../lib/actions";
import { PaneView } from "./PaneView";
import { ErrorBoundary } from "./ErrorBoundary";
import type { Density } from "../layout/density";

/** What a tile needs to pick its own font size. */
export interface TileFonts {
  base: number;
  density: Density;
  overrides: Record<string, number>;
}

interface Props {
  node: LayoutNode;
  agents: Map<string, AgentView>;
  focusedPaneId: string | null;
  maximized: boolean;
  paneCount: number;
  spaceId: string;
  /** Agents offered in empty panes. */
  members: AgentView[];
  fonts: TileFonts;
}

export function LayoutView(props: Props) {
  const { node } = props;
  const { closePane } = useActions();
  if (node.type === "pane") {
    // One broken pane must not take the whole space down; "Close" closes the pane.
    return (
      <ErrorBoundary where={`pane ${node.agentId ?? "(empty)"}`} variant="pane" resetKey={node.agentId} onClose={() => closePane(node.id)}>
        <PaneView
          pane={node}
          agent={node.agentId ? (props.agents.get(node.agentId) ?? null) : null}
          focused={node.id === props.focusedPaneId}
          maximized={props.maximized}
          canClose={props.paneCount > 1 || !!node.agentId}
          members={props.members}
          fonts={props.fonts}
          spaceId={props.spaceId}
        />
      </ErrorBoundary>
    );
  }
  return <SplitView {...props} node={node} />;
}

function SplitView(props: Props & { node: SplitNode }) {
  const { node } = props;
  const { setSplitSizes } = useActions();
  const ref = useRef<HTMLDivElement>(null);
  const [live, setLive] = useState<number[] | null>(null);
  const sizes = live ?? node.sizes;

  const startDrag = (index: number, e: RPointerEvent) => {
    const el = ref.current;
    if (!el) return;
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const rect = el.getBoundingClientRect();
    const total = node.dir === "row" ? rect.width : rect.height;
    const start = node.dir === "row" ? e.clientX : e.clientY;
    const initial = [...node.sizes];
    let latest = initial;
    const min = 0.08;
    const move = (ev: PointerEvent) => {
      const pos = node.dir === "row" ? ev.clientX : ev.clientY;
      let d = (pos - start) / total;
      d = Math.max(min - initial[index], Math.min(initial[index + 1] - min, d));
      latest = initial.map((s, i) => (i === index ? s + d : i === index + 1 ? s - d : s));
      setLive(latest);
    };
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      target.removeEventListener("pointercancel", up);
      document.body.classList.remove("resizing");
      setSplitSizes(node.id, latest);
      setLive(null);
    };
    document.body.classList.add("resizing");
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
    target.addEventListener("pointercancel", up);
  };

  return (
    <div className="split" data-dir={node.dir} ref={ref}>
      {node.children.map((c, i) => (
        <Fragment key={c.id}>
          {i > 0 && (
            <div
              className="divider"
              data-dir={node.dir}
              role="separator"
              aria-orientation={node.dir === "row" ? "vertical" : "horizontal"}
              onPointerDown={(e) => startDrag(i - 1, e)}
            />
          )}
          <div className="split-cell" style={{ flex: `${sizes[i]} 1 0px` }}>
            <LayoutView {...props} node={c} />
          </div>
        </Fragment>
      ))}
    </div>
  );
}
