import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
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
import { Icon } from "./Icon";
import { accountsOfKind, folderName, projectAccount, useAccounts } from "../lib/useAccounts";

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
  /** "More options" (worktree, rules) shown. */
  const [more, setMore] = useState(false);
  /** The account picked here (undefined: the project's, else Default). */
  const [accountPick, setAccountPick] = useState<string | null | undefined>(undefined);
  const [accountForProject, setAccountForProject] = useState(false);
  const accounts = useAccounts();
  const { run, openSettings } = useActions();
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
  // The account: shown once the kind has one (where Pitwall starts it in a folder).
  const kindAccounts = folder ? accountsOfKind(accounts, kind) : [];
  const projectDefault = projectAccount(accounts, projectPath, kind) ?? null;
  const chosenAccount = accountPick === undefined ? projectDefault : accountPick;

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
        ...(kindAccounts.length ? { account: chosenAccount ?? "default" } : {}),
      });
      if (kindAccounts.length && accountForProject && chosenAccount !== projectDefault) {
        await api.setProjectAccount(projectPath, kind, chosenAccount);
      }
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
        onKeyDown={(e) => {
          // ⌘↵ starts it from anywhere in the dialog.
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            submit();
          }
        }}
      >
        <div className="modal-body form">
          {targets.length > 0 && (
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
              <span className="field-label">Project</span>
              <ProjectPicker
                recents={recents}
                value={projectPath}
                onPick={(path) => {
                  if (recents.some((r) => r.path === path)) setProjectChoice(path);
                  else {
                    setProjectChoice(OTHER);
                    setOtherPath(path);
                  }
                  setAccountPick(undefined);
                }}
              />
              {projectPath && projectError && <span className="field-error">{projectError}</span>}
            </div>
          )}

          {kindAccounts.length > 0 && (
            <div className="field">
              <span className="field-label">Account</span>
              <div className="acct-chips">
                {[{ id: null as string | null, name: "Default" }, ...kindAccounts.map((a) => ({ id: a.id as string | null, name: a.name }))].map((c) => (
                  <button
                    key={c.id ?? "default"}
                    type="button"
                    className="acct-chip"
                    data-on={chosenAccount === c.id}
                    onClick={() => {
                      setAccountPick(c.id);
                      setAccountForProject(false);
                    }}
                  >
                    {c.name}
                    {c.id !== null && c.id === projectDefault && <span className="acct-chip-tag">project</span>}
                  </button>
                ))}
                <button type="button" className="acct-chip acct-chip-add" title="Settings → Agents → Accounts" onClick={() => {
                  onClose();
                  openSettings("agents");
                }}>
                  + Add
                </button>
              </div>
              {projectPath && chosenAccount !== projectDefault && (
                <label className="check">
                  <input type="checkbox" checked={accountForProject} onChange={(e) => setAccountForProject(e.target.checked)} />
                  <span>Use it for new agents in {folderName(projectPath)}</span>
                </label>
              )}
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

          {folder && (worktreeSupported || !!selected?.caps?.rules) && (
            <div className="more-options">
              <button type="button" className="more-toggle" aria-expanded={more} onClick={() => setMore((m) => !m)}>
                <span className="chev" data-open={more}>
                  <Icon name="chevron" size={12} />
                </span>
                More options
                {!more && worktree && <span className="more-summary">· Separate worktree</span>}
              </button>
              {more && worktreeSupported && (
                <label className="check">
                  <input type="checkbox" checked={worktreeChecked} onChange={(e) => setWorktree(e.target.checked)} />
                  <span>
                    Separate worktree
                    <span className="hint block">Its own copy of the repo, so it doesn't clash with other agents in the same project.</span>
                  </span>
                </label>
              )}
              {more && <RulesField canRules={!!selected?.caps?.rules} projectPath={projectPath} worktree={worktree} value={rules} onChange={setRules} />}
            </div>
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
            {busy ? (folder ? "Starting…" : "Creating…") : (form?.submit ?? "Start")} <Kbd>⌘↵</Kbd>
          </button>
        </footer>
      </form>
    </Modal>
  );
}

/**
 * Project: a searchable picker (VS Code's quick pick). The field shows the folder's name and
 * path; opened, it lists a pasted path to use, "Choose folder…", then the projects that match
 * (name matches first).
 */
function ProjectPicker({ recents, value, onPick }: { recents: RecentProject[]; value: string; onPick(path: string): void }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  // Drawn over the dialog (not clipped by its scrolling body), under the field.
  const [at, setAt] = useState<{ left: number; top: number; width: number } | null>(null);
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const r = ref.current?.getBoundingClientRect();
      if (r) setAt({ left: r.left, top: r.bottom + 4, width: r.width });
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !popRef.current?.contains(t)) setOpen(false);
    };
    window.addEventListener("mousedown", close, true);
    return () => window.removeEventListener("mousedown", close, true);
  }, [open]);

  const q = query.trim();
  type Pick = { kind: "path"; path: string } | { kind: "choose" } | { kind: "recent"; r: RecentProject };
  const picks: Pick[] = [];
  if (q.startsWith("/") || q.startsWith("~")) picks.push({ kind: "path", path: q });
  picks.push({ kind: "choose" });
  const needle = q.toLowerCase();
  picks.push(
    ...recents
      .map((r) => {
        const name = folderName(r.path).toLowerCase();
        if (!needle) return [false, r] as const;
        if (name.includes(needle)) return [!name.startsWith(needle), r] as const;
        if (r.display.toLowerCase().includes(needle) || r.path.toLowerCase().includes(needle)) return [true, r] as const;
        return null;
      })
      .filter((x): x is readonly [boolean, RecentProject] => !!x)
      .sort((a, b) => Number(a[0]) - Number(b[0]))
      .map(([, r]) => ({ kind: "recent" as const, r })),
  );
  const cur = Math.min(active, picks.length - 1);
  const apply = (p: Pick) => {
    setOpen(false);
    setQuery("");
    if (p.kind === "path") onPick(p.path);
    else if (p.kind === "recent") onPick(p.r.path);
    // In the browser there is no folder dialog: paste a path instead.
    else if (canPickFolder) void pickFolder("Choose a project folder", value).then((f) => f && onPick(f));
  };
  const shown = recents.find((r) => r.path === value);
  return (
    <div className="picker" ref={ref}>
      <button type="button" className="picker-field" data-open={open} onClick={() => setOpen((o) => !o)} aria-haspopup="listbox" aria-expanded={open}>
        <Icon name="folder" size={16} />
        {value ? (
          <span className="picker-value">
            <span className="picker-name">{folderName(value)}</span>
            <span className="picker-path mono">{shown?.display ?? value}</span>
          </span>
        ) : (
          <span className="picker-value picker-empty">Choose a project folder…</span>
        )}
        <span className="picker-chev">
          <Icon name="chevron" size={12} />
        </span>
      </button>
      {open &&
        at &&
        createPortal(
        <div className="picker-pop" role="listbox" data-own-esc ref={popRef} style={{ left: at.left, top: at.top, width: at.width }}>
          <div className="picker-search">
            <Icon name="search" size={14} />
            <input
              className="picker-input"
              placeholder="Search projects, or paste a path"
              value={query}
              autoFocus
              spellCheck={false}
              onChange={(e) => {
                setQuery(e.target.value);
                setActive(0);
              }}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown") setActive(Math.min(cur + 1, picks.length - 1));
                else if (e.key === "ArrowUp") setActive(Math.max(cur - 1, 0));
                else if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) apply(picks[cur]);
                else if (e.key === "Escape") setOpen(false);
                else return;
                e.preventDefault();
                e.stopPropagation();
              }}
            />
          </div>
          <div className="picker-list">
            {picks.map((p, i) => (
              <button
                type="button"
                key={p.kind === "recent" ? p.r.path : p.kind}
                className="picker-row"
                data-on={i === cur}
                onMouseEnter={() => setActive(i)}
                onClick={() => apply(p)}
                title={p.kind === "choose" && !canPickFolder ? "No folder dialog in the browser demo: paste a path above" : undefined}
              >
                {p.kind === "choose" && (
                  <>
                    <Icon name="folder" size={14} />
                    <span className="picker-row-main">Choose folder…</span>
                    <span className="picker-row-hint">Browse</span>
                  </>
                )}
                {p.kind === "path" && (
                  <>
                    <Icon name="folder" size={14} />
                    <span>Use</span>
                    <span className="picker-row-path mono">{p.path}</span>
                  </>
                )}
                {p.kind === "recent" && (
                  <>
                    <span className="picker-check">{p.r.path === value && <Icon name="check" size={14} />}</span>
                    <span className="picker-row-name">{folderName(p.r.path)}</span>
                    <span className="picker-row-path mono">{p.r.display}</span>
                  </>
                )}
              </button>
            ))}
            {picks.length === 1 && q && <p className="picker-none">No projects match</p>}
          </div>
          <div className="picker-foot">↑↓ to move · ↵ to pick</div>
        </div>,
          document.body,
        )}
    </div>
  );
}
