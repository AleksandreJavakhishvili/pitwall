// The dialog's form logic mirrors pitwall-proto's `CreateForm::{values, summarize}`:
// the same cases as its Rust tests, and an agw-shaped form as the backend sends it.
import { describe, expect, it } from "vitest";
import type { CreateChoice, CreateField, CreateForm } from "../types";
import { formValues, nameProblem, shown, summarize } from "./createForm";

const choice = (value: string, phrase: string, creates: string | null = null): CreateChoice => ({ value, label: value, detail: null, phrase, creates });
const field = (p: Partial<CreateField> & Pick<CreateField, "id" | "label">): CreateField => ({
  input: "select",
  choices: [],
  default: null,
  placeholder: null,
  hint: null,
  when: null,
  defaultsToName: false,
  rule: null,
  ...p,
});
const isNew = { field: "workspace", value: "+new" };

const form: CreateForm = {
  provider: "p",
  machine: "vm",
  machineLabel: "vm",
  folder: false,
  name: { pattern: "^[a-z]+$", maxLen: 10, hint: "letters" },
  fields: [
    field({
      id: "workspace",
      label: "Workspace",
      default: "work",
      choices: [choice("work", "in workspace work"), choice("+new", "in a new workspace {workspaceName}", "workspace {workspaceName} ({workspaceTemplate})")],
    }),
    field({ id: "workspaceName", label: "Workspace name", input: "text", when: isNew, defaultsToName: true, rule: { pattern: "^[a-z]+$", maxLen: 10, hint: "letters" } }),
    field({ id: "workspaceTemplate", label: "Workspace template", when: isNew, default: "default", choices: [choice("default", "default")] }),
    field({ id: "runAs", label: "Runs as", default: "admin", choices: [choice("admin", "as admin")] }),
  ],
  summary: "Creates session {name} {workspace} on vm {runAs}.",
  submit: "Create",
  error: null,
};

describe("a machine's create form", () => {
  it("fills defaults for the fields that apply", () => {
    expect(formValues(form, {})).toEqual({ workspace: "work", runAs: "admin" });
    expect(formValues(form, { workspace: "+new" })).toEqual({ workspace: "+new", workspaceTemplate: "default", runAs: "admin" });
    // A field that doesn't apply is dropped, even if something was typed in it earlier.
    expect(formValues(form, { workspaceName: "old" })).toEqual({ workspace: "work", runAs: "admin" });
    expect(shown(form.fields[1], { workspace: "work" })).toBe(false);
    expect(shown(form.fields[1], { workspace: "+new" })).toBe(true);
  });

  it("reads like the backend's summary", () => {
    expect(summarize(form, "api", formValues(form, {}))).toEqual({ text: "Creates session api in workspace work on vm as admin.", creates: [] });
    const fresh = summarize(form, "api", formValues(form, { workspace: "+new" }));
    expect(fresh.text).toBe("Creates session api in a new workspace api on vm as admin.");
    expect(fresh.creates).toEqual(["workspace api (default)"]);
    expect(summarize(form, "api", formValues(form, { workspace: "+new", workspaceName: "scratch" })).creates).toEqual(["workspace scratch (default)"]);
    expect(summarize({ ...form, summary: null }, "x", {})).toEqual({ text: "", creates: [] });
  });

  it("checks names against the machine's rule", () => {
    const agw = { pattern: "^(?!.*--)[a-z0-9]([a-z0-9_-]*[a-z0-9])?$", maxLen: 34, hint: "agw rule" };
    expect(nameProblem(agw, "api-fix")).toBeNull();
    expect(nameProblem(agw, "Api")).toBe("agw rule");
    expect(nameProblem(agw, "a--b")).toBe("agw rule");
    expect(nameProblem(agw, "api-")).toBe("agw rule");
    expect(nameProblem(agw, "a".repeat(35))).toBe("At most 34 characters");
  });
});
