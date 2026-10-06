// One xterm instance per agent, created once and kept alive for the agent's
// lifetime. Components only mount the host element; switching agents never
// recreates a terminal, so screen state and scrollback survive.
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { api, errorText } from "../api";
import { OWNED_SHORTCUT } from "../lib/shortcuts";
import { compute as responsive } from "../lib/useBreakpoint";
import { fitCells, tileBox, type Box, type TermSize } from "./spawnSize";

export type { TermSize } from "./spawnSize";

export const FONT = '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, Menlo, monospace';

const DARK: ITheme = {
  background: "#0d0f12",
  foreground: "#d9dde3",
  cursor: "#e8ebef",
  cursorAccent: "#0d0f12",
  selectionBackground: "#3a4250",
  black: "#1b1f25",
  red: "#ff6b6b",
  green: "#5fd38d",
  yellow: "#f2c35b",
  blue: "#6aa6ff",
  magenta: "#c792ea",
  cyan: "#5fc9d3",
  white: "#d9dde3",
  brightBlack: "#5b636f",
  brightRed: "#ff8787",
  brightGreen: "#7fe0a5",
  brightYellow: "#ffd479",
  brightBlue: "#8cbcff",
  brightMagenta: "#d7aefb",
  brightCyan: "#7fdde5",
  brightWhite: "#f4f6f8",
};

const LIGHT: ITheme = {
  background: "#fbfbfc",
  foreground: "#1d2128",
  cursor: "#1d2128",
  cursorAccent: "#fbfbfc",
  selectionBackground: "#c9d6ea",
  black: "#1d2128",
  red: "#c92a2a",
  green: "#2b8a3e",
  yellow: "#a96800",
  blue: "#1c64d6",
  magenta: "#8b3fc4",
  cyan: "#0b7f8a",
  white: "#d5d9de",
  brightBlack: "#6b7280",
  brightRed: "#e03131",
  brightGreen: "#37a24f",
  brightYellow: "#c27c00",
  brightBlue: "#3b7ee8",
  brightMagenta: "#a35ad8",
  brightCyan: "#1898a4",
  brightWhite: "#ffffff",
};

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
const currentTheme = () => (darkQuery.matches ? DARK : LIGHT);

interface Entry {
  term: Terminal;
  fit: FitAddon | null;
  host: HTMLDivElement;
  opened: boolean;
  detach: (() => void) | null;
  gen: number;
  cols: number;
  rows: number;
  /** Size of the last successful fit (survives the forced re-send after a restart). */
  fitted: TermSize | null;
  /** View-only instances (Wall) never send input or resize the PTY. */
  viewOnly: boolean;
  /** Per-tile font size (set by its pane); null follows the global base size. */
  font?: number | null;
}

const entries = new Map<string, Entry>(); // interactive, one per agent
const viewers = new Map<string, Entry>(); // view-only, Wall only
let fontSize = 13;
/** Last size any interactive terminal was fitted to. */
let lastFitted: TermSize | null = null;

const fontsReady: Promise<unknown> = document.fonts
  ? Promise.all([
      document.fonts.load(`13px "JetBrains Mono Variable"`),
      document.fonts.load(`bold 13px "JetBrains Mono Variable"`),
    ]).catch(() => undefined)
  : Promise.resolve();

darkQuery.addEventListener("change", () => {
  for (const e of [...entries.values(), ...viewers.values()]) e.term.options.theme = currentTheme();
});

/** Off-screen home for terminals that are not in any pane right now. */
function parking(): HTMLElement {
  let el = document.getElementById("term-parking");
  if (!el) {
    el = document.createElement("div");
    el.id = "term-parking";
    el.setAttribute("aria-hidden", "true");
    el.style.cssText = "position:fixed;left:-10000px;top:0;width:0;height:0;overflow:hidden;";
    document.body.appendChild(el);
  }
  return el;
}

function makeTerminal(viewOnly: boolean): Terminal {
  const term = new Terminal({
    fontFamily: FONT,
    fontSize,
    lineHeight: 1.15,
    allowProposedApi: true,
    cursorBlink: !viewOnly,
    scrollback: viewOnly ? 500 : 10_000,
    macOptionIsMeta: true,
    theme: currentTheme(),
    disableStdin: viewOnly,
  });
  term.loadAddon(new Unicode11Addon());
  term.unicode.activeVersion = "11";
  return term;
}

function create(agentId: string, running: boolean): Entry {
  const term = makeTerminal(false);
  const fit = new FitAddon();
  term.loadAddon(fit);
  // Let app shortcuts (⌘K, ⌘J, ⌘1…) through instead of the terminal eating them.
  term.attachCustomKeyEventHandler((ev) => !OWNED_SHORTCUT(ev));
  term.onData((data) => {
    api.writeInput(agentId, data).catch(() => {});
  });
  const host = document.createElement("div");
  host.className = "xterm-host";
  const entry: Entry = {
    term,
    fit,
    host,
    opened: false,
    detach: null,
    gen: 0,
    cols: 0,
    rows: 0,
    fitted: null,
    viewOnly: false,
  };
  entries.set(agentId, entry);
  if (running) attach(agentId, entry);
  return entry;
}

function attach(agentId: string, e: Entry) {
  const gen = ++e.gen;
  e.detach?.();
  e.detach = null;
  api
    .attachOutput(agentId, (bytes) => {
      if (e.gen === gen) e.term.write(bytes);
    })
    .then((detach) => {
      if (e.gen === gen) e.detach = detach;
      else detach();
    })
    .catch((err) => {
      // Stopped/exited agents can't be attached; their overlay explains it.
      console.warn(`[pitwall] attach ${agentId}: ${errorText(err)}`);
    });
}

function open(e: Entry, webgl: boolean) {
  if (e.opened) return;
  e.opened = true;
  e.term.open(e.host);
  if (!webgl) return;
  try {
    const addon = new WebglAddon();
    addon.onContextLoss(() => addon.dispose()); // fall back to the DOM renderer
    e.term.loadAddon(addon);
  } catch {
    // WebGL unavailable: xterm's default renderer is fine.
  }
}

/** Mount the agent's interactive terminal into `parent` (idempotent; re-parents). */
export function mountTerminal(agentId: string, parent: HTMLElement, running: boolean) {
  const e = entries.get(agentId) ?? create(agentId, running);
  if (e.host.parentElement !== parent) parent.appendChild(e.host);
  fontsReady.then(() => {
    if (entries.get(agentId) !== e) return;
    open(e, true);
    fitTerminal(agentId);
  });
}

/** Move the terminal out of `parent` (if it is still there) into the parking lot. */
export function unmountTerminal(agentId: string, parent: HTMLElement) {
  const e = entries.get(agentId);
  if (e && e.host.parentElement === parent) parking().appendChild(e.host);
}

/** Fit to the container and tell the backend about new dimensions. */
export function fitTerminal(agentId: string) {
  const e = entries.get(agentId);
  if (!e || !e.opened || !e.fit || !e.host.isConnected || e.host.closest("#term-parking, .wall")) return;
  const rect = e.host.getBoundingClientRect();
  if (rect.width < 20 || rect.height < 20) return;
  try {
    e.fit.fit();
  } catch {
    return;
  }
  const { cols, rows } = e.term;
  e.fitted = { cols, rows };
  lastFitted = e.fitted;
  if (cols !== e.cols || rows !== e.rows) {
    e.cols = cols;
    e.rows = rows;
    api.resize(agentId, cols, rows).catch(() => {
      // Not delivered (agent not there yet / gone): send again on the next fit.
      if (e.cols === cols && e.rows === rows) e.cols = 0;
    });
  }
}

export function focusTerminal(agentId: string) {
  entries.get(agentId)?.term.focus();
}

export function hasTerminal(agentId: string): boolean {
  return entries.has(agentId);
}

export function terminalSize(agentId: string): { cols: number; rows: number } | null {
  const e = entries.get(agentId);
  return e && e.opened ? { cols: e.term.cols, rows: e.term.rows } : null;
}

// ── spawn sizes: start each process at the size it will be shown at ────────

/** The size the agent's terminal was last fitted to, if it has one. */
export function fittedSize(agentId: string): TermSize | undefined {
  return entries.get(agentId)?.fitted ?? undefined;
}

function cellOf(term: Terminal): Box | null {
  // FitAddon reads the same private field.
  const cell = (term as unknown as { _core?: { _renderService?: { dimensions?: { css?: { cell?: { width: number; height: number } } } } } })
    ._core?._renderService?.dimensions?.css?.cell;
  return cell && cell.width > 0 && cell.height > 0 ? { w: cell.width, h: cell.height } : null;
}

let probed: { size: number; cell: Box } | null = null;

/** Cell size at the current font: from an open terminal, else a throwaway one. */
async function cellSize(): Promise<Box | null> {
  for (const e of entries.values()) {
    const c = e.opened && e.term.options.fontSize === fontSize ? cellOf(e.term) : null;
    if (c) return c;
  }
  if (probed?.size === fontSize) return probed.cell;
  await fontsReady;
  const host = document.createElement("div");
  host.style.cssText = "width:400px;height:200px;";
  parking().appendChild(host);
  const term = makeTerminal(true);
  try {
    term.open(host);
    const cell = cellOf(term);
    if (cell) probed = { size: fontSize, cell };
    return cell;
  } catch {
    return null;
  } finally {
    term.dispose();
    host.remove();
  }
}

function boxOf(el: Element | null | undefined): Box | null {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return r.width >= 20 && r.height >= 20 ? { w: r.width, h: r.height } : null;
}

/** Size of the terminal showing in `pane`, if it holds a fitted one. */
function sizeInPane(pane: Element | null): TermSize | null {
  const host = pane?.querySelector(".xterm-host");
  if (!host) return null;
  for (const e of entries.values()) if (e.host === host && e.fitted) return e.fitted;
  return null;
}

/**
 * Size for a new agent about to be placed in this window's active space
 * (W.placeAgent: the first empty pane, else the focused one). Falls back to
 * the focused pane's terminal, then the last fitted terminal; `undefined`
 * leaves it to the backend default.
 */
export async function newAgentSize(): Promise<TermSize | undefined> {
  const area = document.querySelector(".space-area");
  if (!area) return tiledAgentSize(0, 1); // first agent: it gets the whole space
  const focused = area?.querySelector('.pane[data-focused="true"]') ?? null;
  const target = area?.querySelector('.pane[data-status="empty"]') ?? focused;
  const own = sizeInPane(target);
  if (own) return own;
  const box = boxOf(target);
  const cell = box ? await cellSize() : null;
  const fit = box && cell ? fitCells(box, cell) : null;
  return fit ?? sizeInPane(focused) ?? lastFitted ?? undefined;
}

/**
 * Size for agent `index` of `count` about to be tiled into a space (onboarding
 * hand-over). Estimated from the space area (or the main area when no space is
 * shown yet), leaving room for a docked right panel that will appear.
 */
export async function tiledAgentSize(index: number, count: number): Promise<TermSize | undefined> {
  let area = boxOf(document.querySelector(".space-area"));
  if (!area) {
    const main = boxOf(document.querySelector("main.center"));
    const willDock = responsive(window.innerWidth).rightDocked && !document.querySelector("aside.right:not(.right-drawer)");
    const rightPx = willDock ? parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--right-w")) || 0 : 0;
    if (main) area = { w: main.w - rightPx, h: main.h - 32 /* .space-bar */ };
  }
  const cell = area ? await cellSize() : null;
  const fit = area && cell ? fitCells(tileBox(area, index, count), cell) : null;
  return fit ?? lastFitted ?? undefined;
}

/**
 * Onboarding hand-over: agent `index` of `planned` new ones, tiled into the
 * All space with up to `existing` agents already there (App: W.tileAgents).
 */
export function handOverSize(index: number, planned: number, existing: number): Promise<TermSize | undefined> {
  const cap = Math.min(6, Math.max(2, responsive(window.innerWidth).maxPerRow * 2));
  return tiledAgentSize(Math.max(0, index), Math.max(1, Math.min(cap, planned + existing)));
}

/** After a restart: clear the screen and attach a fresh output stream. */
export function reattachTerminal(agentId: string) {
  const e = entries.get(agentId);
  if (!e) return;
  e.term.reset();
  attach(agentId, e);
  e.cols = 0; // force a resize to reach the new process
  fitTerminal(agentId);
}

export function disposeTerminal(agentId: string) {
  const e = entries.get(agentId);
  if (!e) return;
  e.gen++;
  e.detach?.();
  e.term.dispose();
  e.host.remove();
  entries.delete(agentId);
}

export function knownTerminals(): string[] {
  return [...entries.keys()];
}

/** Base font size: every terminal without a per-tile size (and every Wall viewer). */
export function setTerminalFontSize(size: number) {
  if (size === fontSize) return;
  fontSize = size;
  for (const [id, e] of entries) {
    if (e.font != null) continue;
    e.term.options.fontSize = size;
    fitTerminal(id);
  }
  for (const e of viewers.values()) e.term.options.fontSize = size;
}

/**
 * Per-tile font size (auto-shrink or ⌘+ / ⌘− on the focused tile), then refit:
 * the backend still gets the real cols × rows through fitTerminal's resize.
 */
export function setTileFont(agentId: string, size: number | null) {
  const e = entries.get(agentId);
  if (!e) return;
  e.font = size;
  const want = size ?? fontSize;
  if (e.term.options.fontSize !== want) e.term.options.fontSize = want;
  fitTerminal(agentId);
}

/** The font size the agent's terminal shows now. */
export function tileFontOf(agentId: string): number | undefined {
  return entries.get(agentId)?.term.options.fontSize;
}

/** Measured cell size per px of font for this agent's terminal, once rendered. */
export function cellPerPx(agentId: string): { w: number; h: number } | null {
  const e = entries.get(agentId);
  const size = e?.term.options.fontSize;
  const cell = e && e.opened && size ? cellOf(e.term) : null;
  return cell && size ? { w: cell.w / size, h: cell.h / size } : null;
}

// ── Wall: view-only terminals ──────────────────────────────────────────────

/**
 * Show the agent's terminal inside a Wall tile without ever resizing the PTY.
 * Reuses the interactive instance when its size matches the PTY; otherwise
 * a view-only instance is attached at the PTY's cols × rows.
 * Returns the element to scale and a cleanup fn.
 */
export function mountWallView(
  agentId: string,
  parent: HTMLElement,
  pty: { cols: number; rows: number; running: boolean },
): { element: HTMLElement; release(): void } | null {
  const own = entries.get(agentId);
  if (own && own.opened && ((own.term.cols === pty.cols && own.term.rows === pty.rows) || !pty.running)) {
    parent.appendChild(own.host);
    return {
      element: own.host,
      release: () => {
        if (own.host.parentElement === parent) parking().appendChild(own.host);
      },
    };
  }
  if (!pty.running) return null;
  let v = viewers.get(agentId);
  if (!v) {
    const term = makeTerminal(true);
    const host = document.createElement("div");
    host.className = "xterm-host xterm-viewer";
    v = { term, fit: null, host, opened: false, detach: null, gen: 0, cols: 0, rows: 0, fitted: null, viewOnly: true };
    viewers.set(agentId, v);
    term.resize(Math.max(2, pty.cols), Math.max(1, pty.rows));
    attach(agentId, v);
  } else if (v.term.cols !== pty.cols || v.term.rows !== pty.rows) {
    v.term.resize(Math.max(2, pty.cols), Math.max(1, pty.rows));
  }
  parent.appendChild(v.host);
  const entry = v;
  fontsReady.then(() => open(entry, false));
  open(entry, false);
  return { element: v.host, release: () => {} };
}

/** Leaving the Wall: drop all view-only instances and their channels. */
export function disposeWallViewers() {
  for (const v of viewers.values()) {
    v.gen++;
    v.detach?.();
    v.term.dispose();
    v.host.remove();
  }
  viewers.clear();
}
