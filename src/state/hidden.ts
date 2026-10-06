// Panes folded away by the render-time fit (too small for this window), per space.
// Written by SpaceView, read by "show agent" so it can swap a folded agent in.
const hidden = new Map<string, Set<string>>();

export function setHiddenPanes(spaceId: string, paneIds: string[]) {
  hidden.set(spaceId, new Set(paneIds));
}

export function isPaneHidden(spaceId: string, paneId: string): boolean {
  return hidden.get(spaceId)?.has(paneId) ?? false;
}
