import type { RunningElsewhere } from "../types";
import { useActions } from "../lib/actions";
import { canBringIn } from "../lib/terminals";
import { Icon } from "./Icon";

/** Sidebar: agents running in other terminal apps (read-only, docs/spec/terminals.md). */
export function ElsewhereGroup({ rows, collapsed, onToggle }: { rows: RunningElsewhere[]; collapsed: boolean; onToggle(): void }) {
  const { openBringIn } = useActions();
  if (!rows.length) return null;
  return (
    <section className="project-group elsewhere-group">
      <header className="project-head">
        <button className="project-toggle" onClick={onToggle} aria-expanded={!collapsed} title="Agents running in other terminal apps">
          <span className="chev" data-open={!collapsed}>
            <Icon name="chevron" size={12} />
          </span>
          <span className="project-name">Elsewhere</span>
          <span className="label-count">{rows.length}</span>
        </button>
      </header>
      {!collapsed && (
        <ul className="agent-list">
          {rows.map((r) => (
            <li key={r.pid} className="elsewhere-row" title={`${r.kindName} in another terminal (pid ${r.pid})${r.cwd ? `\n${r.cwd}` : ""}`}>
              <span className="elsewhere-dot" aria-hidden />
              <span className="agent-row-main">
                <span className="agent-row-top">
                  <span className="agent-name">{r.title ?? r.cwdDisplay ?? r.kindName}</span>
                </span>
                <span className="agent-row-sub">
                  <span className="agent-kind">{r.kindName}</span>
                  {r.title && r.cwdDisplay && <span className="elsewhere-cwd">{r.cwdDisplay}</span>}
                </span>
              </span>
              <button
                className="small-btn elsewhere-bring"
                disabled={!canBringIn(r)}
                onClick={() => openBringIn(r)}
                title={canBringIn(r) ? "Resume this conversation in Pitwall" : "Pitwall can't tell which conversation this is yet"}
              >
                Bring in
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
