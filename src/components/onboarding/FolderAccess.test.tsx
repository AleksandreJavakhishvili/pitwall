// Welcome screen step 1 (macOS folder access): what each phase shows.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { FolderAccessView } from "./FolderAccess";
import type { AccessPhase } from "../../lib/permissions";

const html = (phase: AccessPhase, openError: string | null = null) =>
  renderToStaticMarkup(<FolderAccessView phase={phase} openError={openError} onOpen={() => {}} onSkip={() => {}} onContinue={() => {}} />);

describe("folder access step", () => {
  it("recommends the per-folder prompts with Continue; Full Disk Access is a secondary link", () => {
    const h = html("ask");
    expect(h).toContain("Desktop");
    expect(h).toContain("recommended");
    expect(h).toContain("At most three prompts");
    expect(h).toContain("Continue");
    expect(h).toContain("Use Full Disk Access instead");
    expect(h).toContain("read everything");
  });

  it("waits for the switch after Open Settings", () => {
    const h = html("waiting");
    expect(h).toContain("Waiting for the switch");
    expect(h).toContain("Open Settings again");
    expect(h).toContain("Quit &amp; Reopen");
  });

  it("shows the grant and moves on with Continue", () => {
    const h = html("granted");
    expect(h).toContain("Full Disk Access is on");
    expect(h).toContain("Continue →");
    expect(h).not.toContain("Use Full Disk Access instead");
  });

  it("renders nothing to read while checking", () => {
    expect(html("checking")).not.toContain("Full Disk Access");
    expect(html("checking")).not.toContain("Continue");
  });

  it("shows why System Settings didn't open", () => {
    expect(html("waiting", "could not open System Settings")).toContain("could not open System Settings");
  });
});
