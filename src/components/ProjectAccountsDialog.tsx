import { useState } from "react";
import type { AgentView } from "../types";
import { api, errorText } from "../api";
import { useActions } from "../lib/actions";
import { accountsOfKind, folderName, projectAccount, useAccounts } from "../lib/useAccounts";
import { Modal } from "./Modal";
import { Icon } from "./Icon";

interface Props {
  project: string;
  /** The project's agents: picking an account moves those of that kind along. */
  agents: AgentView[];
  onClose(): void;
}

/**
 * "Accounts…" on a project: for each agent that can have more than one login, which
 * account the project's new agents use, chosen from chips, or a new one (named here, it
 * becomes the project's, and an agent starts on it in the project to log in). Picking one
 * moves the project's agents of that kind there too.
 */
export function ProjectAccountsDialog({ project, agents, onClose }: Props) {
  const v = useAccounts();
  const { moveAgents, openSettings } = useActions();
  const [adding, setAdding] = useState<{ kind: string; name: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const name = folderName(project);
  const movers = (kind: string, account: string | null) =>
    agents.filter((a) => a.kind === kind && (a.account ?? null) !== account && !a.engineer).map((a) => a.id);

  const pick = async (kind: string, account: string | null, label: string) => {
    try {
      await api.setProjectAccount(project, kind, account);
    } catch (e) {
      setError(errorText(e));
      return;
    }
    onClose();
    moveAgents(movers(kind, account), account, label, kind, project, true);
  };

  const create = async () => {
    if (!adding || busy) return;
    setBusy(true);
    setError(null);
    try {
      const a = await api.addAccount(adding.kind, adding.name);
      await api.setProjectAccount(project, adding.kind, a.id);
      onClose();
      // Its login terminal opens here; the project's agents follow once logged in.
      moveAgents(movers(adding.kind, a.id), a.id, a.name, adding.kind, project, true);
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <Modal title={`Accounts · ${name}`} onClose={onClose} width={480}>
      <div className="modal-body form">
        <p className="hint">The login {name}'s agents run on. Picking one moves its agents there too, conversations included; one not logged in yet opens its login first.</p>
        {v?.kinds.map((k) => {
          const current = projectAccount(v, project, k.id) ?? null;
          const chips = [{ id: null as string | null, label: "Default" }, ...accountsOfKind(v, k.id).map((a) => ({ id: a.id as string | null, label: a.name }))];
          return (
            <div className="field" key={k.id}>
              <span className="field-label">{k.name}</span>
              <div className="acct-chips">
                {chips.map((c) => (
                  <button key={c.id ?? "default"} type="button" className="acct-chip" data-on={current === c.id} onClick={() => void pick(k.id, c.id, c.label)}>
                    {current === c.id && <Icon name="check" size={12} />}
                    {c.label}
                  </button>
                ))}
                {adding?.kind !== k.id && (
                  <button type="button" className="acct-chip acct-chip-add" onClick={() => setAdding({ kind: k.id, name: "" })}>
                    + New account
                  </button>
                )}
              </div>
              {adding?.kind === k.id && (
                <div className="row gap">
                  <input
                    className="input"
                    style={{ flex: 1 }}
                    placeholder="Work"
                    value={adding.name}
                    autoFocus
                    onChange={(e) => {
                      setError(null);
                      setAdding({ kind: k.id, name: e.target.value });
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") void create();
                      if (e.key === "Escape") {
                        e.stopPropagation();
                        setAdding(null);
                      }
                    }}
                  />
                  <button type="button" className="ghost-btn" onClick={() => setAdding(null)}>
                    Cancel
                  </button>
                  <button type="button" className="primary-btn" disabled={busy || !adding.name.trim()} onClick={() => void create()}>
                    {busy ? "Starting…" : "Create and log in"}
                  </button>
                </div>
              )}
            </div>
          );
        })}
        {v && v.kinds.length === 0 && <p className="hint">None of your agents can have more than one login.</p>}
        {error && <p className="form-error">{error}</p>}
      </div>
      <footer className="modal-foot">
        <button
          type="button"
          className="link-btn acct-manage"
          onClick={() => {
            onClose();
            openSettings("agents");
          }}
        >
          Manage accounts…
        </button>
        <button type="button" className="primary-btn" onClick={onClose} data-autofocus>
          Done
        </button>
      </footer>
    </Modal>
  );
}
