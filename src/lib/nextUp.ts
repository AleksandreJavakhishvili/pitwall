import { useSyncExternalStore } from "react";

// Whose Next up popover is open (one per window): the tile header's
// "N queued" chip and the palette's "Show Next up for <agent>" open it.
let open: string | null = null;
const subs = new Set<() => void>();

function set(id: string | null) {
  if (open === id) return;
  open = id;
  subs.forEach((f) => f());
}

export const openNextUp = (agentId: string) => set(agentId);
export const closeNextUp = () => set(null);

export function useNextUpOpen(): string | null {
  return useSyncExternalStore(
    (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
    () => open,
  );
}

/** "2 queued", or null with nothing queued (no chip). */
export function chipLabel(queued: number): string | null {
  return queued > 0 ? `${queued} queued` : null;
}
