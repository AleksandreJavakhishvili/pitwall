import type { AgentView } from "../types";
import { NextUp } from "./NextUp";
import { Changes } from "./Changes";
import { LastSent } from "./LastSent";
import { StatusGlyph } from "./StatusGlyph";
import { FilesPanel } from "./explorer/FilesPanel";
import { usePersistentFlag } from "../lib/usePersistentFlag";

/** Changes / Files: one section, two tabs (Files only where the agent's folder can be read). */
function PanelTabs({ files, onFiles, changed }: { files: boolean; onFiles(on: boolean): void; changed: number }) {
  return (
    <div className="panel-tabs" role="tablist" aria-label="Changes or files">
      <button role="tab" className="panel-tab" aria-selected={!files} onClick={() => onFiles(false)}>
        Changes
        {changed > 0 && <span className="label-count">{changed}</span>}
      </button>
      <button role="tab" className="panel-tab" aria-selected={files} onClick={() => onFiles(true)}>
        Files
      </button>
    </div>
  );
}

/** Details for the focused pane's agent; docked on wide windows, a drawer otherwise. */
export function RightPanel({ agent, drawer, onClose }: { agent: AgentView; drawer?: boolean; onClose?: () => void }) {
  const [filesTab, setFilesTab] = usePersistentFlag("pitwall.right.files", false);
  const files = filesTab && agent.caps.explorer;
  const tabs = agent.caps.explorer ? <PanelTabs files={files} onFiles={setFilesTab} changed={agent.caps.diff ? agent.filesChanged : 0} /> : undefined;
  return (
    <aside className={`right ${drawer ? "right-drawer" : ""}`} aria-label={`${agent.name} details`}>
      <div className="right-head">
        <StatusGlyph status={agent.status} size="sm" />
        <span className="right-name">{agent.name}</span>
        <span className="spacer" />
        {drawer && (
          <button className="icon-btn icon-btn-sm" onClick={onClose} aria-label="Close details">
            ✕
          </button>
        )}
      </div>
      <NextUp agent={agent} />
      {files ? <FilesPanel key={agent.id} agent={agent} tabs={tabs} /> : <Changes agent={agent} tabs={tabs} />}
      <LastSent agent={agent} />
    </aside>
  );
}
