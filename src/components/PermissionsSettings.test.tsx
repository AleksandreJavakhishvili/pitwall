// Settings → Permissions: Full Disk Access status and the Open Settings button.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { Access, PermissionsStatus } from "../types";
import { PermissionsRow } from "./PermissionsSettings";

const st = (fullDiskAccess: Access): PermissionsStatus => ({
  applies: true,
  fullDiskAccess,
  desktop: fullDiskAccess === "granted" ? "granted" : "unknown",
  documents: "unknown",
  downloads: "unknown",
});
const html = (s: PermissionsStatus | null) => renderToStaticMarkup(<PermissionsRow status={s} onOpen={() => {}} />);

describe("settings permissions row", () => {
  it("offers Open Settings with the steps when not granted", () => {
    const h = html(st("denied"));
    expect(h).toContain("Full Disk Access");
    expect(h).toContain("chip-warn");
    expect(h).toContain("not granted");
    expect(h).toContain("Open Settings");
    expect(h).toContain("Privacy &amp; Security");
  });

  it("just confirms when granted", () => {
    const h = html(st("granted"));
    expect(h).toContain("chip-ok");
    expect(h).toContain("granted");
    expect(h).not.toContain("Open Settings");
  });

  it("shows checking before the first answer", () => {
    const h = html(null);
    expect(h).toContain("checking…");
    expect(h).not.toContain("<button");
  });
});
