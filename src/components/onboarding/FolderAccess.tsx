// Welcome screen, step before the scan: macOS folder access (roadmap Wave 3).
// Nothing here touches Desktop/Documents/Downloads; the scan (which may) only
// starts once the user granted Full Disk Access or chose "Skip for now".
import { useEffect, useReducer, useState } from "react";
import { api, errorText } from "../../api";
import { accessStep, pollsStatus, usePermissions, type AccessEvent, type AccessPhase } from "../../lib/permissions";

/** Runs the step; calls `onDone` when it's finished (granted, skipped, or nothing to ask). */
export function FolderAccessStep({ onDone }: { onDone(): void }) {
  const [phase, dispatch] = useReducer(accessStep, "checking" as AccessPhase);
  const { status, failed } = usePermissions(pollsStatus(phase));
  const [openError, setOpenError] = useState<string | null>(null);

  useEffect(() => {
    if (status) dispatch({ type: "status", status });
  }, [status]);
  useEffect(() => {
    if (failed) dispatch({ type: "failed" });
  }, [failed]);
  useEffect(() => {
    if (phase === "done") onDone();
  }, [phase, onDone]);

  const send = (e: AccessEvent) => dispatch(e);
  const open = () => {
    setOpenError(null);
    send({ type: "open" });
    api.openPrivacySettings("fullDiskAccess").catch((e) => setOpenError(errorText(e)));
  };

  return (
    <FolderAccessView
      phase={phase}
      openError={openError}
      onOpen={open}
      onSkip={() => send({ type: "skip" })}
      onContinue={() => send({ type: "continue" })}
    />
  );
}

interface ViewProps {
  phase: AccessPhase;
  openError?: string | null;
  onOpen(): void;
  onSkip(): void;
  onContinue(): void;
}

export function FolderAccessView({ phase, openError, onOpen, onSkip, onContinue }: ViewProps) {
  // A blank welcome background while the first (instant) check runs: no flash of the step.
  if (phase === "checking" || phase === "done") return <div className="onb" data-mode="welcome" aria-busy="true" />;
  const granted = phase === "granted";
  return (
    <div className="onb" data-mode="welcome" role="dialog" aria-modal="true" aria-label="Welcome to Pitwall — folder access">
      <div className="onb-inner">
        <header className="onb-head">
          <div className="onb-board" aria-hidden>
            <span>PIT</span>
            <span>1/2</span>
            <span data-live={!granted}>{granted ? "GO" : "ACCESS"}</span>
          </div>
          <h1 className="onb-title">Welcome to Pitwall</h1>
          <p className="muted onb-lede">
            First, folder access. Your agents work inside your projects, and projects often live in Desktop, Documents
            or Downloads. macOS guards those folders and asks “Pitwall would like to access…” once for each one.
          </p>
        </header>

        <section className="onb-access" data-phase={phase}>
          <div className="onb-section-head">
            <h2 className="label">Allow Full Disk Access</h2>
            <span className="spacer" />
            {granted ? (
              <span className="chip chip-ok">✓ granted</span>
            ) : (
              <span className="chip chip-subtle">not yet</span>
            )}
          </div>
          <p className="muted">
            One switch instead of a prompt per folder. Pitwall reads your projects to show what your agents changed, and
            the agents you start work in those folders.
          </p>
          <ol className="onb-access-steps">
            <li>
              <b>Open Settings</b> — <span className="muted">Privacy &amp; Security → Full Disk Access opens.</span>
            </li>
            <li>
              <b>Turn on Pitwall</b> —{" "}
              <span className="muted">
                if it isn't listed, click <span className="mono">+</span> and choose Pitwall in Applications.
              </span>
            </li>
            <li>
              <b>Come back here</b> — <span className="muted">this page notices by itself.</span>
            </li>
          </ol>
          <p className="onb-access-live" aria-live="polite" data-on={granted}>
            <span className="onb-glyph" aria-hidden>
              {granted ? "✓" : ""}
            </span>
            {granted
              ? "Full Disk Access is on. macOS won't ask about your folders again."
              : phase === "waiting"
                ? "Waiting for the switch…"
                : "Not granted yet."}
          </p>
          {phase === "waiting" && (
            <p className="hint">
              If macOS offers “Quit &amp; Reopen”, either choice is fine — Pitwall picks up where you left off.
            </p>
          )}
          {openError && <p className="form-error">{openError}</p>}
          <p className="hint">You can turn it off any time in the same place.</p>
        </section>

        <footer className="onb-foot">
          {!granted && <span className="muted-sm">Skipping is fine: macOS will ask for each folder instead.</span>}
          <span className="spacer" />
          {!granted && (
            <button className="ghost-btn" onClick={onSkip}>
              Skip for now
            </button>
          )}
          {granted ? (
            <button className="primary-btn primary-lg" onClick={onContinue} autoFocus>
              Continue →
            </button>
          ) : (
            <button className="primary-btn primary-lg" onClick={onOpen}>
              {phase === "waiting" ? "Open Settings again" : "Open Settings"}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}
