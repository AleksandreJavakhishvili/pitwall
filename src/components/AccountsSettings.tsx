import { useState } from "react";
import type { Account } from "../types";
import { api, errorText } from "../api";
import { useActions } from "../lib/actions";
import { accountsOfKind, folderName, projectsOf, useAccounts } from "../lib/useAccounts";

/**
 * Settings → Agents → Accounts: more than one Claude Code or Codex login. Each account is a
 * login folder; rows say who it is logged in as and which projects use it, with Log in, Rename
 * and Remove; "Add account" makes one and starts its login.
 */
export function AccountsSettings() {
  const v = useAccounts();
  const { logIn } = useActions();
  const [adding, setAdding] = useState<{ kind: number; name: string; dir: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const kinds = v?.kinds ?? [];

  const add = async () => {
    if (!adding || !kinds[adding.kind]) return;
    setError(null);
    const kind = kinds[adding.kind].id;
    try {
      const a = await api.addAccount(kind, adding.name, adding.dir || undefined);
      setAdding(null);
      // A folder already logged in needs no login.
      if (!adding.dir.trim()) logIn(kind, a.id);
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div className="setting accounts">
      <div className="setting-head">
        <span className="setting-title">Accounts</span>
        <span className="spacer" />
        {!adding && (
          <button className="small-btn" onClick={() => setAdding({ kind: 0, name: "", dir: "" })}>
            Add account
          </button>
        )}
      </div>
      <p className="muted">
        More than one Claude Code or Codex login: each agent runs on one, a project can pick the one its new agents use, and
        an agent can move to another (its conversation goes with it).
      </p>
      {adding && (
        <div className="acct-form">
          <div className="setting-row">
            <span className="muted">Tool</span>
            <span className="spacer" />
            <div className="seg" role="radiogroup" aria-label="Tool">
              {kinds.map((k, i) => (
                <button key={k.id} role="radio" aria-checked={i === adding.kind} className="seg-btn" data-on={i === adding.kind} onClick={() => setAdding({ ...adding, kind: i })}>
                  {k.name}
                </button>
              ))}
            </div>
          </div>
          <div className="setting-row">
            <span className="muted">Name</span>
            <span className="spacer" />
            <input className="input acct-input" placeholder="Work" value={adding.name} autoFocus onChange={(e) => setAdding({ ...adding, name: e.target.value })} />
          </div>
          <div className="setting-row">
            <span className="muted">Existing folder</span>
            <span className="spacer" />
            <input
              className="input acct-input mono"
              placeholder="optional, e.g. ~/.claude-work"
              value={adding.dir}
              spellCheck={false}
              onChange={(e) => setAdding({ ...adding, dir: e.target.value })}
            />
          </div>
          <p className="hint">
            Pitwall runs its agents with {kinds[adding.kind]?.env ?? "the login folder variable"} set to the account's folder. A new one
            asks you to log in the first time.
          </p>
          <div className="row gap acct-form-foot">
            <button className="ghost-btn" onClick={() => setAdding(null)}>
              Cancel
            </button>
            <button className="primary-btn" disabled={!adding.name.trim()} onClick={() => void add()}>
              Add and log in
            </button>
          </div>
        </div>
      )}
      {kinds.map((k) => (
        <div key={k.id} className="acct-kind">
          <span className="acct-kind-label">{k.name}</span>
          <AccountRow kind={k.id} login={k.login} folder={k.id === "codex" ? "~/.codex" : "~/.claude"} />
          {accountsOfKind(v, k.id).map((a) => (
            <AccountRow key={a.id} kind={k.id} account={a} login={a.login} folder={a.folder} projects={projectsOf(v, a.id)} onError={setError} />
          ))}
        </div>
      ))}
      {error && <p className="form-error">{error}</p>}
    </div>
  );
}

/** One account (none: the kind's Default): name, login, folder, projects; Log in, Rename, Remove. */
function AccountRow({
  kind,
  account,
  login,
  folder,
  projects = [],
  onError,
}: {
  kind: string;
  account?: Account;
  login: string | null;
  folder: string;
  projects?: string[];
  onError?(e: string): void;
}) {
  const { logIn } = useActions();
  const [renaming, setRenaming] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const shown = login ?? (account ? "not logged in" : "your own login");
  const act = (p: Promise<unknown>) => p.catch((e) => onError?.(errorText(e)));
  return (
    <div className="acct-row">
      <span className="acct-dot" data-on={!!login} />
      <span className="acct-main">
        <span className="acct-top">
          {renaming !== null ? (
            <input
              className="input acct-rename"
              value={renaming}
              autoFocus
              onChange={(e) => setRenaming(e.target.value)}
              onBlur={() => {
                if (account) void act(api.renameAccount(account.id, renaming));
                setRenaming(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                if (e.key === "Escape") {
                  e.stopPropagation();
                  setRenaming(null);
                }
              }}
            />
          ) : (
            <span className="acct-name">{account?.name ?? "Default"}</span>
          )}
          <span className="acct-login" data-on={!!login} title={shown}>
            {shown}
          </span>
        </span>
        <span className="acct-folder mono" title={folder}>
          {folder}
        </span>
        {projects.length > 0 && <span className="acct-projects">New agents in {projects.map(folderName).join(", ")}</span>}
      </span>
      {account &&
        (removing ? (
          <span className="acct-actions">
            <span className="hint">Remove? Its folder stays.</span>
            <button className="danger-btn small" onClick={() => void act(api.removeAccount(account.id))}>
              Remove
            </button>
            <button className="ghost-btn small" onClick={() => setRemoving(false)}>
              Keep
            </button>
          </span>
        ) : (
          <span className="acct-actions">
            <button className="small-btn" onClick={() => logIn(kind, account.id)}>
              {login ? "Log in again" : "Log in"}
            </button>
            <button className="small-btn" onClick={() => setRenaming(account.name)}>
              Rename
            </button>
            <button className="small-btn" onClick={() => setRemoving(true)}>
              Remove
            </button>
          </span>
        ))}
    </div>
  );
}
