// The pure part of the Wall's snapshot renderer (screenView.ts): attribute
// bits, xterm's colour rules and cell metrics. No DOM, so it is unit-tested.
import type { ITheme } from "@xterm/xterm";

/** xterm's line height (registry.ts terminals use the same). */
export const LINE_HEIGHT = 1.15;

// ScreenRun attribute bits (pitwall-proto `screen_attr`).
export const A = {
  BOLD: 1,
  ITALIC: 1 << 1,
  DIM: 1 << 2,
  INVERSE: 1 << 3,
  HIDDEN: 1 << 4,
  STRIKE: 1 << 5,
  UNDERLINE_SHIFT: 6,
  UNDERLINE_MASK: 0b111 << 6,
  WIDE: 1 << 9,
  CLUSTER: 1 << 10,
} as const;
export const RGB_FLAG = 0x100_0000;

// ── colours ─────────────────────────────────────────────────────────────────

const hex2 = (n: number) => n.toString(16).padStart(2, "0");

/** xterm's 256 colours: the theme's 16, then the 6×6×6 cube and 24 greys. */
export function palette(theme: ITheme): string[] {
  const t = theme;
  const out = [
    t.black, t.red, t.green, t.yellow, t.blue, t.magenta, t.cyan, t.white,
    t.brightBlack, t.brightRed, t.brightGreen, t.brightYellow, t.brightBlue, t.brightMagenta, t.brightCyan, t.brightWhite,
  ].map((c) => (c ?? "#000000").toLowerCase());
  const v = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
  for (let i = 0; i < 216; i++) out.push(`#${hex2(v[((i / 36) % 6) | 0])}${hex2(v[((i / 6) % 6) | 0])}${hex2(v[i % 6])}`);
  for (let i = 0; i < 24; i++) out.push(`#${hex2(8 + i * 10).repeat(3)}`);
  return out;
}

/** xterm's `color.multiplyOpacity(c, 0.5)` for an opaque #rrggbb. */
const half = (css: string) => `${css.slice(0, 7)}80`;

export interface Colors {
  fg: string;
  bg: string;
  cursor: string;
  ansi: string[];
}

export const colorsOf = (theme: ITheme): Colors => ({
  fg: (theme.foreground ?? "#ffffff").toLowerCase(),
  bg: (theme.background ?? "#000000").toLowerCase(),
  cursor: (theme.cursor ?? "#ffffff").toLowerCase(),
  ansi: palette(theme),
});

/** CSS for one span, as DomRendererRowFactory styles a cell (minimum contrast off). */
export function spanStyle(fgIn: number, bgIn: number, attrs: number, c: Colors): string {
  let fg = fgIn;
  let bg = bgIn;
  const inverse = (attrs & A.INVERSE) !== 0;
  if (inverse) [fg, bg] = [bg, fg];
  const dim = (attrs & A.DIM) !== 0;
  let css = "";
  // Background.
  if (bg >= RGB_FLAG) css += `background-color:#${(bg & 0xffffff).toString(16).padStart(6, "0")};`;
  else if (bg > 0) css += `background-color:${c.ansi[bg - 1]};`;
  else if (inverse) css += `background-color:${c.fg};`;
  // Foreground.
  if (fg >= RGB_FLAG) {
    // An inline truecolor beats xterm's .xterm-dim rule: no dimming.
    css += `color:#${(fg & 0xffffff).toString(16).padStart(6, "0")};`;
  } else if (fg > 0) {
    let i = fg - 1;
    if (attrs & A.BOLD && i < 8) i += 8; // drawBoldTextInBrightColors
    css += `color:${dim ? half(c.ansi[i]) : c.ansi[i]};`;
  } else if (inverse) {
    css += `color:${dim ? half(c.bg) : c.bg};`;
  } else if (dim) {
    css += `color:${half(c.fg)};`;
  }
  if (attrs & A.BOLD) css += "font-weight:bold;";
  if (attrs & A.ITALIC) css += "font-style:italic;";
  // xterm.css: .xterm-strikethrough comes after the underline rules and wins.
  const ul = (attrs & A.UNDERLINE_MASK) >> A.UNDERLINE_SHIFT;
  if (attrs & A.STRIKE) css += "text-decoration:line-through;";
  else if (ul) css += `text-decoration:${["", "underline", "double underline", "wavy underline", "dotted underline", "dashed underline"][ul] ?? "underline"};`;
  return css;
}

// ── metrics ─────────────────────────────────────────────────────────────────

export interface Metrics {
  cellW: number;
  cellH: number;
  width: number;
  height: number;
}

/** RenderDimensions as xterm's DomRenderer computes them (letterSpacing 0). */
export function metricsFor(char: { w: number; h: number }, cols: number, rows: number, dpr: number): Metrics {
  const devCellW = char.w * dpr;
  const devCellH = Math.floor(Math.ceil(char.h * dpr) * LINE_HEIGHT);
  const width = Math.round((devCellW * cols) / dpr);
  const height = Math.round((devCellH * rows) / dpr);
  return { cellW: width / cols, cellH: height / rows, width, height };
}

