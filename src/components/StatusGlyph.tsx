import type { Status } from "../types";
import { STATUS_GLYPH, STATUS_WORD } from "../lib/status";

/** Status flag glyph; colour + animation come from CSS via data-status. */
export function StatusGlyph({ status, size = "md" }: { status: Status; size?: "sm" | "md" | "lg" }) {
  return (
    <span className={`glyph glyph-${size}`} data-status={status} aria-label={STATUS_WORD[status]} title={STATUS_WORD[status]}>
      {status === "working" ? <span className="pulse-dot" /> : STATUS_GLYPH[status]}
    </span>
  );
}
