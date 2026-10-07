// A `ScreenFrame` built from an xterm.js buffer — what the backend's screen
// copy sends, made in the browser. The mock backend (src/mock.ts) uses it for
// the Wall, and it is the reference when checking that the backend's frames
// (alacritty_terminal) match what xterm would show for the same bytes.
import type { Terminal, IBufferCell } from "@xterm/xterm";
import type { ScreenFrame } from "../gen/ScreenFrame";
import type { ScreenRun } from "../gen/ScreenRun";
import { A, RGB_FLAG } from "./screenView";

function color(cell: IBufferCell, fg: boolean): number {
  if (fg ? cell.isFgDefault() : cell.isBgDefault()) return 0;
  const v = fg ? cell.getFgColor() : cell.getBgColor();
  if (fg ? cell.isFgRGB() : cell.isBgRGB()) return RGB_FLAG | v;
  return v + 1;
}

function attrsOf(cell: IBufferCell): number {
  let a = 0;
  if (cell.isBold()) a |= A.BOLD;
  if (cell.isItalic()) a |= A.ITALIC;
  if (cell.isDim()) a |= A.DIM;
  if (cell.isInverse()) a |= A.INVERSE;
  if (cell.isInvisible()) a |= A.HIDDEN;
  if (cell.isStrikethrough()) a |= A.STRIKE;
  if (cell.isUnderline()) {
    const style = (cell as unknown as { extended?: { underlineStyle?: number } }).extended?.underlineStyle || 1;
    a |= style << A.UNDERLINE_SHIFT;
  }
  return a;
}

const blank = (cell: IBufferCell) =>
  !cell.getChars().trim() && cell.isBgDefault() && !cell.isInverse() && !cell.isUnderline() && !cell.isStrikethrough();

/** The visible screen of `term` as one full frame. */
export function frameFromXterm(term: Terminal): ScreenFrame {
  const buf = term.buffer.active;
  const lines: ScreenFrame["lines"] = [];
  const cell = buf.getNullCell();
  for (let y = 0; y < term.rows; y++) {
    const line = buf.getLine(buf.viewportY + y);
    const runs: ScreenRun[] = [];
    if (line) {
      let end = term.cols;
      while (end > 0 && line.getCell(end - 1, cell) && (cell.getWidth() === 0 || blank(cell))) end--;
      for (let x = 0; x < end; x++) {
        line.getCell(x, cell);
        const w = cell.getWidth();
        if (w === 0) continue;
        const chars = cell.getChars() || " ";
        const fg = color(cell, true);
        const bg = color(cell, false);
        let attrs = attrsOf(cell);
        const alone = w === 2 || [...chars].length > 1;
        if (alone) attrs |= w === 2 ? A.WIDE : A.CLUSTER;
        const last = runs[runs.length - 1];
        if (!alone && last && last[1] === fg && last[2] === bg && last[3] === attrs) last[0] += chars;
        else runs.push([chars, fg, bg, attrs]);
      }
    }
    lines.push([y, runs]);
  }
  const core = (term as unknown as { _core?: { coreService?: { isCursorHidden?: boolean } } })._core;
  const hidden = core?.coreService?.isCursorHidden ?? false;
  return { cols: term.cols, rows: term.rows, cursor: hidden ? null : [buf.cursorX, buf.cursorY], full: true, lines };
}
