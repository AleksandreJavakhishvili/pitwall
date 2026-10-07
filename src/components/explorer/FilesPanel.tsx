import type { AgentView } from "../../types";
import { useActions } from "../../lib/actions";
import { keys } from "../../lib/host";
import { RefreshControl } from "../Freshness";
import { WhereLine } from "../Changes";
import { Icon } from "../Icon";
import { ExplorerTree } from "./ExplorerTree";
import { useShowIgnored, useTree } from "./useTree";

/** A small "Show ignored files" switch (off by default). */
export function IgnoredToggle({ on, onChange }: { on: boolean; onChange(on: boolean): void }) {
  return (
    <label className="ex-check ex-ignored" title="Also list what .gitignore leaves out (dimmed)">
      <input type="checkbox" checked={on} onChange={(e) => onChange(e.target.checked)} />
      Ignored
    </label>
  );
}

/** The right panel's Files tab: the focused agent's folder, read-only (docs/spec/explorer.md). */
export function FilesPanel({ agent: a, tabs }: { agent: AgentView; tabs: React.ReactNode }) {
  const { openExplorer, openQuickOpen } = useActions();
  const [ignored, setIgnored] = useShowIgnored();
  const { state, model } = useTree(a, ignored);
  return (
    <section className="panel-section panel-files">
      <div className="section-head">
        {tabs}
        <span className="spacer" />
        <RefreshControl refreshing={state.refreshing} updatedAt={state.updatedAt} onRefresh={() => void model.refresh()} label="Refresh files" />
        <button className="icon-btn icon-btn-sm" onClick={() => openExplorer({ agentId: a.id })} title="Open the file viewer" aria-label="Open the file viewer">
          <Icon name="expand" size={13} />
        </button>
      </div>
      <WhereLine agent={a} />
      <div className="ex-tools">
        <button className="ex-tool" onClick={() => openQuickOpen(a.id)} title={keys("Go to file (⌘P)")}>
          <Icon name="file" size={12} />
          Go to file
          <kbd className="kbd">{keys("⌘P", { compact: true })}</kbd>
        </button>
        <button className="ex-tool" onClick={() => openExplorer({ agentId: a.id, pane: "search" })} title={keys("Search in files (⌘⇧F)")}>
          <Icon name="search" size={12} />
          Search
          <kbd className="kbd">{keys("⌘⇧F", { compact: true })}</kbd>
        </button>
        <span className="spacer" />
        <IgnoredToggle on={ignored} onChange={setIgnored} />
      </div>
      <ExplorerTree state={state} model={model} onOpen={(path) => openExplorer({ agentId: a.id, path })} label={`Files of ${a.name}`} />
    </section>
  );
}
