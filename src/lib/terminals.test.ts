import { describe, expect, it } from "vitest";
import type { AgentView, RunningElsewhere } from "../types";
import {
  bringInRequest,
  canBringIn,
  looksLikePath,
  restartAction,
  terminalFolder,
  terminalName,
  terminalRequest,
  visibleElsewhere,
} from "./terminals";

const row = (p: Partial<RunningElsewhere>): RunningElsewhere => ({
  pid: 1,
  kind: "claude",
  kindName: "Claude Code",
  cwd: "/w/shop",
  cwdDisplay: "~/w/shop",
  sessionId: "s1",
  title: null,
  inPitwall: false,
  outsideProject: false,
  displayProject: null,
  ...p,
});

describe("terminals", () => {
  it("opens where you are: focused agent, else selected project, else home", () => {
    expect(terminalFolder({ cwd: "/w/app/.claude/worktrees/x" }, "/w/app")).toBe("/w/app/.claude/worktrees/x");
    expect(terminalFolder(null, "/w/app")).toBe("/w/app");
    expect(terminalFolder(null, null)).toBe("~");
  });

  it("names a terminal after its folder, with a suffix when taken", () => {
    expect(terminalName("/w/orders-api/", [])).toBe("orders-api");
    expect(terminalName("/w/orders-api", ["orders-api"])).toBe("orders-api-2");
    expect(terminalName("~", [])).toBe("home");
    expect(terminalName("/w/My App", [])).toBe("my-app");
    expect(terminalRequest("  ", [])).toEqual({ name: "home", kind: "shell", projectPath: "~", worktree: false });
    expect(terminalRequest("/w/x", [{ name: "x" }])).toMatchObject({ name: "x-2", kind: "shell", projectPath: "/w/x" });
  });

  it("recognises typed paths", () => {
    expect(looksLikePath("~/code")).toBe(true);
    expect(looksLikePath(" /tmp")).toBe(true);
    expect(looksLikePath("~")).toBe(true);
    expect(looksLikePath("orders")).toBe(false);
    expect(looksLikePath("~foo")).toBe(false);
  });

  it("hides elsewhere rows whose conversation Pitwall already has", () => {
    const rows = [row({ pid: 1, sessionId: "a" }), row({ pid: 2, sessionId: "b" }), row({ pid: 3, sessionId: null }), row({ pid: 4, inPitwall: true })];
    const shown = visibleElsewhere(rows, [{ sessionId: "a" }, { sessionId: null }, {}]);
    expect(shown.map((r) => r.pid)).toEqual([2, 3]);
  });

  it("brings a known conversation in with continue_conversation", () => {
    expect(canBringIn(row({ sessionId: null }))).toBe(false);
    expect(canBringIn(row({ cwd: null }))).toBe(false);
    expect(bringInRequest(row({ sessionId: null }), [])).toBeNull();
    expect(bringInRequest(row({}), [{ name: "shop" }])).toEqual({
      kind: "claude",
      sessionId: "s1",
      projectPath: "/w/shop",
      name: "shop-2",
    });
    expect(bringInRequest(row({ cwd: "/Users/me", displayProject: "/w/site" }), [])).toMatchObject({
      projectPath: "/Users/me",
      displayProject: "/w/site",
      name: "site",
    });
  });
});

describe("terminal shortcuts", () => {
  const key = (k: string, shift = false) => ({ key: k, metaKey: true, ctrlKey: false, altKey: false, shiftKey: shift }) as KeyboardEvent;
  it("⌘T and ⌘⇧T belong to the app, not the terminal", async () => {
    const { OWNED_SHORTCUT } = await import("./shortcuts");
    expect(OWNED_SHORTCUT(key("t"))).toBe(true);
    expect(OWNED_SHORTCUT(key("T", true))).toBe(true);
    expect(OWNED_SHORTCUT(key("y", true))).toBe(false);
  });

  it("with Ctrl+Shift shortcuts (Linux, Windows) plain Ctrl stays the terminal's", async () => {
    const { OWNED_SHORTCUT } = await import("./shortcuts");
    const { appChord, DEFAULT_HOST, keys, setHost } = await import("./host");
    const ev = (code: string, key: string, mods: Partial<KeyboardEvent>) =>
      ({ code, key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods }) as KeyboardEvent;
    setHost({ ...DEFAULT_HOST, shortcuts: "ctrlShift" });
    try {
      expect(OWNED_SHORTCUT(ev("KeyK", "k", { ctrlKey: true }))).toBe(false);
      expect(OWNED_SHORTCUT(ev("KeyK", "K", { ctrlKey: true, shiftKey: true }))).toBe(true);
      expect(appChord(ev("Digit2", "@", { ctrlKey: true, shiftKey: true }))).toEqual({ key: "2", shift: false });
      expect(appChord(ev("KeyT", "T", { ctrlKey: true, shiftKey: true, altKey: true }))).toEqual({ key: "t", shift: true });
      expect(OWNED_SHORTCUT(key("t"))).toBe(false);
      expect(keys("⌘⇧T")).toBe("Ctrl+Shift+Alt+T");
      expect(keys("⌘K")).toBe("Ctrl+Shift+K");
      expect(keys("⌘K", { mod: "meta" })).toBe("⌘K");
    } finally {
      setHost(null);
    }
  });

  it("labels Restart/Resume from caps, naming the agent a terminal restarts as", () => {
    const caps = (resume: boolean) => ({ caps: { resume } as AgentView["caps"] });
    expect(restartAction({ ...caps(false) })).toEqual({ label: "Restart", busy: "Restarting…", note: "" });
    expect(restartAction({ ...caps(true), restartAs: null }).label).toBe("Resume");
    expect(restartAction({ ...caps(true) }).note).toContain("previous session");
    const t = restartAction({ ...caps(true), restartAs: "Claude Code" });
    expect([t.label, t.busy]).toEqual(["Resume Claude Code", "Resuming…"]);
    expect(t.note).toContain("Starts the shell and picks up Claude Code's previous session");
    const fresh = restartAction({ ...caps(false), restartAs: "Codex" });
    expect([fresh.label, fresh.note]).toEqual(["Restart Codex", " Starts the shell and Codex in it."]);
  });
});
