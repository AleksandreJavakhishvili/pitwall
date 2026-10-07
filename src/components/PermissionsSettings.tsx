import type { PermissionsStatus } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { fdaBadge, usePermissions } from "../lib/permissions";

/** Settings → Permissions: Full Disk Access, live while Settings is open. Hidden where macOS privacy doesn't apply. */
export function PermissionsSettings() {
  const { run } = useActions();
  const { status } = usePermissions(true);
  if (status && !status.applies) return null;
  return <PermissionsRow status={status} onOpen={() => void run(api.openPrivacySettings("fullDiskAccess"), "open System Settings")} />;
}

export function PermissionsRow({ status, onOpen }: { status: PermissionsStatus | null; onOpen(): void }) {
  const badge = fdaBadge(status);
  const granted = status?.fullDiskAccess === "granted";
  return (
    <div className="setting">
      <div className="setting-head">
        <span className="label">Permissions · Full Disk Access</span>
        <span className="spacer" />
        <span className={`chip ${badge.tone === "ok" ? "chip-ok" : badge.tone === "warn" ? "chip-warn" : "chip-subtle"}`}>
          {badge.text}
        </span>
      </div>
      <p className="muted">
        {granted
          ? "macOS won't ask about Desktop, Documents or Downloads. Turn it off in System Settings any time."
          : "Without it, macOS asks once for each protected folder (Desktop, Documents, Downloads) your projects are in."}
      </p>
      {status && !granted && (
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
  );
}
