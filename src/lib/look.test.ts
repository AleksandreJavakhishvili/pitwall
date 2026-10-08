import { describe, expect, it } from "vitest";
// @ts-expect-error no @types/node in this project (same as vite.config.ts)
import { readFileSync } from "node:fs";
import { glassLabel, glassTier, isLook, LOOK_KEY, MOTION_KEY, showsGlass } from "./look";

const on = { native: true, reduceTransparency: false, increaseContrast: false };
const off = { ...on, native: false };

describe("Glass tiers", () => {
  it("native material where the desktop has one and it went on; lite otherwise", () => {
    expect(glassTier("vibrancy", on)).toBe("native");
    expect(glassTier("mica", on)).toBe("mica");
    expect(glassTier("vibrancy", off)).toBe("lite");
    expect(glassTier("mica", null)).toBe("lite");
    expect(glassTier("none", on)).toBe("lite");
  });

  it("names the tier in Settings", () => {
    expect(glassLabel("vibrancy")).toBe("Glass · native");
    expect(glassLabel("mica")).toBe("Glass · Mica");
    expect(glassLabel("none")).toBe("Glass lite");
  });

  it("falls back to Flat when the OS asks for less transparency", () => {
    expect(showsGlass("glass", false)).toBe(true);
    expect(showsGlass("glass", true)).toBe(false);
    expect(showsGlass("flat", false)).toBe(false);
  });

  it("validates stored values", () => {
    expect(isLook("flat") && isLook("glass")).toBe(true);
    for (const v of ["", "Glass", null, undefined, 1]) expect(isLook(v)).toBe(false);
  });
});

// Vitest blanks CSS imports, so read the files from disk.
const read = (p: string): string => readFileSync(new URL(p, import.meta.url), "utf8");

describe("look wiring", () => {
  it("index.html's pre-paint script reads the same localStorage keys", () => {
    const html = read("../../index.html");
    expect(html).toContain(`"${LOOK_KEY}"`);
    expect(html).toContain(`"${MOTION_KEY}"`);
  });

  it("the two light Glass token blocks stay identical", () => {
    const css = read("../styles/glass.css");
    const body = (sel: string) => {
      const i = css.indexOf(sel);
      expect(i).toBeGreaterThan(-1);
      const open = css.indexOf("{", i + sel.length - 1);
      return css.slice(open + 1, css.indexOf("}", open)).replace(/\s+/g, " ").trim();
    };
    const forced = body(':root[data-look="glass"][data-theme="light"] {');
    const system = body(':root[data-look="glass"]:not([data-theme]) {');
    expect(forced.length).toBeGreaterThan(100);
    expect(system).toBe(forced);
  });

  it("terminals and Wall tiles never get glass", () => {
    const css = read("../styles/glass.css");
    expect(css).not.toMatch(/\.(pane|wall-tile|xterm|term-slot)[^-]/);
  });
});
