import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import type { AgentView } from "../../types";
import { api, errorText } from "../../api";
import { LARGE_CAP, TEXT_CAP, type FileView } from "../../explorerApi";
import type { ExplorerTarget } from "../../lib/actions";
import { absolutePath, findEntry, sizeLabel } from "../../lib/explorer";
import { STATUS_TITLE, splitPath } from "../../lib/fileTree";
import { useRefreshRequest } from "../../lib/freshness";
import { keys } from "../../lib/host";
import { StatusGlyph } from "../StatusGlyph";
import { FileIcon } from "../FileIcon";
import { Icon } from "../Icon";
import { Kbd } from "../Kbd";
import { RefreshControl } from "../Freshness";
import { ExplorerTree } from "./ExplorerTree";
import { IgnoredToggle } from "./FilesPanel";
import { SearchPane } from "./SearchPane";
import { noteRecent } from "./QuickOpen";
import { useShowIgnored, useTree } from "./useTree";
import type { Reveal } from "./FileEditor";
import "../review/review.css";
import "./explorer.css";

// The editor (CodeMirror, shared with Review's chunk) loads only when a text file is shown.
const FileEditor = lazy(() => import("./FileEditor"));

/** What the viewer is asked to show (a new `nonce`: show it again). */
export interface ViewerTarget extends ExplorerTarget {
  nonce: number;
}

interface Doc {
  view: FileView | null;
  error: string | null;
  loading: boolean;
  /** Read again and different: what's on disk now (not shown until "Reload"). */
  newer: FileView | null;
}

// Open tabs per agent while the window lives (closing the viewer keeps them).
const sessions = new Map<string, { tabs: string[]; active: string | null }>();

const differs = (a: FileView | null, b: FileView) => !a || a.size !== b.size || a.kind !== b.kind || a.text !== b.text;

/**
 * The read-only file viewer (docs/spec/explorer.md): full main area like
 * Review. Left: the agent's tree or Search; center: tabs of open files, each
 * in Review's read-only CodeMirror. Nothing here can change a file.
 */
export default function Viewer({
  agent,
  target,
  onExit,
  onShowDiff,
  onQuickOpen,
}: {
  agent: AgentView;
  target: ViewerTarget;
  onExit(): void;
  onShowDiff(path: string): void;
  onQuickOpen(): void;
}) {
  const [ignored, setIgnored] = useShowIgnored();
  const { state, model } = useTree(agent, ignored);
  const saved = sessions.get(agent.id);
  const [tabs, setTabs] = useState<string[]>(saved?.tabs ?? []);
  const [active, setActive] = useState<string | null>(saved?.active ?? null);
  const [docs, setDocs] = useState<Record<string, Doc>>({});
  const [reveal, setReveal] = useState<Reveal | null>(null);
  const [pane, setPane] = useState<"files" | "search">(target.pane ?? "files");
  const [searchFocus, setSearchFocus] = useState(0);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    sessions.set(agent.id, { tabs, active });
  }, [agent.id, tabs, active]);

  const read = useCallback(
    (path: string, opts: { large?: boolean; quiet?: boolean } = {}) => {
      if (!opts.quiet) setDocs((d) => ({ ...d, [path]: { view: d[path]?.view ?? null, error: null, loading: true, newer: null } }));
      api.readFile(agent.id, path, opts.large).then(
        (v) =>
          setDocs((d) => {
            const cur = d[path];
            // A quiet re-read keeps what's shown and offers the newer text.
            if (opts.quiet && cur?.view) return differs(cur.view, v) ? { ...d, [path]: { ...cur, newer: v } } : d;
            return { ...d, [path]: { view: v, error: null, loading: false, newer: null } };
          }),
        (e) =>
          setDocs((d) => (opts.quiet && d[path]?.view ? d : { ...d, [path]: { view: null, error: errorText(e), loading: false, newer: null } })),
      );
    },
    [agent.id],
  );

  const open = useCallback(
    (path: string, at?: { line: number; from?: number; to?: number }) => {
      setTabs((t) => (t.includes(path) ? t : [...t, path]));
      setActive(path);
      noteRecent(agent.id, path);
      void model.reveal(path);
      setReveal(at ? { ...at, nonce: Date.now() } : null);
    },
    [agent.id, model],
  );

  // Asked from outside (tree in the panel, ⌘P, ⇧⌘F, a search hit).
  useEffect(() => {
    if (target.path) open(target.path, target.line ? { line: target.line, from: target.from, to: target.to } : undefined);
    if (target.pane) setPane(target.pane);
    if (target.pane === "search") setSearchFocus((n) => n + 1);
  }, [target.nonce]); // eslint-disable-line react-hooks/exhaustive-deps

  // The active file is read when first shown.
  useEffect(() => {
    if (active && !docs[active]) read(active);
  }, [active, docs, read]);

  // ↻ / ⌘⇧R, the agent's changes moving: read the shown file again quietly.
  const recheck = () => {
    const shown = active ? docs[active]?.view : null;
    // A file loaded anyway stays loaded that way.
    if (active && shown) read(active, { quiet: true, large: shown.kind === "text" && shown.size > TEXT_CAP });
  };
  useRefreshRequest(recheck);
  const sig = `${agent.added}:${agent.removed}:${agent.filesChanged}`;
  const firstSig = useRef(true);
  useEffect(() => {
    if (firstSig.current) {
      firstSig.current = false;
      return;
    }
    recheck();
  }, [sig]); // eslint-disable-line react-hooks/exhaustive-deps

  const close = (path: string) => {
    setTabs((t) => {
      const i = t.indexOf(path);
      const next = t.filter((p) => p !== path);
      if (active === path) setActive(next[Math.min(i, next.length - 1)] ?? null);
      return next;
    });
    // Closed files drop their documents.
    setDocs((d) => {
      const n = { ...d };
      delete n[path];
      return n;
    });
  };

  // Esc leaves the viewer (unless a dialog, the editor or a field has it), as in Review.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if (document.querySelector(".backdrop")) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.(".cm-editor, textarea, input, select, .rv-menu")) return;
      e.preventDefault();
      onExit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onExit]);

  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1200);
    return () => clearTimeout(t);
  }, [copied]);

  const doc = active ? docs[active] : undefined;
  const entry = active ? findEntry(state, active) : null;
  const status = entry?.status ?? null;
  const v = doc?.view ?? null;

  let body: React.ReactNode;
  if (!active) {
    body = (
      <div className="rv-empty ex-empty">
        <span>Pick a file on the left</span>
        <button className="small-btn" onClick={onQuickOpen}>
          Go to file <Kbd>⌘P</Kbd>
        </button>
      </div>
    );
  } else if (doc?.error) {
    body = (
      <p className="hint hint-error pad fresh-error" role="alert">
        <span title={doc.error}>Couldn't read {active}: {doc.error.split("\n")[0]}</span>
        <button type="button" className="retry-btn" onClick={() => read(active)}>
          Retry
        </button>
      </p>
    );
  } else if (!v || (doc?.loading && v.path !== active)) {
    body = <p className="hint pad">Reading…</p>;
  } else if (v.kind === "binary") {
    body = <div className="rv-empty">Binary file · {sizeLabel(v.size)} · not shown</div>;
  } else if (v.kind === "tooLarge") {
    body = (
      <div className="rv-empty ex-empty">
        <span>
          Large file · {sizeLabel(v.size)} · not opened
        </span>
        {v.size <= LARGE_CAP ? (
          <button className="small-btn" disabled={doc?.loading} onClick={() => read(active, { large: true })}>
            {doc?.loading ? "Reading…" : "Load anyway"}
          </button>
        ) : (
          <span className="hint">Files over {sizeLabel(LARGE_CAP)} aren't shown.</span>
        )}
      </div>
    );
  } else {
    body = (
      <Suspense fallback={<p className="hint pad">Loading editor…</p>}>
        <FileEditor path={active} text={v.text ?? ""} reveal={reveal} />
      </Suspense>
    );
  }

  return (
    <div className="review explorer">
      <div className="review-bar">
        <span className="label">Files</span>
        <span className="ex-bar-agent">
          <StatusGlyph status={agent.status} size="sm" />
          <span className="rv-agent-label">{agent.name}</span>
        </span>
        <span className="muted-sm ex-bar-hint" title={agent.cwd}>
          read-only · {agent.cwdDisplay}
        </span>
        <span className="spacer" />
        <RefreshControl refreshing={state.refreshing} updatedAt={state.updatedAt} onRefresh={() => (void model.refresh(), recheck())} label="Refresh files" />
        <button className="small-btn" onClick={onQuickOpen} title={keys("Go to file (⌘P)")}>
          Go to file <Kbd>⌘P</Kbd>
        </button>
        <button className="small-btn" onClick={onExit}>
          Back <Kbd>esc</Kbd>
        </button>
      </div>

      <div className="review-body">
        <aside className="rv-list ex-side" aria-label={`Files of ${agent.name}`}>
          <div className="ex-side-head">
            <div className="rv-seg" role="group" aria-label="Side pane">
              <button aria-pressed={pane === "files"} onClick={() => setPane("files")}>
                Explorer
              </button>
              <button
                aria-pressed={pane === "search"}
                title={keys("Search in files (⌘⇧F)")}
                onClick={() => {
                  setPane("search");
                  setSearchFocus((n) => n + 1);
                }}
              >
                Search
              </button>
            </div>
            <span className="spacer" />
            {pane === "files" && <IgnoredToggle on={ignored} onChange={setIgnored} />}
          </div>
          {pane === "files" ? (
            <ExplorerTree state={state} model={model} selected={active} onOpen={(p) => open(p)} label={`Files of ${agent.name}`} />
          ) : (
            <SearchPane agent={agent} focusNonce={searchFocus} onOpen={(hit) => open(hit.path, hit)} />
          )}
        </aside>

        <section className="rv-main">
          {tabs.length > 0 && (
            <div className="ex-tabs" role="tablist" aria-label="Open files">
              {tabs.map((p) => {
                const s = findEntry(state, p)?.status ?? null;
                return (
                  <div
                    key={p}
                    role="tab"
                    aria-selected={p === active}
                    className="ex-tab"
                    data-status={s ?? undefined}
                    title={p}
                    tabIndex={0}
                    onClick={() => setActive(p)}
                    onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), setActive(p))}
                    onAuxClick={(e) => e.button === 1 && close(p)}
                  >
                    <FileIcon path={p} size={14} />
                    <span className="ex-tab-name">{splitPath(p).base}</span>
                    <button
                      className="ex-tab-close"
                      aria-label={`Close ${splitPath(p).base}`}
                      title="Close"
                      onClick={(e) => {
                        e.stopPropagation();
                        close(p);
                      }}
                    >
                      <Icon name="x" size={11} />
                    </button>
                  </div>
                );
              })}
            </div>
          )}
          {active && (
            <div className="rv-head">
              <FileIcon path={active} />
              <span className="rv-path mono" title={active}>
                <span className="file-base">{splitPath(active).base}</span>
                <span className="file-dir"> {splitPath(active).dir.replace(/\/$/, "")}</span>
              </span>
              {status && (
                <span className="status-letter" data-status={status} title={STATUS_TITLE[status]} aria-label={STATUS_TITLE[status]}>
                  {status}
                </span>
              )}
              {v && v.path === active && <span className="muted-sm ex-size">{sizeLabel(v.size)}</span>}
              <span className="spacer" />
              <button
                className="ghost-btn ex-head-btn"
                title={absolutePath(agent.cwd, active)}
                onClick={() => void navigator.clipboard?.writeText(absolutePath(agent.cwd, active)).then(() => setCopied(true), () => {})}
              >
                <Icon name="copy" size={13} />
                {copied ? "Copied" : "Copy path"}
              </button>
              {status && agent.caps.review && (
                <button className="ghost-btn ex-head-btn" onClick={() => onShowDiff(active)} title="This file's changes, in Review">
                  <Icon name="review" size={13} />
                  Show diff
                </button>
              )}
            </div>
          )}
          {doc?.newer && (
            <div className="ex-stale" role="status">
              Changed on disk since it was opened.
              <button
                className="retry-btn"
                onClick={() => setDocs((d) => (d[active!]?.newer ? { ...d, [active!]: { view: d[active!].newer, error: null, loading: false, newer: null } } : d))}
              >
                Reload
              </button>
            </div>
          )}
          {body}
        </section>
      </div>
    </div>
  );
}
