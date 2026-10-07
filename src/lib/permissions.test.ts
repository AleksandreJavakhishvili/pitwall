// macOS folder access: the welcome step's state machine, the settings chip,
// the access-error hint and the browser mock's fake grant.
import { describe, expect, it } from "vitest";
import type { Access, PermissionsStatus } from "../types";
import { accessStep, fdaBadge, isAccessError, pollsStatus, takeAccessHint, type AccessEvent, type AccessPhase } from "./permissions";
import { createPermissionsMock, MOCK_GRANT_AFTER_MS } from "../components/onboarding/mockPermissions";

const st = (fullDiskAccess: Access, applies = true): PermissionsStatus => ({
  applies,
  fullDiskAccess,
  desktop: "unknown",
  documents: "unknown",
  downloads: "unknown",
});
const status = (a: Access, applies = true): AccessEvent => ({ type: "status", status: st(a, applies) });
const run = (events: AccessEvent[], from: AccessPhase = "checking") => events.reduce(accessStep, from);

describe("folder access step", () => {
  it("is skipped when already granted, not macOS, unknown or the check fails", () => {
    expect(run([status("granted")])).toBe("done");
    expect(run([status("denied", false)])).toBe("done");
    expect(run([status("unknown")])).toBe("done");
    expect(run([{ type: "failed" }])).toBe("done");
  });

  it("asks when denied, waits after Open Settings, and shows the grant before moving on", () => {
    expect(run([status("denied")])).toBe("ask");
    expect(run([status("denied"), { type: "open" }])).toBe("waiting");
    expect(run([status("denied"), { type: "open" }, status("denied")])).toBe("waiting");
    expect(run([status("denied"), { type: "open" }, status("granted")])).toBe("granted");
    // The grant can arrive without Open Settings (switched on by hand).
    expect(run([status("denied"), status("granted")])).toBe("granted");
    expect(run([status("denied"), status("granted"), { type: "continue" }])).toBe("done");
  });

  it("ignores events that don't fit the phase", () => {
    expect(run([{ type: "continue" }], "ask")).toBe("ask");
    expect(run([{ type: "open" }], "granted")).toBe("granted");
    expect(run([status("denied")], "granted")).toBe("granted");
    expect(run([{ type: "failed" }], "waiting")).toBe("waiting");
    expect(run([status("denied"), { type: "open" }], "done")).toBe("done");
  });

  it("can be skipped from anywhere", () => {
    for (const p of ["checking", "ask", "waiting", "granted"] as AccessPhase[]) expect(accessStep(p, { type: "skip" })).toBe("done");
  });

  it("polls only while waiting for the switch", () => {
    expect((["checking", "ask", "waiting", "granted", "done"] as AccessPhase[]).filter(pollsStatus)).toEqual(["ask", "waiting"]);
  });
});

describe("settings chip", () => {
  it("names the state", () => {
    expect(fdaBadge(null)).toEqual({ text: "checking…", tone: "subtle" });
    expect(fdaBadge(st("granted"))).toEqual({ text: "granted", tone: "ok" });
    expect(fdaBadge(st("denied"))).toEqual({ text: "not granted", tone: "warn" });
    expect(fdaBadge(st("unknown"))).toEqual({ text: "unknown", tone: "subtle" });
    expect(fdaBadge(st("unknown", false)).text).toBe("not needed");
  });
});

describe("access-error hint", () => {
  it("recognises macOS privacy refusals", () => {
    expect(isAccessError("fatal: cannot open '.git/HEAD': Operation not permitted")).toBe(true);
    expect(isAccessError("Operation not permitted (os error 1)")).toBe(true);
    expect(isAccessError("EPERM: open failed")).toBe(true);
    expect(isAccessError("fatal: not a git repository")).toBe(false);
    expect(isAccessError("os error 13")).toBe(false);
  });

  it("is shown once", () => {
    const m = new Map<string, string>();
    const storage = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
    expect(takeAccessHint(storage)).toBe(true);
    expect(takeAccessHint(storage)).toBe(false);
    expect(takeAccessHint(null)).toBe(false);
  });
});

describe("browser mock", () => {
  it("flips to granted a while after Open Settings", async () => {
    let t = 1000;
    const mock = createPermissionsMock(() => t);
    expect((await mock.permissionsStatus()).fullDiskAccess).toBe("denied");
    await mock.openPrivacySettings("fullDiskAccess");
    expect((await mock.permissionsStatus()).fullDiskAccess).toBe("denied");
    t += MOCK_GRANT_AFTER_MS;
    const s = await mock.permissionsStatus();
    expect(s.fullDiskAccess).toBe("granted");
    expect(s.desktop).toBe("granted");
  });
});
