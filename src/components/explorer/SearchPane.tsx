import { useEffect, useMemo, useRef, useState } from "react";
import type { AgentView } from "../../types";
import { api, errorText } from "../../api";
import { SEARCH_CANCELLED, searchQuery, type SearchResult } from "../../explorerApi";
import { globList, groupMatches, matchRuns } from "../../lib/explorer";
import { splitPath } from "../../lib/fileTree";
import { FileIcon } from "../FileIcon";
import { Icon } from "../Icon";

export interface SearchOpen {
  path: string;
  line: number;
  /** 0-based columns of the first match on that line. */
  from: number;
  to: number;
}

interface Form {
  query: string;
  caseSensitive: boolean;
  wholeWord: boolean;
  regex: boolean;
  include: string;
  exclude: string;
  /** Leave out node_modules and bower_components (VS Code's default search.exclude). */
  defaultExcludes: boolean;
  details: boolean;
}

interface Saved {
  form: Form;
  result: SearchResult | null;
  error: string | null;
  collapsed: Record<string, boolean>;
}

const EMPTY_FORM: Form = { query: "", caseSensitive: false, wholeWord: false, regex: false, include: "", exclude: "", defaultExcludes: true, details: false };

// What each agent's search showed, while the window lives (closing the viewer keeps it).
const saved = new Map<string, Saved>();

/** ⇧⌘F: search in the agent's folder (ripgrep, else git grep, on its machine). */
export function SearchPane({ agent, focusNonce, onOpen }: { agent: AgentView; focusNonce: number; onOpen(hit: SearchOpen): void }) {
  const prev = saved.get(agent.id);
  const [form, setForm] = useState<Form>(prev?.form ?? EMPTY_FORM);
  const [result, setResult] = useState<SearchResult | null>(prev?.result ?? null);
  const [error, setError] = useState<string | null>(prev?.error ?? null);
  const [busy, setBusy] = useState(false);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>(prev?.collapsed ?? {});
  const input = useRef<HTMLInputElement>(null);
  const seq = useRef(0);

  useEffect(() => {
    saved.set(agent.id, { form, result, error, collapsed });
  }, [agent.id, form, result, error, collapsed]);

  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, [focusNonce]);

  // Searches as you type (debounced); a newer search cancels the one running.
  const [again, setAgain] = useState(0);
  const key = JSON.stringify([form.query, form.caseSensitive, form.wholeWord, form.regex, form.include, form.exclude, form.defaultExcludes, again]);
  const lastKey = useRef(prev ? key : "");
  useEffect(() => {
    if (key === lastKey.current) return;
    lastKey.current = key;
    const n = ++seq.current;
    if (!form.query) {
      setResult(null);
      setError(null);
      setBusy(false);
      void api.cancelSearch(agent.id).catch(() => {});
      return;
    }
    const t = setTimeout(() => {
      setBusy(true);
      api
        .searchFiles(
          agent.id,
          searchQuery(form.query, {
            caseSensitive: form.caseSensitive,
            wholeWord: form.wholeWord,
            regex: form.regex,
            include: globList(form.include),
            exclude: globList(form.exclude),
            defaultExcludes: form.defaultExcludes,
          }),
        )
        .then(
          (r) => {
            if (n !== seq.current) return;
            setResult(r);
            setError(null);
            setCollapsed({});
          },
          (e) => {
            const msg = errorText(e);
            if (n !== seq.current || msg === SEARCH_CANCELLED) return;
            setError(msg);
            setResult(null);
          },
        )
        .finally(() => n === seq.current && setBusy(false));
    }, 300);
    return () => clearTimeout(t);
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps

  // Leaving the pane (or the viewer) stops a search still running.
  useEffect(() => () => void api.cancelSearch(agent.id).catch(() => {}), [agent.id]);

  const groups = useMemo(() => groupMatches(result?.matches ?? []), [result]);
  const set = (p: Partial<Form>) => setForm((f) => ({ ...f, ...p }));
  const toggle = (k: "caseSensitive" | "wholeWord" | "regex", label: string, text: React.ReactNode) => (
    <button
      type="button"
      className="ex-opt"
      aria-pressed={form[k]}
      title={label}
      aria-label={label}
      onClick={() => set({ [k]: !form[k] })}
    >
      {text}
    </button>
  );

  const total = result?.matches.length ?? 0;
  const summary = !result
    ? null
    : total === 0
      ? "No results."
      : `${total} result${total === 1 ? "" : "s"} in ${result.files} file${result.files === 1 ? "" : "s"}${result.truncated ? " — stopped there, narrow the search" : ""}`;

  return (
    <div className="ex-search">
      <div className="ex-search-form">
        <div className="ex-search-row">
          <button
            type="button"
            className="icon-btn icon-btn-sm ex-details"
            aria-expanded={form.details}
            title="Files to include / exclude"
            aria-label="Files to include / exclude"
            onClick={() => set({ details: !form.details })}
          >
            <span className="chev" data-open={form.details}>
              <Icon name="chevron" size={10} />
            </span>
          </button>
          <div className="ex-search-box">
            <input
              ref={input}
              className="ex-search-input"
              placeholder="Search"
              value={form.query}
              spellCheck={false}
              aria-label={`Search in ${agent.name}'s files`}
              onChange={(e) => set({ query: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === "Enter") setAgain((n) => n + 1);
              }}
            />
            {toggle("caseSensitive", "Match case", "Aa")}
            {toggle("wholeWord", "Match whole word", <u>ab</u>)}
            {toggle("regex", "Use regular expression", ".*")}
          </div>
        </div>
        {form.details && (
          <div className="ex-search-globs">
            <label className="ex-glob">
              <span className="label">files to include</span>
              <input className="input" value={form.include} placeholder="e.g. *.ts, src/**" spellCheck={false} onChange={(e) => set({ include: e.target.value })} />
            </label>
            <label className="ex-glob">
              <span className="label">files to exclude</span>
              <input className="input" value={form.exclude} placeholder="e.g. *.test.ts" spellCheck={false} onChange={(e) => set({ exclude: e.target.value })} />
            </label>
            <label className="ex-check">
              <input type="checkbox" checked={form.defaultExcludes} onChange={(e) => set({ defaultExcludes: e.target.checked })} />
              Leave out node_modules and bower_components
            </label>
            <p className="hint">.gitignore applies; .git is never searched.</p>
          </div>
        )}
      </div>
      <div className="ex-search-sum" aria-live="polite">
        {busy ? "Searching…" : error ? null : summary}
      </div>
      {error && (
        <p className="hint hint-error ex-search-error" role="alert" title={error}>
          {error.split("\n")[0]}
        </p>
      )}
      <ul className="file-list ex-results" aria-label="Search results">
        {groups.map((g) => {
          const open = !collapsed[g.path];
          const { dir, base } = splitPath(g.path);
          return (
            <li key={g.path}>
              <button
                className="file-row ex-result-file"
                aria-expanded={open}
                title={g.path}
                onClick={() => setCollapsed((c) => ({ ...c, [g.path]: open }))}
              >
                <span className="chev" data-open={open}>
                  <Icon name="chevron" size={10} />
                </span>
                <FileIcon path={g.path} />
                <span className="file-path">
                  <span className="file-base">{base}</span>
                  <span className="file-dir"> {dir.replace(/\/$/, "")}</span>
                </span>
                <span className="ex-count">{g.matches.length}</span>
              </button>
              {open && (
                <ul className="rv-tree-group">
                  {g.matches.map((m) => (
                    <li key={`${m.line}:${m.column}`}>
                      <button
                        className="file-row ex-match"
                        title={`${g.path}:${m.line}`}
                        onClick={() => {
                          const r = m.ranges[0];
                          const from = m.textOffset + (r?.start ?? m.column - 1);
                          onOpen({ path: g.path, line: m.line, from, to: m.textOffset + (r?.end ?? m.column - 1) });
                        }}
                      >
                        <span className="ex-match-line">{m.line}</span>
                        <span className="ex-match-text">
                          {m.textOffset > 0 && "…"}
                          {matchRuns(m).map((run, i) => (run.hit ? <mark key={i}>{run.text}</mark> : <span key={i}>{run.text}</span>))}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
