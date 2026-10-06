import type { AgentView } from "../types";
import { Kbd } from "./Kbd";

/** Only rendered when at least one agent is blocked. */
export function AttentionBar({ blocked, onJump }: { blocked: AgentView[]; onJump(): void }) {
  const first = blocked[0];
  const more = blocked.length - 1;
  return (
    <div className="attention-bar" role="status">
      <span className="attention-glyph">▲</span>
      <span className="attention-text">
        <strong>{first.name} needs you</strong>
        {first.statusDetail && <span className="attention-detail">: {first.statusDetail}</span>}
        {more > 0 && <span className="attention-more"> · +{more} more</span>}
      </span>
      <button className="attention-jump" onClick={onJump}>
        Jump <Kbd>⌘J</Kbd>
      </button>
    </div>
  );
}
