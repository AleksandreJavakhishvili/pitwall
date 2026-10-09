// Settings → Folder access: per-folder prompts recommended, Full Disk Access advanced.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { Access, PermissionsStatus } from "../types";
import { PermissionsRow } from "./PermissionsSettings";
import { stepPage } from "./SettingsDialog";

const st = (fullDiskAccess: Access, applies = true): PermissionsStatus => ({
  applies,
  fullDiskAccess,
  desktop: fullDiskAccess === "granted" ? "granted" : "unknown",
  documents: "unknown",
  downloads: "unknown",
});
const html = (s: PermissionsStatus | null) => renderToStaticMarkup(<PermissionsRow status={s} onOpen={() => {}} />);

describe("settings folder access", () => {
  it("recommends per-folder prompts and offers Full Disk Access as advanced, with a warning", () => {
    const h = html(st("denied"));
    expect(h).toContain("Ask per folder");
    expect(h).toContain("recommended");
    expect(h).toContain("at most three prompts");
    expect(h).toContain("Advanced");
    expect(h).toContain("chip-warn");
    expect(h).toContain("not granted");
    expect(h).toContain("read everything");
    expect(h).toContain("Use Full Disk Access instead");
  });

  it("just confirms when granted", () => {
    const h = html(st("granted"));
    expect(h).toContain("chip-ok");
    expect(h).toContain("granted");
    expect(h).not.toContain("Use Full Disk Access instead");
  });

  it("shows checking before the first answer", () => {
    const h = html(null);
    expect(h).toContain("checking…");
    expect(h).not.toContain("<button");
  });

  it("says nothing is needed where macOS privacy doesn't apply", () => {
    expect(html(st("unknown", false))).toContain("guard folders the way macOS does");
  });
});

describe("settings pages", () => {
  it("arrows move between pages and wrap", () => {
    expect(stepPage("general", 1)).toBe("agents");
    expect(stepPage("general", -1)).toBe("about");
    expect(stepPage("about", 1)).toBe("general");
  });
});
