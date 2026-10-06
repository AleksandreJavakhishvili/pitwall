// The New-agent dialog's machine parts, as data from the backend: where new
// agents can start ("Runs on", `list_machines`) and a machine's own fields
// (`create_form`). Nothing here knows which provider it is (architecture.md §3).
import type { CreateField, ProviderMachines } from "../types";
import { nameProblem } from "../lib/createForm";

/** A machine new agents can be made on ("Runs on"). */
export interface Target {
  key: string;
  provider: string;
  machine: string;
  label: string;
}

/** Machines where new agents can start, in the providers' order. */
export function createTargets(list: ProviderMachines[]): Target[] {
  return list
    .filter((p) => p.canCreate)
    .flatMap((p) =>
      p.machines.map((m) => ({
        key: `${p.provider}:${m.id}`,
        provider: p.provider,
        machine: m.id,
        label: m.label === p.label ? m.label : `${p.label} · ${m.label}`,
      })),
    );
}

/** One field of a machine's form (a select or a name). */
export function FormField({ field, value, onChange }: { field: CreateField; value: string; onChange(v: string): void }) {
  const id = `na-f-${field.id}`;
  const problem = field.input === "text" && value && field.rule ? nameProblem(field.rule, value) : null;
  return (
    <div className="field">
      <label className="field-label" htmlFor={id}>
        {field.label}
      </label>
      {field.input === "select" ? (
        <select id={id} className="input" value={value} onChange={(e) => onChange(e.target.value)}>
          {field.choices.map((c) => (
            <option key={c.value} value={c.value}>
              {c.detail ? `${c.label} — ${c.detail}` : c.label}
            </option>
          ))}
        </select>
      ) : (
        <input
          id={id}
          className="input mono"
          value={value}
          placeholder={field.placeholder ?? undefined}
          maxLength={field.rule?.maxLen}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
      )}
      {problem ? <span className="field-error">{problem}</span> : field.hint && <span className="hint">{field.hint}</span>}
    </div>
  );
}
