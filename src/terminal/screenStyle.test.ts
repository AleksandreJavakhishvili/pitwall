import { describe, expect, it } from "vitest";
import { A, RGB_FLAG, colorsOf, metricsFor, palette, spanStyle } from "./screenStyle";

const theme = {
  foreground: "#D9DDE3",
  background: "#0d0f12",
  cursor: "#e8ebef",
  black: "#000001",
  red: "#000002",
  green: "#000003",
  yellow: "#000004",
  blue: "#000005",
  magenta: "#000006",
  cyan: "#000007",
  white: "#000008",
  brightBlack: "#000009",
  brightRed: "#00000a",
  brightGreen: "#00000b",
  brightYellow: "#00000c",
  brightBlue: "#00000d",
  brightMagenta: "#00000e",
  brightCyan: "#00000f",
  brightWhite: "#000010",
};
const c = colorsOf(theme);
const pal = (i: number) => i + 1; // wire palette encoding

describe("palette", () => {
  it("is the theme's 16 colours, then xterm's cube and greys", () => {
    const p = palette(theme);
    expect(p).toHaveLength(256);
    expect(p[1]).toBe("#000002");
    expect(p[16]).toBe("#000000");
    expect(p[17]).toBe("#00005f");
    expect(p[196]).toBe("#ff0000");
    expect(p[231]).toBe("#ffffff");
    expect(p[232]).toBe("#080808");
    expect(p[255]).toBe("#eeeeee");
  });
});

describe("spanStyle (xterm DOM renderer colour rules)", () => {
  it("default colours draw nothing extra", () => {
    expect(spanStyle(0, 0, 0, c)).toBe("");
  });
  it("bold makes the first 8 palette colours bright, and only foregrounds", () => {
    expect(spanStyle(pal(1), pal(1), A.BOLD, c)).toBe("background-color:#000002;color:#00000a;font-weight:bold;");
    expect(spanStyle(pal(9), 0, A.BOLD, c)).toBe("color:#00000a;font-weight:bold;");
  });
  it("dim halves palette and default foregrounds but not truecolor", () => {
    expect(spanStyle(pal(2), 0, A.DIM, c)).toBe("color:#00000380;");
    expect(spanStyle(0, 0, A.DIM, c)).toBe("color:#d9dde380;");
    expect(spanStyle(RGB_FLAG | 0x123456, 0, A.DIM, c)).toBe("color:#123456;");
  });
  it("inverse swaps, with the defaults becoming each other", () => {
    expect(spanStyle(0, 0, A.INVERSE, c)).toBe("background-color:#d9dde3;color:#0d0f12;");
    expect(spanStyle(pal(4), 0, A.INVERSE, c)).toBe("background-color:#000005;color:#0d0f12;");
    expect(spanStyle(RGB_FLAG | 0x010203, pal(200), A.INVERSE, c)).toBe(`background-color:#010203;color:${palette(theme)[200]};`);
  });
  it("underline styles, and strikethrough wins over underline", () => {
    expect(spanStyle(0, 0, 3 << A.UNDERLINE_SHIFT, c)).toBe("text-decoration:wavy underline;");
    expect(spanStyle(0, 0, A.STRIKE | (1 << A.UNDERLINE_SHIFT), c)).toBe("text-decoration:line-through;");
    expect(spanStyle(0, 0, A.ITALIC, c)).toBe("font-style:italic;");
  });
});

describe("metricsFor (xterm RenderDimensions)", () => {
  it("rounds the canvas to whole CSS pixels and spreads it over the cells", () => {
    const m = metricsFor({ w: 7.8, h: 15.6 }, 100, 30, 2);
    // Device cell height: floor(ceil(15.6 * 2) * 1.15) = 36 → 18 CSS px per row.
    expect(m.height).toBe(540);
    expect(m.cellH).toBe(18);
    expect(m.width).toBe(780);
    expect(m.cellW).toBeCloseTo(7.8);
  });
});
