import { useActions } from "../lib/actions";
import { Kbd } from "./Kbd";
import { Icon } from "./Icon";

export function EmptyState() {
  const { openNewAgent } = useActions();
  return (
    <div className="empty">
      <div className="empty-board" aria-hidden>
        <span>P1</span>
        <span>—</span>
        <span>BOX</span>
      </div>
      <h1 className="empty-title">The pit wall is quiet</h1>
      <p className="muted">
        Start a coding agent in one of your projects. Pitwall shows its terminal, what it changed, and tells you when it
        needs you.
      </p>
      <button className="primary-btn primary-lg" onClick={openNewAgent} data-autofocus>
        <Icon name="plus" /> New agent <Kbd>⌘N</Kbd>
      </button>
      <p className="hint">
        <Kbd>⌘K</Kbd> opens the command palette any time.
      </p>
    </div>
  );
}
