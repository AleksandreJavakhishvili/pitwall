import type { AgentView } from "../types";
import { stripState } from "../lib/statusStrip";
import { Kbd } from "./Kbd";

/**
 * The bottom "needs you" strip. Always rendered at the same height
 * (--statusbar-h): only its look changes (calm / amber / mint), so it never
 * resizes the terminals above it.
 */
export function StatusStrip({ agents, onJump, onShow }: { agents: AgentView[]; onJump(): void; onShow(agentId: string): void }) {
  const s = stripState(agents);
  const lead = s.lead;
  return (
    <div className="status-strip" data-tone={s.tone} role="status" aria-live="polite">
      {s.tone === "blocked" && lead ? (
        <>
          <span className="strip-glyph" aria-hidden>
            ▲
          </span>
          <span className="strip-text">
            <strong className="strip-lead">{lead.name} needs you</strong>
            {lead.statusDetail && <span className="strip-detail">: {lead.statusDetail}</span>}
            {s.more > 0 && <span className="strip-more"> · +{s.more} more</span>}
          </span>
          <button className="strip-btn" onClick={onJump}>
            Jump <Kbd>⌘J</Kbd>
          </button>
        </>
      ) : s.tone === "done" && lead ? (
        <>
          <span className="strip-glyph" aria-hidden>
            ⚑
          </span>
          <span className="strip-text">
            <strong className="strip-lead">{lead.name} finished</strong>
            {s.more > 0 && <span className="strip-more"> · +{s.more} more</span>}
            {s.working > 0 && <span className="strip-quiet"> · ◐ {s.working} working</span>}
          </span>
          <button className="strip-btn" onClick={() => onShow(lead.id)}>
            Show
          </button>
        </>
      ) : (
        <>
          <span className="strip-text strip-calm">
            {s.total === 0 ? (
              "No agents yet"
            ) : (
              <>
                <span className="strip-count">◐ {s.working} working</span>
                <span className="strip-hint"> · Nothing needs you</span>
              </>
            )}
          </span>
          <span className="strip-keys">
            <Kbd>⌘K</Kbd> commands
          </span>
        </>
      )}
    </div>
  );
}
