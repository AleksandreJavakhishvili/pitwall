import { describe, expect, it } from "vitest";
// @ts-expect-error no @types/node in this project (same as vite.config.ts)
import { readFileSync } from "node:fs";
import { isThemePref, resolveScheme, THEME_KEY, THEME_PREFS, themeAttr } from "./theme";

describe("resolveScheme", () => {
  it("System follows the OS; Dark and Light ignore it", () => {
    expect(resolveScheme("system", true)).toBe("dark");
    expect(resolveScheme("system", false)).toBe("light");
    for (const os of [true, false]) {
      expect(resolveScheme("dark", os)).toBe("dark");
      expect(resolveScheme("light", os)).toBe("light");
    }
  });

  it("maps to the <html data-theme> attribute (absent for System)", () => {
    expect(themeAttr("system")).toBeNull();
    expect(themeAttr("dark")).toBe("dark");
    expect(themeAttr("light")).toBe("light");
  });

  it("validates stored values", () => {
    for (const p of THEME_PREFS) expect(isThemePref(p)).toBe(true);
    for (const v of ["", "auto", null, undefined, 1]) expect(isThemePref(v)).toBe(false);
  });
});

// Vitest blanks CSS imports (even ?raw), so read the files from disk.
const read = (p: string): string => readFileSync(new URL(p, import.meta.url), "utf8");

describe("theme wiring", () => {
  it("index.html's pre-paint script reads the same localStorage key", () => {
    expect(read("../../index.html")).toContain(`"${THEME_KEY}"`);
  });

  it("the two light token blocks in tokens.css stay identical", () => {
    const css = read("../styles/tokens.css");
    const body = (sel: string) => {
      const i = css.indexOf(sel);
      expect(i).toBeGreaterThan(-1);
      const open = css.indexOf("{", i + sel.length - 1);
      return css.slice(open + 1, css.indexOf("}", open)).replace(/\s+/g, " ").trim();
    };
    const forced = body(':root[data-theme="light"] {');
    const system = body(":root:not([data-theme]) {");
    expect(forced.length).toBeGreaterThan(100);
    expect(system).toBe(forced);
  });
});
