// macOS folder access (roadmap Wave 3): the welcome screen's "Folder access"
// step, Settings → Permissions, and a one-time hint when macOS blocks a folder.
// `permissions_status` is read-only and never makes macOS ask.
import { useEffect, useState } from "react";
import { api } from "../api";
import type { PermissionsStatus } from "../types";

/** How often the status is re-read while someone is waiting for the switch. */
export const PERMISSIONS_POLL_MS = 2000;

// ── welcome-screen step ──────────────────────────────────────────────────

/**
 * checking → (already granted / nothing to ask) done
 *          → ask → waiting (System Settings opened) → granted → done
 * "Skip for now" goes to done from anywhere; macOS then asks per folder.
 */
export type AccessPhase = "checking" | "ask" | "waiting" | "granted" | "done";

export type AccessEvent =
  | { type: "status"; status: PermissionsStatus }
  | { type: "failed" }
  | { type: "open" }
  | { type: "continue" }
  | { type: "skip" };

export function accessStep(phase: AccessPhase, e: AccessEvent): AccessPhase {
  if (phase === "done") return phase;
  switch (e.type) {
    case "skip":
      return "done";
    case "failed":
      // An older backend or a failed check: never block the welcome screen.
      return phase === "checking" ? "done" : phase;
    case "status": {
      const granted = e.status.fullDiskAccess === "granted";
      if (phase === "checking") return !e.status.applies || e.status.fullDiskAccess !== "denied" ? "done" : "ask";
      if (phase === "ask" || phase === "waiting") return granted ? "granted" : phase;
      return phase;
    }
    case "open":
      return phase === "ask" || phase === "waiting" ? "waiting" : phase;
    case "continue":
      return phase === "granted" ? "done" : phase;
  }
}

/** Re-read the status every ~2 s in these phases (the user may flip the switch any time). */
export const pollsStatus = (phase: AccessPhase) => phase === "ask" || phase === "waiting";

// ── status for Settings ──────────────────────────────────────────────────

export type FdaBadge = { text: string; tone: "ok" | "warn" | "subtle" };

/** Settings → Permissions chip. */
export function fdaBadge(s: PermissionsStatus | null): FdaBadge {
  if (!s) return { text: "checking…", tone: "subtle" };
  if (!s.applies) return { text: "not needed", tone: "subtle" };
  if (s.fullDiskAccess === "granted") return { text: "granted", tone: "ok" };
  if (s.fullDiskAccess === "denied") return { text: "not granted", tone: "warn" };
  return { text: "unknown", tone: "subtle" };
}

/** Reads `permissions_status` now and every `PERMISSIONS_POLL_MS` while `poll` (and the window is visible). */
export function usePermissions(poll: boolean): { status: PermissionsStatus | null; failed: boolean } {
  const [status, setStatus] = useState<PermissionsStatus | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let alive = true;
    const load = () => {
      if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
      api.permissionsStatus().then(
        (s) => alive && setStatus(s),
        () => alive && setFailed(true),
      );
    };
    load();
    if (!poll) return () => void (alive = false);
    const t = window.setInterval(load, PERMISSIONS_POLL_MS);
    // Coming back from System Settings: check right away.
    window.addEventListener("focus", load);
    return () => {
      alive = false;
      window.clearInterval(t);
      window.removeEventListener("focus", load);
    };
  }, [poll]);
  return { status, failed };
}

// ── later: a one-time hint when macOS blocks a folder ────────────────────

/** macOS privacy refusals as they reach the UI (git, file reads, spawn). */
export function isAccessError(text: string): boolean {
  return /operation not permitted|\bEPERM\b|os error 1\b/i.test(text);
}

/** Window event the app listens to (App.tsx shows the hint). */
export const ACCESS_ERROR_EVENT = "pitwall:access-error";

/** Report a failed read: a macOS privacy refusal may earn the one-time hint. */
export function noteAccessError(text: string): void {
  if (isAccessError(text) && typeof window !== "undefined") window.dispatchEvent(new Event(ACCESS_ERROR_EVENT));
}

const HINT_KEY = "pitwall.accessHintShown";

/** True the first time only (per machine): show the Full Disk Access hint. */
export function takeAccessHint(storage: Pick<Storage, "getItem" | "setItem"> | null = safeStorage()): boolean {
  try {
    if (!storage || storage.getItem(HINT_KEY)) return false;
    storage.setItem(HINT_KEY, "1");
    return true;
  } catch {
    return false;
  }
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}
