// "Runs on" and a machine's fields come from the backend as data
// (`list_machines`, `create_form`); the dialog renders whatever they say.
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { CreateField, ProviderMachines } from "../types";
import { createTargets, FormField } from "./CreateFormFields";

describe("new agent dialog", () => {
  it("offers the machines where new agents can start", () => {
    const list: ProviderMachines[] = [
      { provider: "p1", label: "This Mac", version: null, canCreate: true, canAddSessions: false, machines: [{ id: "here", label: "This Mac", detail: null }], error: null },
      { provider: "p2", label: "vms", version: "1", canCreate: true, canAddSessions: true, machines: [{ id: "a", label: "alpha", detail: null }, { id: "b", label: "beta", detail: null }], error: null },
      { provider: "p3", label: "ro", version: null, canCreate: false, canAddSessions: true, machines: [{ id: "x", label: "x", detail: null }], error: null },
    ];
    expect(createTargets(list).map((t) => [t.key, t.label])).toEqual([
      ["p1:here", "This Mac"],
      ["p2:a", "vms · alpha"],
      ["p2:b", "vms · beta"],
    ]);
  });

  it("renders a field's choices and its name rule", () => {
    const select: CreateField = {
      id: "workspace",
      label: "Workspace",
      input: "select",
      choices: [
        { value: "work", label: "work", detail: "template agentworks", phrase: null, creates: null },
        { value: "+new", label: "New workspace…", detail: null, phrase: null, creates: "workspace {workspaceName}" },
      ],
      default: "work",
      placeholder: null,
      hint: "Where it works",
      when: null,
      defaultsToName: false,
      rule: null,
    };
    const html = renderToStaticMarkup(<FormField field={select} value="work" onChange={() => {}} />);
    expect(html).toContain("Workspace");
    expect(html).toContain('<option value="work" selected="">work — template agentworks</option>');
    expect(html).toContain('<option value="+new">New workspace…</option>');
    expect(html).toContain("Where it works");
    const text: CreateField = { ...select, id: "workspaceName", label: "Workspace name", input: "text", choices: [], placeholder: "same as the session", hint: null, rule: { pattern: "^[a-z]+$", maxLen: 29, hint: "letters only" } };
    expect(renderToStaticMarkup(<FormField field={text} value="" onChange={() => {}} />)).toContain('placeholder="same as the session"');
    const bad = renderToStaticMarkup(<FormField field={text} value="Bad!" onChange={() => {}} />);
    expect(bad).toContain("letters only");
    expect(bad).toContain('maxLength="29"');
  });
});
