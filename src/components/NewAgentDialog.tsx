import { useEffect, useMemo, useState } from "react";
import type { AgentView, CreateForm, KindView, NameRule, RecentProject } from "../types";
import { api, errorText } from "../api";
import { NAME_RE } from "../lib/status";
import { formValues, nameProblem, shown, summarize, type Values } from "../lib/createForm";
import { createTargets, FormField, type Target } from "./CreateFormFields";
import { newAgentSize } from "../terminal/registry";
import { Modal } from "./Modal";
import { Kbd } from "./Kbd";
import { useActions } from "../lib/actions";
import { canPickFolder, pickFolder } from "../lib/pickFolder";
import { NO_RULES, RulesField, reportRulesAfterCreate, rulesRequest, type RulesChoice } from "./rules/RulesField";

const OTHER = "__other__";

function slug(s: string): string {
  const base = s
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[^a-z]+/, "")
    .replace(/-+$/, "")
    .slice(0, 28);
  return base || "agent";
}

function suggestName(projectPath: string, taken: Set<string>): string {
  const base = slug(projectPath.split("/").filter(Boolean).pop() ?? "agent");
  if (!taken.has(base)) return base;
  for (let i = 2; ; i++) if (!taken.has(`${base}-${i}`)) return `${base}-${i}`;
}

/** Pitwall's own name rule, until the machine's form says otherwise. */
const PITWALL_NAME: NameRule = {
  pattern: NAME_RE.source,
  maxLen: 32,
  hint: "Lowercase letters, digits, - or _; starts with a letter; max 32",
};

interface Props {
  existing: AgentView[];
  /** Preselect this project folder. */
  initialProject?: string;
  onClose(): void;
  onCreated(a: AgentView): void;
}

export function NewAgentDialog({ existing, initialProject, onClose, onCreated }: Props) {
  const [kinds, setKinds] = useState<KindView[] | null>(null);
  const [recents, setRecents] = useState<RecentProject[]>([]);
  const [kind, setKind] = useState<string>("");
  const [customCommand, setCustomCommand] = useState("");
  const [projectChoice, setProjectChoice] = useState<string>(OTHER);
  const [otherPath, setOtherPath] = useState("");
  const [name, setName] = useState("");
  const [nameTouched, setNameTouched] = useState(false);
  const [worktreeChecked, setWorktree] = useState(false);
  const [rules, setRules] = useState<RulesChoice>(NO_RULES);
  const { run } = useActions();
  const [busy, setBusy] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  const taken = useMemo(() => new Set(existing.map((a) => a.name)), [existing]);
  const [targets, setTargets] = useState<Target[]>([]);
  const [targetKey, setTargetKey] = useState("");
  const [form, setForm] = useState<CreateForm | null>(null);
  const [formLoading, setFormLoading] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Values>({});

  useEffect(() => {
    api
      .listMachines()
      .then((list) => {
        const t = createTargets(list);
        setTargets(t);
        setTargetKey((k) => k || t[0]?.key || "");
      })
      .catch(() => setTargets([]));
  }, []);

  const target = targets.find((t) => t.key === targetKey);
  useEffect(() => {
    if (!target) return;
    let live = true;
    setFormLoading(true);
    setFormError(null);
    setChosen({});
    api
      .createForm(target.provider, target.machine)
      .then((f) => live && setForm(f))
      .catch((e) => {
        if (!live) return;
        setForm(null);
        setFormError(errorText(e));
      })
      .finally(() => live && setFormLoading(false));
    return () => {
      live = false;
    };
  }, [target]);

  // This Mac's form until the chosen machine's arrives: a kind and a folder.
  const folder = form?.folder ?? true;
  const nameRule = form?.name ?? PITWALL_NAME;
  const values = useMemo(() => (form && !folder ? formValues(form, chosen) : {}), [form, folder, chosen]);
  const summary = form && !folder ? summarize(form, name, values) : null;

  useEffect(() => {
    Promise.all([
      api.listKinds().catch(() => [] as KindView[]),
      api.recentProjects().catch(() => [] as RecentProject[]),
      api.listProjects().catch(() => []),
    ]).then(([k, recent, listed]) => {
      // Recent conversations first, then the user's project list.
      const r = [
        ...recent,
        ...listed
          .filter((p) => !recent.some((x) => x.path === p.path))
          .map((p) => ({ path: p.path, display: p.display, lastUsed: 0 })),
      ];
      setKinds(k);
      setRecents(r);
      const firstInstalled = k.find((x) => x.installed && !x.caps?.customCommand) ?? k.find((x) => x.installed);
      setKind(firstInstalled?.id ?? "");
      if (initialProject && r.some((x) => x.path === initialProject)) setProjectChoice(initialProject);
      else if (initialProject) setOtherPath(initialProject);
      else if (r.length) setProjectChoice(r[0].path);
    });
  }, [initialProject]);

  const projectPath = projectChoice === OTHER ? otherPath.trim() : projectChoice;
  const selected = kinds?.find((k) => k.id === kind);
  // Only kinds whose CLI has its own worktree flag, where the provider can
  // find it (docs/spec/worktrees.md).
  const worktreeSupported = !!selected?.caps?.worktree;
  const customCommandKind = !!selected?.caps?.customCommand;
  const worktree = worktreeSupported && worktreeChecked;

  useEffect(() => {
    if (!nameTouched && folder && projectPath) setName(suggestName(projectPath, taken));
  }, [projectPath, nameTouched, taken, folder]);

  const kindOptions: KindView[] = kinds ?? [];

  const nameError = !name
    ? "Name required"
    : nameProblem(nameRule, name)
      ? nameProblem(nameRule, name)
      : taken.has(name)
        ? "Another agent already has this name"
        : null;
  const projectError = !folder
    ? null
    : !projectPath
      ? "Pick a project folder"
      : !projectPath.startsWith("/") && !projectPath.startsWith("~")
        ? "Use an absolute path"
        : null;
  const commandError = folder && customCommandKind && !customCommand.trim() ? "Enter a command" : null;
  const fieldsError =
    form && !folder
      ? form.fields
          .filter((f) => shown(f, values))
          .map((f) =>
            f.input === "select" ? (values[f.id] ? null : `Choose ${f.label}`) : values[f.id] && f.rule ? nameProblem(f.rule, values[f.id]) : null,
          )
          .find((e) => !!e) ?? null
      : null;
  const valid = !nameError && !projectError && !commandError && !fieldsError && !formLoading && !formError && (!folder || !!kind);

  const submit = async () => {
    if (!valid || busy) return;
    setBusy(true);
    setSubmitError(null);
    const where = target ? { provider: target.provider, machine: target.machine } : {};
    try {
      if (!folder) {
        // The platform decides what runs and where (the form's fields); the button is the user's go-ahead.
        const a = await api.createAgent({ name, kind: "", projectPath: "", worktree: false, ...where, options: values, ...(await newAgentSize()) });
        onCreated(a);
        return;
      }
      const a = await api.createAgent({
        name,
        kind,
        projectPath,
        worktree,
        ...where,
        ...(await newAgentSize()),
        ...(customCommandKind ? { customCommand } : {}),
        ...rulesRequest(rules, worktree),
      });
      onCreated(a);
      void reportRulesAfterCreate(a.id, run);
    } catch (e) {
      setSubmitError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <Modal title="New agent" onClose={onClose} width={520}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <div className="modal-body form">
          {targets.length > 1 && (
            <div className="field">
              <label className="field-label" htmlFor="na-target">
                Runs on
              </label>
              <select id="na-target" className="input" value={targetKey} onChange={(e) => setTargetKey(e.target.value)} disabled={busy}>
                {targets.map((t) => (
                  <option key={t.key} value={t.key}>
                    {t.label}
                  </option>
                ))}
              </select>
              {formLoading && <span className="hint">Reading what {target?.label ?? "it"} has…</span>}
              {formError && <span className="field-error">{formError}</span>}
              {form?.error && <span className="hint">Some choices couldn't be listed: {form.error}</span>}
            </div>
          )}

          {folder && (
          <div className="field">
            <span className="field-label">Agent</span>
            {kinds === null ? (
              <p className="hint">Looking for installed agents…</p>
            ) : (
              <div className="kind-grid" role="radiogroup">
                {kindOptions.map((k) => (
                  <label
                    key={k.id}
                    className="kind-card"
                    data-checked={kind === k.id}
                    data-disabled={!k.installed}
                    title={k.installed ? k.path : `${k.name} isn't installed (not found on your shell PATH)`}
                  >
                    <input
                      type="radio"
                      name="kind"
                      value={k.id}
                      checked={kind === k.id}
                      disabled={!k.installed}
                      onChange={() => setKind(k.id)}
                    />
                    <span className="kind-name">{k.name}</span>
                    <span className="kind-sub">{k.caps?.customCommand ? "any CLI" : k.installed ? "installed" : "not installed"}</span>
                  </label>
                ))}
              </div>
            )}
            {customCommandKind && (
              <input
                className="input mono"
                placeholder="e.g. aider --model sonnet"
                value={customCommand}
                onChange={(e) => setCustomCommand(e.target.value)}
                autoFocus
                spellCheck={false}
              />
            )}
          </div>
          )}

          {folder && (
          <div className="field">
            <label className="field-label" htmlFor="na-project">
              Project
            </label>
            <select
              id="na-project"
              className="input"
              value={projectChoice}
              onChange={(e) => {
                setProjectChoice(e.target.value);
                // "Choose folder…": native picker; the typed path below stays as fallback.
                if (e.target.value === OTHER && canPickFolder) pickFolder().then((p) => p && setOtherPath(p));
              }}
            >
              {recents.map((r) => (
                <option key={r.path} value={r.path}>
                  {r.display}
                </option>
              ))}
              <option value={OTHER}>Choose folder…</option>
            </select>
            {projectChoice === OTHER && (
              <div className="row gap">
                <input
                  className="input mono"
                  style={{ flex: 1 }}
                  placeholder="/Users/you/code/project"
                  value={otherPath}
                  onChange={(e) => setOtherPath(e.target.value)}
                  spellCheck={false}
                  autoFocus={recents.length > 0}
                />
                {canPickFolder && (
                  <button
                    type="button"
                    className="small-btn"
                    onClick={() => pickFolder("Choose a project folder", otherPath).then((p) => p && setOtherPath(p))}
                  >
                    Browse…
                  </button>
                )}
              </div>
            )}
            {projectChoice === OTHER && otherPath && projectError && <span className="field-error">{projectError}</span>}
          </div>
          )}

          <div className="field">
            <label className="field-label" htmlFor="na-name">
              Name
            </label>
            <input
              id="na-name"
              className="input mono"
              value={name}
              onChange={(e) => {
                setNameTouched(true);
                setName(e.target.value);
              }}
              spellCheck={false}
              autoComplete="off"
              maxLength={nameRule.maxLen}
              data-autofocus
            />
            {name && nameError ? (
              <span className="field-error">{nameError}</span>
            ) : (
              !folder && form && <span className="hint">Also its name on {form.machineLabel}. {nameRule.hint}</span>
            )}
          </div>

          {form &&
            !folder &&
            form.fields
              .filter((f) => shown(f, values))
              .map((f) => (
                <FormField
                  key={f.id}
                  field={f}
                  value={chosen[f.id] ?? (f.input === "select" ? (values[f.id] ?? "") : "")}
                  onChange={(v) => setChosen((c) => ({ ...c, [f.id]: v }))}
                />
              ))}

          {summary && name && !nameError && (
            <div className="create-summary" role="status">
              <p>{summary.text}</p>
              {summary.creates.length > 0 && (
                <ul className="create-extras">
                  {summary.creates.map((c) => (
                    <li key={c}>
                      Also creates {c}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}

          {folder && worktreeSupported && (
            <label className="check">
              <input type="checkbox" checked={worktreeChecked} onChange={(e) => setWorktree(e.target.checked)} />
              <span>
                Separate worktree
                <span className="hint block">
                  Separate copy of the repo, so this agent doesn't clash with others in the same project.
                </span>
              </span>
            </label>
          )}

          {folder && (
            <RulesField canRules={!!selected?.caps?.rules} projectPath={projectPath} worktree={worktree} value={rules} onChange={setRules} />
          )}

          {busy && !folder && form && (
            <p className="hint" role="status">
              Creating on {form.machineLabel}… a new workspace or agent user can take a few minutes.
            </p>
          )}
          {submitError && <p className="form-error">{submitError}</p>}
        </div>
        <footer className="modal-foot">
          <button type="button" className="ghost-btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="primary-btn" disabled={!valid || busy}>
            {busy ? (folder ? "Starting…" : "Creating…") : (form?.submit ?? "Start")} <Kbd>↵</Kbd>
          </button>
        </footer>
      </form>
    </Modal>
  );
}
