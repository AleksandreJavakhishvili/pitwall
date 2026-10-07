import { keys } from "../lib/host";
import { updatedLabel } from "../lib/freshness";
import { useNow } from "../lib/useNow";
import { Icon } from "./Icon";

/** The refresh shortcut, as this desktop types it (⌘⇧R; Ctrl+Shift+Alt+R where the chord is Ctrl+Shift). */
export const refreshTitle = () => `Refresh (${keys("⌘⇧R")})`;

/** "updated 12 s ago", kept current. */
export function UpdatedAgo({ at }: { at: number }) {
  const now = useNow(5_000);
  return <>{updatedLabel(at, Math.max(now, at))}</>;
}

/** "refreshing…" / "updated 12 s ago" and the ↻ button (a source-control view's header). */
export function RefreshControl({
  refreshing,
  updatedAt,
  onRefresh,
  label = "Refresh",
  note = true,
}: {
  refreshing: boolean;
  updatedAt: number | null;
  onRefresh(): void;
  /** What it refreshes, for screen readers ("Refresh changes"). */
  label?: string;
  /** Show the text next to the button. */
  note?: boolean;
}) {
  return (
    <span className="fresh">
      {note && (
        <span className="fresh-note" aria-live="polite">
          {refreshing ? "refreshing…" : updatedAt !== null ? <UpdatedAgo at={updatedAt} /> : null}
        </span>
      )}
      <button
        type="button"
        className="icon-btn icon-btn-sm fresh-btn"
        data-busy={refreshing}
        aria-busy={refreshing}
        aria-label={label}
        title={refreshTitle()}
        onClick={onRefresh}
      >
        <Icon name="refresh" size={13} />
      </button>
    </span>
  );
}

/** An error next to the data it concerns, with Retry. */
export function FreshError({ error, onRetry, prefix = "" }: { error: string; onRetry(): void; prefix?: string }) {
  const first = error.split("\n")[0].slice(0, 160);
  return (
    <p className="hint hint-error fresh-error" role="alert">
      <span title={error}>
        {prefix}
        {first}
      </span>
      <button type="button" className="retry-btn" onClick={onRetry}>
        Retry
      </button>
    </p>
  );
}
