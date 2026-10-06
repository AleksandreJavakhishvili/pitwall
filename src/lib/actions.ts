import { createContext, useContext } from "react";
import type { AgentView, FileChange } from "../types";
import type { Preset, Side } from "../layout/tree";
import type { Density } from "../layout/density";
import type { TilingContext } from "./tiling";

/** App-level actions shared with nested components. */
export interface Actions {
  /** Focus an agent wherever it lives, or show it in the current space. */
  showAgent(agentId: string): void;
  /** Run a backend call; failures become a red toast. Resolves to undefined on failure. */
  run<T>(p: Promise<T>, what: string): Promise<T | undefined>;
  /** Apply an AgentView returned by a command immediately. */
  patch(a: AgentView): void;
  openDiff(agentId: string, file: FileChange): void;
  openRemove(agentId: string): void;
  /** `projectPath` preselects the project (non-string args, e.g. click events, are ignored). */
  openNewAgent(projectPath?: unknown): void;
  restart(agentId: string): void;

  // spaces & panes (active space of this window unless given)
  dropAgent(agentId: string, target: { spaceId?: string; paneId?: string; side?: Side }): void;
  focusPane(paneId: string): void;
  closePane(paneId: string): void;
  toggleMaximize(paneId?: string): void;
  setSplitSizes(splitId: string, sizes: number[]): void;
  /** `ctx` sizes Auto grid; without it the active space area is measured. */
  applyPreset(preset: Preset, ctx?: TilingContext): void;
  /** Global density, or a space's own (`null` = follow the global one). */
  setDensity(density: Density | null, spaceId?: string): void;
  /** Base terminal font size (`null` = default). */
  setBaseFont(size: number | null): void;
  /** Current global layout preferences (Settings). */
  layoutPrefs: { density: Density; fontSize: number };
  openProjectSpace(project: string, display: string): void;
  toggleCollapsed(project: string, where: "sidebar" | "wall"): void;
}

export const ActionsContext = createContext<Actions | null>(null);

export function useActions(): Actions {
  const a = useContext(ActionsContext);
  if (!a) throw new Error("ActionsContext missing");
  return a;
}
