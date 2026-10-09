// Welcome screen, step before the scan: macOS folder access (roadmap Wave 3).
// Per-folder prompts are the recommended path ("Continue"); Full Disk Access
// is the alternative. Nothing here touches Desktop/Documents/Downloads; the
// scan (which may) starts once the user continues.
import { useEffect, useReducer, useState } from "react";
import { api, errorText } from "../../api";
import { accessStep, pollsStatus, usePermissions, type AccessEvent, type AccessPhase } from "../../lib/permissions";

/** Runs the step; calls `onDone` when it's finished (continued, granted, or nothing to ask). */
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
            or Downloads, which macOS guards.
          </p>
        </header>

        <section className="onb-access" data-phase={phase}>
          <div className="onb-section-head">
            <h2 className="label">macOS asks once per folder</h2>
            <span className="spacer" />
            {granted ? (
              <span className="chip chip-subtle">not needed now</span>
            ) : (
              <span className="chip chip-ok">recommended</span>
            )}
          </div>
          <p className="muted">
            When Pitwall first reads a project in one of these folders, macOS asks “Pitwall would like to access…”. Allow
            it and it won't ask about that folder again.
          </p>
          <div className="onb-access-folders">
            <span>Desktop</span>
            <span>Documents</span>
            <span>Downloads</span>
          </div>
          <p className="hint">At most three prompts, once each. Projects anywhere else never prompt.</p>
          <div className="onb-access-fda">
            {granted ? (
              <p className="onb-access-live" aria-live="polite" data-on>
                <span className="onb-glyph" aria-hidden>
                  ✓
                </span>
                Full Disk Access is on. macOS won't ask about your folders.
              </p>
            ) : phase === "waiting" ? (
              <>
                <b>Full Disk Access</b>
                <p className="hint">
                  In Privacy &amp; Security → Full Disk Access, turn on Pitwall (if it isn't listed, click{" "}
                  <span className="mono">+</span> and choose it in Applications), then come back: this page notices by
                  itself.
                </p>
                <p className="onb-access-live" aria-live="polite">
                  Waiting for the switch…{" "}
                  <button className="retry-btn" onClick={onOpen}>
                    Open Settings again
                  </button>
                </p>
                <p className="hint">
                  If macOS offers “Quit &amp; Reopen”, either choice is fine — Pitwall picks up where you left off.
                </p>
              </>
            ) : (
              <p className="hint">
                Rather not see prompts?{" "}
                <button className="retry-btn" onClick={onOpen}>
                  Use Full Disk Access instead…
                </button>
              </p>
            )}
            {!granted && (
              <p className="hint">
                Full Disk Access lets Pitwall and every agent it starts read everything on this Mac, not just your
                projects.
              </p>
            )}
          </div>
          {openError && <p className="form-error">{openError}</p>}
        </section>

        <footer className="onb-foot">
          <span className="muted-sm">You can change this later in Settings → Folder access.</span>
          <span className="spacer" />
          {granted ? (
            <button className="primary-btn primary-lg" onClick={onContinue} autoFocus>
              Continue →
            </button>
          ) : (
            <button className="primary-btn primary-lg" onClick={onSkip}>
              Continue
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}
