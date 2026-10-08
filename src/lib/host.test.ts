import { afterEach, describe, expect, it } from "vitest";
import { appChord, DEFAULT_HOST, host, keys, loadHost, setHost, terminalClipboardChord } from "./host";
import { OWNED_SHORTCUT } from "./shortcuts";

type Ev = Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">;
const ev = (key: string, code: string, mods: Partial<Ev> = {}): KeyboardEvent =>
  ({ key, code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods }) as KeyboardEvent;

afterEach(() => setHost(null));

describe("app shortcuts follow the desktop's modifier", () => {
  it("⌘ on a Dock desktop", () => {
    expect(appChord(ev("k", "KeyK", { metaKey: true }), "meta")).toEqual({ key: "k", shift: false });
    expect(appChord(ev("N", "KeyN", { metaKey: true, shiftKey: true }), "meta")).toEqual({ key: "n", shift: true });
    expect(appChord(ev("k", "KeyK", { ctrlKey: true }), "meta")).toBeNull();
  });

  it("Ctrl+Shift where Ctrl belongs to the terminal; Alt for the shifted variant", () => {
    expect(appChord(ev("K", "KeyK", { ctrlKey: true, shiftKey: true }), "ctrlShift")).toEqual({ key: "k", shift: false });
    // Shift turns 1 into "!", "." into ">": the physical key counts.
    expect(appChord(ev("!", "Digit1", { ctrlKey: true, shiftKey: true }), "ctrlShift")).toEqual({ key: "1", shift: false });
    expect(appChord(ev(">", "Period", { ctrlKey: true, shiftKey: true }), "ctrlShift")).toEqual({ key: ".", shift: false });
    expect(appChord(ev("N", "KeyN", { ctrlKey: true, shiftKey: true, altKey: true }), "ctrlShift")).toEqual({ key: "n", shift: true });
    // Plain Ctrl (Ctrl+C, Ctrl+R, Ctrl+W …) and ⌘ stay with the terminal.
    expect(appChord(ev("c", "KeyC", { ctrlKey: true }), "ctrlShift")).toBeNull();
    expect(appChord(ev("k", "KeyK", { metaKey: true }), "ctrlShift")).toBeNull();
  });

  it("the terminal lets owned chords through on either desktop", () => {
    setHost({ ...DEFAULT_HOST, shortcuts: "ctrlShift" });
    expect(OWNED_SHORTCUT(ev("K", "KeyK", { ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(OWNED_SHORTCUT(ev("T", "KeyT", { ctrlKey: true, shiftKey: true, altKey: true }))).toBe(true);
    expect(OWNED_SHORTCUT(ev("r", "KeyR", { ctrlKey: true }))).toBe(false);
    expect(OWNED_SHORTCUT(ev("k", "KeyK", { metaKey: true }))).toBe(false);
    setHost(null);
    expect(OWNED_SHORTCUT(ev("k", "KeyK", { metaKey: true }))).toBe(true);
  });

  it("copy / paste in a terminal: Ctrl+Shift+C / V only where Ctrl is the terminal's", () => {
    expect(terminalClipboardChord(ev("C", "KeyC", { ctrlKey: true, shiftKey: true }), "ctrlShift")).toBe("copy");
    expect(terminalClipboardChord(ev("V", "KeyV", { ctrlKey: true, shiftKey: true }), "ctrlShift")).toBe("paste");
    expect(terminalClipboardChord(ev("c", "KeyC", { ctrlKey: true }), "ctrlShift")).toBeNull();
    expect(terminalClipboardChord(ev("C", "KeyC", { ctrlKey: true, shiftKey: true }), "meta")).toBeNull();
  });
});

describe("shortcut labels", () => {
  it("are unchanged with ⌘ and spelled out with Ctrl+Shift", () => {
    expect(keys("⌘K", { mod: "meta" })).toBe("⌘K");
    expect(keys("⌘K", { mod: "ctrlShift" })).toBe("Ctrl+Shift+K");
    expect(keys("Move to new window (⌘⇧N)", { mod: "ctrlShift" })).toBe("Move to new window (Ctrl+Shift+Alt+N)");
    expect(keys("⌘+ / ⌘− / ⌘0", { mod: "ctrlShift" })).toBe("Ctrl+Shift++ / Ctrl+Shift+− / Ctrl+Shift+0");
    expect(keys("⌘↵", { mod: "ctrlShift", submit: true })).toBe("Ctrl+↵");
    expect(keys("⌘3", { mod: "ctrlShift", compact: true })).toBe("⌃⇧3");
  });
});

describe("loading what the desktop offers", () => {
  it("uses the backend's answer", async () => {
    await loadHost(async () => ({ machineLabel: "This computer", shortcuts: "ctrlShift", dataDir: "~/.local/share/pitwall", dock: false, tray: false, menu: "none", badge: "dock", localSockets: "unix", glass: "none" }));
    expect(host().shortcuts).toBe("ctrlShift");
    expect(host().machineLabel).toBe("This computer");
    await loadHost(async () => ({ machineLabel: "This PC", shortcuts: "ctrlShift", dataDir: "~/AppData/Roaming/Pitwall", dock: false, tray: true, menu: "file", badge: "taskbar", localSockets: "namedPipe", glass: "mica" }));
    expect(host().tray).toBe(true);
    expect(host().menu).toBe("file");
  });

  it("falls back to the defaults when the backend fails or is slow", async () => {
    await loadHost(() => Promise.reject(new Error("unknown command")));
    expect(host()).toEqual(DEFAULT_HOST);
    await loadHost(() => new Promise(() => {}), 10);
    expect(host()).toEqual(DEFAULT_HOST);
  });
});
