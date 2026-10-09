import { useEffect, useState } from "react";
import type { CliStatus } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";

/** Settings → "Command-line tool": link `pitwall` into a bin folder, after asking. */
export function CliSettings() {
  const { run } = useActions();
  const [status, setStatus] = useState<CliStatus | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [dir, setDir] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    run(api.cliStatus(), "read the command-line tool's status").then((s) => {
      if (!s) return;
      setStatus(s);
      setDir(s.dirs[0]?.path ?? null);
    });
  }, [run]);

  const install = async () => {
    if (!dir) return;
    setBusy(true);
    const s = await run(api.installCli(dir), "install the command-line tool");
    setBusy(false);
    if (s) {
      setStatus(s);
      setConfirming(false);
    }
  };

  const chosen = status?.dirs.find((d) => d.path === dir);
  return (
    <div className="setting">
      <div className="setting-head">
        <span className="setting-title">Command-line tool</span>
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
        <span className="mono">pitwall</span> lets you and your agents add agents and agw sessions to Pitwall and change
        these settings from a terminal. Anything that changes another machine or something outside Pitwall waits for your
        OK here.
      </p>
      {status?.installed && <p className="hint mono">{status.installed}</p>}
      {status && !status.bin && <p className="hint">This build doesn't include the tool.</p>}
      {status?.bin && !status.installed && !confirming && (
        <button className="small-btn" onClick={() => setConfirming(true)}>
          Install command-line tool…
        </button>
      )}
      {confirming && status?.bin && (
        <div className="confirm-box">
          <p>Pitwall will create one link:</p>
          {status.dirs.map((d) => (
            <label className="check" key={d.path}>
              <input type="radio" name="cli-dir" checked={dir === d.path} onChange={() => setDir(d.path)} />
              <span className="mono">{d.path}/pitwall</span>
            </label>
          ))}
          <ul>
            <li>
              It points at <span className="mono">{status.bin}</span>; nothing else changes.
            </li>
            <li>An existing “pitwall” that isn't Pitwall's is never replaced.</li>
            <li>To uninstall, delete the link.</li>
            {chosen && !chosen.onPath && <li>Add this folder to your shell's PATH to type just “pitwall”.</li>}
          </ul>
          <div className="row gap">
            <button className="primary-btn" onClick={install} disabled={busy || !dir}>
              {busy ? "Installing…" : "Install"}
            </button>
            <button className="ghost-btn" onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
