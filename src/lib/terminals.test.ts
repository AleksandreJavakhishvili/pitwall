import { describe, expect, it } from "vitest";
import type { RunningElsewhere } from "../types";
import {
  bringInRequest,
  canBringIn,
  looksLikePath,
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
});
