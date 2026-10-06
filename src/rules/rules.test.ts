import { describe, expect, it } from "vitest";
import { ruleLabel } from "./api";
import { createMockRulesApi } from "./mock";
import { rulesRequest } from "../components/rules/RulesField";

describe("rules helpers", () => {
  it("labels rule ids without the source", () => {
    expect(ruleLabel("library:web/style.md")).toBe("web/style.md");
    expect(ruleLabel("plain.md")).toBe("plain.md");
  });

  it("only asks for main-checkout writes without a worktree", () => {
    expect(rulesRequest({ ruleSetId: null, applyToMainCheckout: true }, true)).toEqual({});
    expect(rulesRequest({ ruleSetId: "s", applyToMainCheckout: true }, false)).toEqual({ ruleSetId: "s", applyToMainCheckout: true });
    expect(rulesRequest({ ruleSetId: "s", applyToMainCheckout: false }, false)).toEqual({ ruleSetId: "s" });
  });

  it("mock never applies without rulesync and rejects duplicate set names", async () => {
    const m = createMockRulesApi();
    await expect(m.apply("a")).rejects.toMatch(/rulesync/);
    await expect(m.saveSet({ name: "web defaults", ruleIds: [] })).rejects.toMatch(/already exists/);
    await m.setNpx(true);
    expect((await m.apply("a")).generated.length).toBe(1);
  });
});
