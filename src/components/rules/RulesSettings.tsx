import { useCallback, useEffect, useMemo, useState } from "react";
import { api, errorText } from "../../api";
import { useActions } from "../../lib/actions";
import { host } from "../../lib/host";
import { rulesApi, ruleLabel, type ImportKind, type RuleFile, type RuleSet, type RuleSource, type RulesStatus } from "../../rules/api";
import { refreshAgentRules } from "../../rules/useAgentRules";
import "./rules.css";

const IMPORT_HINT: Record<ImportKind, string> = {
  file: "/path/to/project/CLAUDE.md",
  project: "/path/to/project (with .rulesync/)",
  git: "https://github.com/team/ai-rules.git",
};

/** Settings → Rules: rulesync status, library, rule sets and project defaults. */
export function RulesSettings() {
  const { run } = useActions();
  const [status, setStatus] = useState<RulesStatus | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    rulesApi.status().then(setStatus, () => setStatus(null));
  }, []);

  const setNpx = async (on: boolean) => {
    const s = await run(rulesApi.setNpx(on), "change the npx setting");
    if (s) setStatus(s);
  };

  return (
    <div className="setting">
      <div className="setting-head">
        <span className="label">Rules</span>
        <span className="muted-sm">via rulesync</span>
        <span className="spacer" />
        {status === null ? (
          <span className="muted-sm">checking…</span>
        ) : status.via === "rulesync" ? (
          <span className="chip chip-ok">rulesync {status.version ?? ""}</span>
        ) : status.via === "npx" ? (
          <span className="chip chip-ok">npx rulesync</span>
        ) : (
          <span className="chip chip-subtle">not installed</span>
        )}
      </div>
      <p className="muted">
        Keep instructions in one library and give agents rule sets. Pitwall calls rulesync to generate each agent's files
        (CLAUDE.local.md, .claude/rules, AGENTS.md…) when it starts; they stay out of git.
      </p>
      {status && status.via !== "rulesync" && (
        <div className="confirm-box">
          <p>
            Install rulesync yourself: <span className="mono">npm install -g rulesync</span> — Pitwall never installs it.
          </p>
          <label className="check">
            <input
              type="checkbox"
              checked={status.npxAllowed}
              disabled={!status.npxFound && !status.npxAllowed}
              onChange={(e) => setNpx(e.target.checked)}
            />
            <span>
              Use <span className="mono">npx -y rulesync</span> when it isn't installed
              <span className="hint block">
                {status.npxFound ? "npx downloads rulesync on first use." : "npx isn't on your shell PATH."}
              </span>
            </span>
          </label>
        </div>
      )}
      <button className="small-btn" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        {open ? "Hide rules" : "Manage rules…"}
      </button>
      {open && <RulesManager available={!!status?.available} />}
    </div>
  );
}

function RulesManager({ available }: { available: boolean }) {
  const { run } = useActions();
  const [lib, setLib] = useState<RuleFile[]>([]);
  const [sets, setSets] = useState<RuleSet[]>([]);
  const [sources, setSources] = useState<RuleSource[]>([]);
  const [editing, setEditing] = useState<RuleSet | "new" | null>(null);

  const reload = useCallback(async () => {
    const [l, s, src] = await Promise.all([
      rulesApi.library().catch(() => [] as RuleFile[]),
      rulesApi.sets().catch(() => [] as RuleSet[]),
      rulesApi.sources().catch(() => [] as RuleSource[]),
    ]);
    setLib(l);
    setSets(s);
    setSources(src);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const changed = useCallback(async () => {
    await reload();
    void refreshAgentRules();
  }, [reload]);

  return (
    <div className="rules-manager">
      <section className="rules-section">
        <header className="rules-section-head">
          <span className="field-label">Library</span>
          <span className="label-count">{lib.length}</span>
          <span className="spacer" />
          <button className="ghost-btn rules-mini" onClick={() => run(rulesApi.revealLibrary(), "open the rules folder")}>
            Open folder
          </button>
          <button className="ghost-btn rules-mini" onClick={() => void changed()}>
            Refresh
          </button>
        </header>
        {lib.length === 0 ? (
          <p className="hint">
            Add rulesync rule files to <span className="mono">{host().dataDir}/rules/rules/</span>, or import below.
          </p>
        ) : (
          <ul className="rules-list">
            {lib.map((r) => (
              <li key={r.id} title={r.path}>
                <span className="mono rules-name">{ruleLabel(r.id)}</span>
                {r.source !== "library" && <span className="chip chip-subtle">{r.source}</span>}
                {r.root && <span className="chip" title="root: the agent's main instruction file">root</span>}
                {r.localRoot && <span className="chip">local</span>}
                <span className="rules-desc">{r.description ?? ""}</span>
              </li>
            ))}
          </ul>
        )}
        <ImportForm available={available} onDone={changed} />
        {sources.length > 0 && (
          <ul className="rules-list">
            {sources.map((s) => (
              <li key={s.name} title={s.root}>
                <span className="mono rules-name">{s.name}</span>
                <span className="chip chip-subtle">{s.kind}</span>
                <span className="rules-desc mono">{s.origin}</span>
                {s.kind === "git" && (
                  <button className="ghost-btn rules-mini" onClick={async () => (await run(rulesApi.pullSource(s.name), "pull rules")) !== undefined && changed()}>
                    Pull
                  </button>
                )}
                <button
                  className="ghost-btn rules-mini"
                  title={s.kind === "git" ? "Forget and delete Pitwall's clone" : "Forget (the project is not touched)"}
                  onClick={async () => (await run(rulesApi.removeSource(s.name), "remove source")) !== undefined && changed()}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="rules-section">
        <header className="rules-section-head">
          <span className="field-label">Rule sets</span>
          <span className="spacer" />
          {editing === null && (
            <button className="ghost-btn rules-mini" onClick={() => setEditing("new")} disabled={lib.length === 0}>
              New set
            </button>
          )}
        </header>
        {editing !== null ? (
          <SetEditor
            set={editing === "new" ? null : editing}
            lib={lib}
            onCancel={() => setEditing(null)}
            onSaved={async () => {
              setEditing(null);
              await changed();
            }}
          />
        ) : sets.length === 0 ? (
          <p className="hint">A rule set is a named pick of library rules, e.g. "Web defaults".</p>
        ) : (
          <ul className="rules-list">
            {sets.map((s) => (
              <li key={s.id}>
                <span className="rules-name">{s.name}</span>
                <span className="rules-desc">{s.ruleIds.map(ruleLabel).join(", ") || "empty"}</span>
                <button className="ghost-btn rules-mini" onClick={() => setEditing(s)}>
                  Edit
                </button>
                <button
                  className="ghost-btn rules-mini"
                  onClick={async () => {
                    if (!window.confirm(`Delete rule set "${s.name}"? Agents keep the files they have until rules are re-applied.`)) return;
                    if ((await run(rulesApi.deleteSet(s.id), "delete rule set")) !== undefined) await changed();
                  }}
                >
                  Delete
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      {sets.length > 0 && <ProjectDefaults sets={sets} onChanged={changed} />}
    </div>
  );
}

function ImportForm({ available, onDone }: { available: boolean; onDone(): void }) {
  const [kind, setKind] = useState<ImportKind>("file");
  const [source, setSource] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  const submit = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await rulesApi.importRules(kind, source.trim());
      setMsg({ ok: true, text: [`Imported ${r.added.length} rule${r.added.length === 1 ? "" : "s"}.`, r.log].filter(Boolean).join("\n") });
      setSource("");
      onDone();
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="rules-import"
      onSubmit={(e) => {
        e.preventDefault();
        if (source.trim() && !busy) void submit();
      }}
    >
      <div className="row gap">
        <select className="input rules-kind" value={kind} onChange={(e) => setKind(e.target.value as ImportKind)} aria-label="Import from">
          <option value="file">CLAUDE.md / AGENTS.md</option>
          <option value="project">Project's .rulesync</option>
          <option value="git">Git repository</option>
        </select>
        <input
          className="input mono"
          placeholder={IMPORT_HINT[kind]}
          value={source}
          onChange={(e) => setSource(e.target.value)}
          spellCheck={false}
          aria-label="Source"
        />
        <button className="small-btn" type="submit" disabled={!source.trim() || busy || (kind === "file" && !available)}>
          {busy ? "Importing…" : "Import"}
        </button>
      </div>
      <span className="hint">
        {kind === "file"
          ? "Runs rulesync import on a copy; your project is not touched."
          : kind === "project"
            ? "Read in place from the project's .rulesync/ (read-only)."
            : `Cloned into ${host().dataDir}/rules-sources; pull to update.`}
      </span>
      {msg && <pre className={msg.ok ? "rules-log" : "rules-log rules-log-error"}>{msg.text}</pre>}
    </form>
  );
}

function SetEditor({ set, lib, onCancel, onSaved }: { set: RuleSet | null; lib: RuleFile[]; onCancel(): void; onSaved(): void }) {
  const { run } = useActions();
  const [name, setName] = useState(set?.name ?? "");
  const [picked, setPicked] = useState<string[]>(set?.ruleIds ?? []);
  const missing = picked.filter((id) => !lib.some((r) => r.id === id));
  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]));

  const save = async () => {
    const s = await run(rulesApi.saveSet({ ...(set ? { id: set.id } : {}), name, ruleIds: picked }), "save rule set");
    if (s) onSaved();
  };

  return (
    <div className="rules-editor">
      <input className="input" placeholder="Set name" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
      <ul className="rules-pick">
        {lib.map((r) => (
          <li key={r.id}>
            <label className="check">
              <input type="checkbox" checked={picked.includes(r.id)} onChange={() => toggle(r.id)} />
              <span>
                <span className="mono">{ruleLabel(r.id)}</span>
                {r.source !== "library" && <span className="muted-sm"> · {r.source}</span>}
                {r.root && <span className="muted-sm"> · root</span>}
                {r.description && <span className="hint block">{r.description}</span>}
              </span>
            </label>
          </li>
        ))}
        {missing.map((id) => (
          <li key={id}>
            <label className="check">
              <input type="checkbox" checked onChange={() => toggle(id)} />
              <span className="hint-error mono">{ruleLabel(id)} (missing)</span>
            </label>
          </li>
        ))}
      </ul>
      <div className="row gap">
        <button className="primary-btn" onClick={save} disabled={!name.trim()}>
          Save
        </button>
        <button className="ghost-btn" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

function ProjectDefaults({ sets, onChanged }: { sets: RuleSet[]; onChanged(): void }) {
  const { run } = useActions();
  const [projects, setProjects] = useState<{ path: string; display: string }[]>([]);
  const [defaults, setDefaults] = useState<Record<string, string>>({});

  useEffect(() => {
    Promise.all([
      api.listProjects().catch(() => []),
      api.recentProjects().catch(() => []),
      rulesApi.projectRules().catch(() => ({}) as Record<string, string>),
    ]).then(([listed, recent, d]) => {
      const seen = new Map<string, string>();
      for (const p of [...listed, ...recent]) if (!seen.has(p.path)) seen.set(p.path, p.display);
      for (const path of Object.keys(d)) if (!seen.has(path)) seen.set(path, path);
      setProjects([...seen].map(([path, display]) => ({ path, display })));
      setDefaults(d);
    });
  }, []);

  const rows = useMemo(() => projects.slice(0, 40), [projects]);
  if (rows.length === 0) return null;

  return (
    <section className="rules-section">
      <header className="rules-section-head">
        <span className="field-label">Project defaults</span>
      </header>
      <p className="hint">Every new agent in the project gets this set. Rules reach new sessions only.</p>
      <ul className="rules-list">
        {rows.map((p) => (
          <li key={p.path} title={p.path}>
            <span className="rules-name rules-project">{p.display}</span>
            <span className="spacer" />
            <select
              className="input rules-kind"
              value={defaults[p.path] ?? ""}
              onChange={async (e) => {
                const id = e.target.value || null;
                if ((await run(rulesApi.setProjectRules(p.path, id), "set project rules")) === undefined) return;
                setDefaults((d) => {
                  const n = { ...d };
                  if (id) n[p.path] = id;
                  else delete n[p.path];
                  return n;
                });
                onChanged();
              }}
            >
              <option value="">None</option>
              {sets.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </li>
        ))}
      </ul>
    </section>
  );
}
