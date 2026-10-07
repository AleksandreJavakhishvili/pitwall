import type { AgentView } from "../types";
import { relTime } from "../lib/time";
import { useNow } from "../lib/useNow";

export function LastSent({ agent: a }: { agent: AgentView }) {
  const now = useNow();
  return (
    <section className="panel-section">
      <div className="section-head">
        <span className="label">Last sent</span>
        <span className="spacer" />
        {a.lastSentAt && (
          <time className="mono muted-sm" dateTime={new Date(a.lastSentAt).toISOString()} title={new Date(a.lastSentAt).toLocaleString()}>
            {relTime(a.lastSentAt, now)}
          </time>
        )}
      </div>
      {a.lastSent ? (
        <blockquote className="last-sent">{a.lastSent}</blockquote>
      ) : (
        <p className="hint">Nothing sent through Pitwall yet. Prompts you type in the terminal aren't tracked.</p>
      )}
    </section>
  );
}
