// Draws an agent's screen from the backend's screen copy (`ScreenFrame`s,
// docs/spec/wall.md) instead of a live xterm.js instance: the Wall's
// view-only tiles. No parser, no scrollback, no render loop: a frame
// replaces the rows it carries, at most ~10 times a second, and only while
// the tile is visible.
//
// It must look exactly like xterm's DOM renderer (which the Wall used
// before), so it reproduces its output: the same row/span structure and
// styles (DomRenderer.ts / DomRendererRowFactory.ts in @xterm/xterm 6), the
// same cell metrics (CharSizeService + RenderDimensions rounding), the same
// per-glyph letter-spacing correction (WidthCache), and the same colour
// rules (theme palette + xterm's 256-colour cube, bold-as-bright, dim at
// 50 % opacity, inverse, the inactive "outline" cursor).
import type { ScreenFrame } from "../gen/ScreenFrame";
import type { ScreenRun } from "../gen/ScreenRun";
import { FONT, charSize, currentTheme } from "./registry";
import type { ITheme } from "@xterm/xterm";
import { A, colorsOf, metricsFor, spanStyle, type Colors, type Metrics } from "./screenStyle";

export { A, RGB_FLAG } from "./screenStyle";
import { onSchemeChange } from "../lib/theme";

/** xterm's WidthCache: rendered width of a glyph (32 copies / 32), per font. */
class WidthCache {
  private cache = new Map<string, number>();
  private spans: HTMLSpanElement[];
  private box: HTMLDivElement;
  constructor(size: number) {
    this.box = document.createElement("div");
    this.box.setAttribute("aria-hidden", "true");
    this.box.style.cssText = `position:absolute;left:-10000px;top:0;white-space:pre;font-kerning:none;font-family:${FONT};font-size:${size}px;`;
    this.spans = [0, 1, 2, 3].map((v) => {
      const s = document.createElement("span");
      s.className = "xterm-char-measure-element";
      s.style.fontWeight = v & 1 ? "bold" : "normal";
      if (v & 2) s.style.fontStyle = "italic";
      this.box.appendChild(s);
      return s;
    });
    document.body.appendChild(this.box);
  }
  get(c: string, bold: boolean, italic: boolean): number {
    const key = c + (bold ? "B" : "") + (italic ? "I" : "");
    let w = this.cache.get(key);
    if (w === undefined) {
      const el = this.spans[(bold ? 1 : 0) | (italic ? 2 : 0)];
      el.textContent = c.repeat(32);
      w = el.offsetWidth / 32;
      if (w > 0) this.cache.set(key, w);
    }
    return w;
  }
}

const widthCaches = new Map<number, WidthCache>();
const widthsFor = (size: number) => {
  let w = widthCaches.get(size);
  if (!w) widthCaches.set(size, (w = new WidthCache(size)));
  return w;
};

// ── rows ────────────────────────────────────────────────────────────────────

/** One cell to draw: its text, cell count, style and the run's attrs. */
interface Cell {
  text: string;
  cells: number;
  fg: number;
  bg: number;
  attrs: number;
}

function* cellsOf(runs: ScreenRun[]): Generator<Cell> {
  for (const [text, fg, bg, attrs] of runs) {
    if (attrs & (A.WIDE | A.CLUSTER)) {
      yield { text, cells: attrs & A.WIDE ? 2 : 1, fg, bg, attrs };
      continue;
    }
    for (const ch of text) yield { text: ch, cells: 1, fg, bg, attrs };
  }
}

/** The spans for one row, merged the way xterm merges them. */
export function rowSpans(runs: ScreenRun[], cursorCol: number | null, m: Metrics, widths: WidthCache | { get(c: string, b: boolean, i: boolean): number }, c: Colors, defaultSpacing: number): HTMLSpanElement[] {
  const out: HTMLSpanElement[] = [];
  let span: HTMLSpanElement | null = null;
  let text = "";
  let key = "";
  let col = 0;
  const flush = () => {
    if (span) span.textContent = text;
  };
  const add = (cell: Cell) => {
    const bold = (cell.attrs & A.BOLD) !== 0;
    const italic = (cell.attrs & A.ITALIC) !== 0;
    let ch = cell.attrs & A.HIDDEN ? " " : cell.text || " ";
    const underline = (cell.attrs & A.UNDERLINE_MASK) !== 0;
    const measured = cell.text || " ";
    if (ch === " " && underline) ch = "\xa0";
    const spacing = cell.cells * m.cellW - widths.get(measured === " " && underline ? "\xa0" : measured, bold, italic);
    const isCursor = cursorCol === col;
    const style = cell.attrs & ~(A.WIDE | A.CLUSTER);
    const k = `${cell.fg}|${cell.bg}|${style}|${spacing}`;
    if (span && !isCursor && k === key && !span.dataset.cursor) {
      text += ch;
    } else {
      flush();
      span = document.createElement("span");
      let css = spanStyle(cell.fg, cell.bg, style, c);
      if (spacing !== defaultSpacing) css += `letter-spacing:${spacing}px;`;
      if (isCursor) {
        css += `outline:1px solid ${c.cursor};outline-offset:-1px;`;
        span.dataset.cursor = "1";
      }
      if (css) span.style.cssText = css;
      out.push(span);
      text = ch;
      key = isCursor ? "" : k;
    }
    col += cell.cells;
  };
  for (const cell of cellsOf(runs)) add(cell);
  // xterm draws the row at least up to the cursor.
  while (cursorCol !== null && col <= cursorCol) add({ text: " ", cells: 1, fg: 0, bg: 0, attrs: 0 });
  flush();
  return out;
}

// ── the view ────────────────────────────────────────────────────────────────

const views = new Set<ScreenView>();

let colorCache: { theme: ITheme; colors: Colors } | null = null;
/** The current theme's colours (rebuilt only when the theme changes). */
function themeColors(): Colors {
  const theme = currentTheme();
  if (colorCache?.theme !== theme) colorCache = { theme, colors: colorsOf(theme) };
  return colorCache.colors;
}
let lastDpr = typeof window === "undefined" ? 1 : window.devicePixelRatio;

/** Theme change (Settings → Appearance / macOS): redraw every snapshot view. */
export function restyleScreenViews() {
  for (const v of views) v.redraw();
}
if (typeof window !== "undefined") onSchemeChange(() => restyleScreenViews());

export class ScreenView {
  /** Same box as xterm's host in a Wall tile: `.xterm-host` sized to the screen. */
  readonly host: HTMLDivElement;
  /** The element to scale (like `.xterm-screen`). */
  readonly screen: HTMLDivElement;
  private rowsEl: HTMLDivElement;
  private rowEls: HTMLDivElement[] = [];
  private lines: ScreenRun[][] = [];
  private cols = 0;
  private rows = 0;
  private cursor: [number, number] | null = null;
  private font: number;
  private metrics: Metrics | null = null;
  private char: { w: number; h: number } | null = null;
  private disposed = false;

  /** Draw the cursor at all (xterm's `isCursorInitialized`, see registry.cursorShown). */
  private showCursor: () => boolean;

  constructor(font: number, showCursor: () => boolean = () => true) {
    this.font = font;
    this.showCursor = showCursor;
    this.host = document.createElement("div");
    this.host.className = "xterm-host wall-snap";
    this.screen = document.createElement("div");
    this.screen.className = "wall-snap-screen";
    this.rowsEl = document.createElement("div");
    this.rowsEl.className = "wall-snap-rows";
    this.rowsEl.setAttribute("aria-hidden", "true");
    this.screen.appendChild(this.rowsEl);
    this.host.appendChild(this.screen);
    views.add(this);
    this.measure();
  }

  /** (Re)measure the font, then draw everything. */
  private measure() {
    const font = this.font;
    void charSize(font).then((char) => {
      if (this.disposed || font !== this.font || !char) return;
      this.char = char;
      this.layout();
      this.redraw();
    });
  }

  setFont(font: number) {
    if (font === this.font) return;
    this.font = font;
    this.char = null;
    this.measure();
  }

  /** Apply a frame from the backend. */
  apply(f: ScreenFrame) {
    const resized = f.cols !== this.cols || f.rows !== this.rows;
    const dirty = new Set<number>();
    if (f.full || resized) {
      this.lines = Array.from({ length: f.rows }, () => []);
      for (let r = 0; r < f.rows; r++) dirty.add(r);
    }
    this.cols = f.cols;
    this.rows = f.rows;
    for (const [row, runs] of f.lines) {
      if (row < f.rows) {
        this.lines[row] = runs;
        dirty.add(row);
      }
    }
    const was = this.cursor;
    const now = (this.showCursor() && f.cursor) || null;
    if (was?.[1] !== now?.[1] || was?.[0] !== now?.[0]) {
      if (was) dirty.add(was[1]);
      if (now) dirty.add(now[1]);
    }
    this.cursor = now;
    if (resized || window.devicePixelRatio !== lastDpr) {
      lastDpr = window.devicePixelRatio;
      this.layout();
      this.redraw();
      return;
    }
    for (const r of dirty) this.drawRow(r);
  }

  /** Size the screen and rows (DomRenderer._updateDimensions). */
  private layout() {
    if (!this.char || !this.cols || !this.rows) return;
    const m = (this.metrics = metricsFor(this.char, this.cols, this.rows, window.devicePixelRatio));
    const c = themeColors();
    this.screen.style.cssText = `position:relative;width:${m.width}px;height:${m.height}px;`;
    const spacing = m.cellW - widthsFor(this.font).get("W", false, false);
    this.rowsEl.style.cssText =
      `pointer-events:none;color:${c.fg};font-family:${FONT};font-size:${this.font}px;font-kerning:none;white-space:pre;` +
      `font-weight:normal;line-height:normal;letter-spacing:${spacing}px;`;
    while (this.rowEls.length < this.rows) this.rowsEl.appendChild((this.rowEls[this.rowEls.length] = document.createElement("div")));
    while (this.rowEls.length > this.rows) this.rowEls.pop()!.remove();
    for (const el of this.rowEls) el.style.cssText = `width:${m.width}px;height:${m.cellH}px;line-height:${m.cellH}px;overflow:hidden;`;
  }

  /** Draw every row (new size, font or theme). */
  redraw() {
    if (!this.metrics) return;
    this.rowsEl.style.color = themeColors().fg;
    for (let r = 0; r < this.rows; r++) this.drawRow(r);
  }

  private drawRow(r: number) {
    const m = this.metrics;
    const el = this.rowEls[r];
    if (!m || !el) return;
    const c = themeColors();
    const widths = widthsFor(this.font);
    const spacing = m.cellW - widths.get("W", false, false);
    const cursorCol = this.cursor && this.cursor[1] === r ? Math.min(this.cursor[0], this.cols - 1) : null;
    el.replaceChildren(...rowSpans(this.lines[r] ?? [], cursorCol, m, widths, c, spacing));
  }

  dispose() {
    this.disposed = true;
    views.delete(this);
    this.host.remove();
  }
}
