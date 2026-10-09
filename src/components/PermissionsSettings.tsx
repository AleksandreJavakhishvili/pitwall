import { useState } from "react";
import type { PermissionsStatus } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { fdaBadge, usePermissions } from "../lib/permissions";

/**
 * Settings → Folder access: macOS's per-folder prompts are the normal path;
 * Full Disk Access is an advanced option, live while Settings is open.
 */
export function PermissionsSettings() {
  const { run } = useActions();
  const { status } = usePermissions(true);
  return <PermissionsRow status={status} onOpen={() => void run(api.openPrivacySettings("fullDiskAccess"), "open System Settings")} />;
}

export function PermissionsRow({ status, onOpen }: { status: PermissionsStatus | null; onOpen(): void }) {
  const [open, setOpen] = useState(false);
  if (status && !status.applies) {
    return (
      <div className="setting">
        <p className="muted">This system doesn't guard folders the way macOS does: Pitwall reads your projects without asking.</p>
      </div>
    );
  }
  const badge = fdaBadge(status);
  const granted = status?.fullDiskAccess === "granted";
  return (
    <>
      <div className="setting">
        <div className="setting-head">
          <span className="setting-title">Ask per folder</span>
          <span className="spacer" />
          <span className={`chip ${granted ? "chip-subtle" : "chip-ok"}`}>{granted ? "not needed now" : "recommended"}</span>
        </div>
        <p className="muted">
          macOS asks “Pitwall would like to access…” the first time Pitwall or an agent reads a project in Desktop, Documents
          or Downloads: at most three prompts, once each. Projects anywhere else never prompt.
        </p>
        <p className="hint">
          Answered “Don't Allow” by mistake? Turn Pitwall on in System Settings → Privacy &amp; Security → Files &amp; Folders.
        </p>
      </div>
      <div className="setting">
        <span className="label settings-sub">Advanced</span>
        <div className="setting-head">
          <span className="setting-title">Full Disk Access</span>
          <span className="spacer" />
          <span className={`chip ${badge.tone === "ok" ? "chip-ok" : badge.tone === "warn" ? "chip-warn" : "chip-subtle"}`}>
            {badge.text}
          </span>
        </div>
        <p className="muted">
          {granted
            ? "On: macOS won't ask about any folder. Pitwall and the agents it starts can read everything on this Mac. Turn it off in System Settings any time."
            : "Skips the prompts, but lets Pitwall and every agent it starts read everything on this Mac, including mail and other apps' data."}
        </p>
        {status && !granted && !open && (
          <button className="retry-btn" onClick={() => setOpen(true)}>
            Use Full Disk Access instead…
          </button>
        )}
        {status && !granted && open && (
          <>
            <ol className="hint settings-steps">
              <li>Open Settings: Privacy &amp; Security → Full Disk Access.</li>
              <li>Turn on Pitwall (or click + and choose it in Applications).</li>
              <li>Come back; this row updates by itself.</li>
            </ol>
            <button className="small-btn" onClick={onOpen}>
              Open Settings
            </button>
          </>
        )}
      </div>
    </>
  );
}
