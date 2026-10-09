import type { AgentView } from "../types";
import type { Space } from "../state/workspace";
import { Kbd } from "./Kbd";
import { Icon } from "./Icon";
import { SpaceTabs } from "./SpaceTabs";
import { keys } from "../lib/host";

interface Props {
  agents: AgentView[];
  isMock: boolean;
  spaces: Space[];
  activeSpaceId: string | null;
  wallOn: boolean;
  sidebarOpen: boolean;
  rightOpen: boolean;
  compact: boolean;
  onSelectSpace(id: string): void;
  onNewSpace(): void;
  onCloseSpace(id: string): void;
  onRenameSpace(id: string, name: string): void;
  onMoveSpace(id: string): void;
  onToggleSidebar(): void;
  onToggleRight(): void;
  onToggleWall(): void;
  reviewOn?: boolean;
  onToggleReview?(): void;
  /** The Race Engineer (docs/spec/engineer.md): open it, or show the one there is. */
  onEngineer?(): void;
  engineerOn?: boolean;
  onPalette(): void;
  onSettings(): void;
}

/** The Apex P's racing line and its chequered tail (website/index.html). */
const MARK_LINE = "M1 52H10C19 52 24 48 24 40V13H33A12.5 12.5 0 0 1 33 38H24";
const MARK_SQUARES = "M1 47.5h4.5V52H1zM5.5 52H10v4.5H5.5zM10 47.5h4.5V52H10zM14.5 52H19v4.5h-4.5z";

export function TopBar(p: Props) {
  const count = (s: AgentView["status"]) => p.agents.filter((a) => a.status === s).length;
  const working = count("working");
  const blocked = count("blocked");
  const done = count("done");

  return (
    <header className="topbar" data-tauri-drag-region>
      <div className="topbar-left" data-tauri-drag-region>
        <button
          className="icon-btn"
          onClick={p.onToggleSidebar}
          title={keys(`${p.sidebarOpen ? "Hide" : "Show"} agents (⌘B)`)}
          aria-pressed={p.sidebarOpen}
        >
          <Icon name="sidebar" />
        </button>
        <span className="wordmark" data-tauri-drag-region aria-label="Pitwall">
          {/* The Apex P, drawn exactly as the website header draws it
              (website/index.html `.brand .mark`): the racing-line P in the
              theme's ink, its chequered tail masked to the stroke. */}
          <svg className="wordmark-mark" viewBox="7 7 50 50" aria-hidden>
            <defs>
              <mask id="wordmark-mask" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">
                <path d={MARK_LINE} fill="none" stroke="#fff" strokeWidth="9" strokeLinejoin="round" />
              </mask>
            </defs>
            <g transform="translate(6.5 -.5)">
              <path className="wordmark-mark-s" d={MARK_LINE} fill="none" strokeWidth="9" strokeLinejoin="round" />
              <path className="wordmark-mark-b" mask="url(#wordmark-mask)" d={MARK_SQUARES} />
            </g>
          </svg>
          {/* The mark is the P, so the word goes on from it. */}
          {!p.compact && <span className="wordmark-rest" aria-hidden>ITWALL</span>}
        </span>
        {/* `?shots=1` hides the chip for marketing screenshots (website/public/shots). */}
        {p.isMock && new URLSearchParams(location.search).get("shots") !== "1" && (
          <span className="chip chip-mock" title="Web demo: showing mock data">MOCK</span>
        )}
      </div>

      <SpaceTabs
        spaces={p.spaces}
        activeSpaceId={p.wallOn ? null : p.activeSpaceId}
        agents={p.agents}
        onSelect={p.onSelectSpace}
        onNew={p.onNewSpace}
        onClose={p.onCloseSpace}
        onRename={p.onRenameSpace}
        onMove={p.onMoveSpace}
      />

      <div className="topbar-right" data-tauri-drag-region>
        <button className="palette-hint" onClick={p.onPalette} title={keys("Command palette (⌘K)")}>
          <Icon name="search" size={14} />
          {!p.compact && <span>Search</span>}
          <Kbd>⌘K</Kbd>
        </button>
        <div className="counts" aria-live="polite">
          {p.agents.length > 0 && (
            <span className="count" data-status="working" title="Agents working">
              <span className="count-glyph">◐</span> {working}
              {!p.compact && " working"}
            </span>
          )}
          {done > 0 && (
            <span className="count" data-status="done" title="Finished, not looked at yet">
              <span className="count-glyph">⚑</span> {done}
              {!p.compact && " done"}
            </span>
          )}
          {blocked > 0 && (
            <span className="count count-loud" data-status="blocked" title="Waiting on you">
              <span className="count-glyph">▲</span> {blocked}
              {!p.compact && " needs you"}
            </span>
          )}
        </div>
        {p.onEngineer && (
          <button
            className="icon-btn wall-btn"
            onClick={p.onEngineer}
            data-on={!!p.engineerOn}
            title="Race Engineer: Pitwall's assistant (setup, agw, spaces, settings)"
          >
            <Icon name="headset" />
          </button>
        )}
        {p.onToggleReview && (
          <button
            className="icon-btn wall-btn"
            onClick={p.onToggleReview}
            aria-pressed={!!p.reviewOn}
            data-on={!!p.reviewOn}
            title={keys("Review: what agents changed (⌘R)")}
          >
            <Icon name="review" />
          </button>
        )}
        <button
          className="icon-btn wall-btn"
          onClick={p.onToggleWall}
          aria-pressed={p.wallOn}
          data-on={p.wallOn}
          title={keys("Wall: every agent at once (⌘E)")}
        >
          <Icon name="wall" />
        </button>
        <button className="icon-btn" onClick={p.onSettings} title="Settings">
          <Icon name="gear" />
        </button>
        <button
          className="icon-btn"
          onClick={p.onToggleRight}
          title={keys(`${p.rightOpen ? "Hide" : "Show"} details (⌘.)`)}
          aria-pressed={p.rightOpen}
        >
          <Icon name="panel" />
        </button>
      </div>
    </header>
  );
}
