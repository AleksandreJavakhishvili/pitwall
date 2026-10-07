// Welcome screen step 1 (macOS folder access): what each phase shows.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { FolderAccessView } from "./FolderAccess";
import type { AccessPhase } from "../../lib/permissions";

const html = (phase: AccessPhase, openError: string | null = null) =>
  renderToStaticMarkup(<FolderAccessView phase={phase} openError={openError} onOpen={() => {}} onSkip={() => {}} onContinue={() => {}} />);

describe("folder access step", () => {
  it("explains why, with three steps, Open Settings and Skip for now", () => {
    const h = html("ask");
    expect(h).toContain("Desktop, Documents");
    expect(h).toContain("Open Settings");
    expect(h).toContain("Turn on Pitwall");
    expect(h).toContain("Come back here");
    expect(h).toContain("Skip for now");
    expect(h).not.toContain("Continue");
  });

  it("waits for the switch after Open Settings", () => {
    const h = html("waiting");
    expect(h).toContain("Waiting for the switch");
    expect(h).toContain("Open Settings again");
    expect(h).toContain("Quit &amp; Reopen");
  });

  it("shows the grant and moves on with Continue", () => {
    const h = html("granted");
    expect(h).toContain("✓ granted");
    expect(h).toContain("Full Disk Access is on");
    expect(h).toContain("Continue");
    expect(h).not.toContain("Skip for now");
  });

  it("renders nothing to read while checking", () => {
    expect(html("checking")).not.toContain("Full Disk Access");
  });

  it("shows why System Settings didn't open", () => {
    expect(html("waiting", "could not open System Settings")).toContain("could not open System Settings");
  });
});
