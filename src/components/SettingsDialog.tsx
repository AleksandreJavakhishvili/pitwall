import { useEffect, useState } from "react";
import type { HooksStatus } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { Modal } from "./Modal";
import { RulesSettings } from "./rules/RulesSettings";
import { LayoutSettings } from "./LayoutSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { CliSettings } from "./CliSettings";
import { PermissionsSettings } from "./PermissionsSettings";

export function SettingsDialog({ onClose, onScanAgain }: { onClose(): void; onScanAgain?(): void }) {
  const { run, hideElsewhere, setHideElsewhere } = useActions();
  const [status, setStatus] = useState<HooksStatus | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    run(api.codexHooksStatus(), "read Codex hooks status").then((s) => s && setStatus(s));
  }, [run]);

  const install = async () => {
    setBusy(true);
    const s = await run(api.installCodexHooks(), "install Codex hooks");
    setBusy(false);
    if (s) {
      setStatus(s);
      setConfirming(false);
    }
  };

  return (
    <Modal title="Settings" onClose={onClose} width={520}>
      <div className="modal-body">
        {onScanAgain && (
          <div className="setting">
            <div className="setting-head">
              <span className="label">Projects & agents</span>
              <span className="spacer" />
              <button className="small-btn" onClick={onScanAgain}>
                Scan again
              </button>
            </div>
            <p className="muted">
              Look for new projects, conversations you can continue and installed agents. Read-only.
            </p>
          </div>
        )}
        <div className="setting">
          <div className="setting-head">
            <span className="label">Exact Codex status (hooks)</span>
            <span className="spacer" />
            {status === null ? (
              <span className="muted-sm">checking…</span>
            ) : status.installed ? (
              <span className="chip chip-ok">installed</span>
            ) : (
              <span className="chip chip-subtle">not installed</span>
            )}
          </div>
          <p className="muted">
            Without hooks, Pitwall reads Codex's screen to tell working, blocked and done apart. Hooks make it exact.
          </p>
          {status && <p className="hint mono">{status.path}</p>}

          {status && !status.installed && !confirming && (
            <button className="small-btn" onClick={() => setConfirming(true)}>
              Install hooks…
            </button>
          )}

          {confirming && (
            <div className="confirm-box">
              <p>
                This edits <span className="mono">~/.codex/hooks.json</span>:
              </p>
              <ul>
                <li>A backup of the current file is made first.</li>
                <li>Pitwall entries are appended; your existing hooks stay.</li>
                <li>Outside Pitwall the hook does nothing and exits silently.</li>
              </ul>
              <div className="row gap">
                <button className="primary-btn" onClick={install} disabled={busy}>
                  {busy ? "Installing…" : "Install"}
                </button>
                <button className="ghost-btn" onClick={() => setConfirming(false)}>
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>
        <div className="setting">
          <div className="setting-head">
            <span className="label">Claude Code</span>
            <span className="spacer" />
            <span className="chip chip-ok">automatic</span>
          </div>
          <p className="muted">Hooks are passed per launch. Your ~/.claude settings are never edited.</p>
        </div>
        <div className="setting">
          <label className="check">
            <input type="checkbox" checked={!hideElsewhere} onChange={(e) => setHideElsewhere(!e.target.checked)} />
            <span>
              Show agents running elsewhere
              <span className="hint block">
                A sidebar group with Claude Code, Codex and other agents running in other terminal apps (checked every
                ~10 s, read-only), each with “Bring in”.
              </span>
            </span>
          </label>
        </div>
        <PermissionsSettings />
        <CliSettings />
        <AppearanceSettings />
        <LayoutSettings />
        <RulesSettings />
      </div>
    </Modal>
  );
}
