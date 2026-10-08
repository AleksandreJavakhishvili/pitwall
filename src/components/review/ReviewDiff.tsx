import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { EditorView } from "@codemirror/view";
import { errorText } from "../../api";
import type { FileVersions } from "../../reviewTypes";
import type { FileChange } from "../../types";
import type { ReviewComment } from "./comments";
import { languageFor, type Lang } from "./editorSetup";
import { DiffView, type MenuRequest } from "./diffView";
import { Kbd } from "../Kbd";
import { currentScheme, onSchemeChange, type Scheme } from "../../lib/theme";

interface Props {
  /** What the file belongs to (an agent, or a worktree): a new one starts fresh. */
  sourceKey: string;
  /** Before/after text of `path` for this source and scope. */
  load(path: string): Promise<FileVersions>;
  file: FileChange;
  /** Part of what is shown (per-task scope); a change reloads. */
  taskId: string | null;
  sideBySide: boolean;
  /** Bumped to refetch (e.g. after the agent's totals moved). */
  version: string;
  comments: ReviewComment[];
  /** Absent: no comments here (worktrees without the agent's own folder). */
  onAddComment?(line: number, text: string): void;
  /** Scroll to this line once content is there. */
  reveal: { line: number; nonce: number } | null;
}

interface Loaded {
  v: FileVersions;
  lang: Lang | null;
}

/** Settings → Appearance, or macOS while on System: the editor colours (review.css) follow it. */
function useScheme(): Scheme {
  const [scheme, setScheme] = useState(currentScheme);
  useEffect(() => onSchemeChange(setScheme), []);
  return scheme;
}

export function ReviewDiff({ sourceKey, load, file, taskId, sideBySide, version, comments, onAddComment, reveal }: Props) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [composer, setComposer] = useState<{ line: number } | null>(null);
  const [draft, setDraft] = useState("");
  const [menu, setMenu] = useState<MenuRequest | null>(null);
  const scheme = useScheme();
  const boxRef = useRef<HTMLDivElement | null>(null);
  const diffRef = useRef<DiffView | null>(null);
  const pendingReveal = useRef<number | null>(null);

  const loadRef = useRef(load);
  loadRef.current = load;
  const commentRef = useRef(onAddComment);
  commentRef.current = onAddComment;
  const commentsRef = useRef(comments);
  commentsRef.current = comments;

  useEffect(() => {
    setLoaded(null);
    setError(null);
    setComposer(null);
  }, [sourceKey, file.path, taskId]);

  useEffect(() => {
    let alive = true;
    Promise.all([loadRef.current(file.path), languageFor(file.path)])
      .then(([v, lang]) => {
        if (!alive) return;
        setLoaded({ v, lang });
        setError(null);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [sourceKey, file.path, taskId, version]);

  const revealNow = (line: number) => {
    if (!diffRef.current?.reveal(line)) pendingReveal.current = line;
    else pendingReveal.current = null;
  };

  const startComment = (line: number) => {
    if (!commentRef.current) return;
    setComposer({ line });
    setDraft("");
  };

  const v = loaded?.v;
  const showable = !!v && !v.binary && !(v.original === null && v.modified === null);

  // The editors: rebuilt for new content or layout, destroyed on unmount (nothing stays behind).
  useLayoutEffect(() => {
    const box = boxRef.current;
    if (!box || !loaded || !showable) return;
    const diff = new DiffView({
      parent: box,
      original: loaded.v.original ?? "",
      modified: loaded.v.modified ?? "",
      lang: loaded.lang,
      sideBySide,
      commentable: !!onAddComment,
      readOnlyText: onAddComment ? "Read-only: click a line number to comment" : "Read-only",
      onComment: startComment,
      onMenu: setMenu,
    });
    diffRef.current = diff;
    diff.setComments(commentsRef.current);
    if (pendingReveal.current) revealNow(pendingReveal.current);
    return () => {
      diffRef.current = null;
      setMenu(null);
      diff.destroy();
    };
  }, [loaded, sideBySide, !!onAddComment]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (reveal) revealNow(reveal.line);
  }, [reveal?.nonce]); // eslint-disable-line react-hooks/exhaustive-deps

  // Commented lines: a marker in the glyph margin + a soft line tint; hover shows the text.
  useEffect(() => {
    diffRef.current?.setComments(comments);
    if (pendingReveal.current) revealNow(pendingReveal.current);
  }, [comments]); // eslint-disable-line react-hooks/exhaustive-deps

  const add = () => {
    if (!composer || !draft.trim() || !onAddComment) return;
    onAddComment(composer.line, draft);
    setComposer(null);
    setDraft("");
  };

  let body: React.ReactNode = null;
  if (error) body = <p className="hint hint-error pad">Couldn't load this file: {error}</p>;
  else if (!v) body = <p className="hint pad">Loading…</p>;
  else if (v.binary) body = <p className="rv-empty">Binary or very large file — not shown</p>;
  else if (v.original === null && v.modified === null) body = <p className="rv-empty">No content on either side</p>;

  return (
    <div className="rv-diff" data-scheme={scheme}>
      {body}
      {!body && <div className="rv-diffbox" ref={boxRef} />}
      {menu && (
        <DiffMenu
          req={menu}
          onComment={menu.line !== null && onAddComment ? () => startComment(menu.line!) : null}
          onClose={() => setMenu(null)}
        />
      )}
      {composer && (
        <div className="rv-composer" role="dialog" aria-label="Add comment">
          <div className="rv-composer-head">
            <span className="label">Comment</span>
            <span className="mono rv-composer-where">
              {file.path}:{composer.line}
            </span>
            <span className="spacer" />
            <button className="icon-btn icon-btn-sm" onClick={() => setComposer(null)} aria-label="Cancel">
              ✕
            </button>
          </div>
          <textarea
            autoFocus
            value={draft}
            rows={3}
            placeholder="What should change here?"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                add();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                setComposer(null);
              }
            }}
          />
          <div className="rv-composer-foot">
            <span className="hint">
              Collected, not sent. <Kbd submit>⌘↵</Kbd> to add
            </span>
            <span className="spacer" />
            <button className="ghost-btn" onClick={() => setComposer(null)}>
              Cancel
            </button>
            <button className="primary-btn" onClick={add} disabled={!draft.trim()}>
              Add comment
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/** The editor's context menu (Monaco's): Add review comment · Copy. */
function DiffMenu({ req, onComment, onClose }: { req: MenuRequest; onComment: (() => void) | null; onClose(): void }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ left: req.x, top: req.y });
  const [active, setActive] = useState(-1);
  const copy = (view: EditorView) => {
    // The editor's own copy handler: the selection, or the whole line when it is empty.
    view.focus();
    document.execCommand("copy");
  };
  const items = [
    ...(onComment ? [{ label: "Add review comment", run: onComment }, null] : []),
    { label: "Copy", run: () => copy(req.view) },
  ];
  const actions = items.filter((i) => i !== null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(0, Math.min(req.x, window.innerWidth - r.width)),
      top: req.y + r.height > window.innerHeight ? Math.max(0, req.y - r.height) : req.y,
    });
    el.focus();
  }, [req]);

  useEffect(() => {
    const away = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const blur = () => onClose();
    window.addEventListener("mousedown", away, true);
    window.addEventListener("blur", blur);
    window.addEventListener("resize", blur);
    return () => {
      window.removeEventListener("mousedown", away, true);
      window.removeEventListener("blur", blur);
      window.removeEventListener("resize", blur);
    };
  }, [onClose]);

  const run = (i: number) => {
    onClose();
    actions[i]?.run();
  };

  let n = -1;
  return (
    <div
      ref={ref}
      className="rv-menu"
      role="menu"
      tabIndex={-1}
      style={pos}
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Escape") {
          e.preventDefault();
          onClose();
          req.view.focus();
        } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const d = e.key === "ArrowDown" ? 1 : -1;
          setActive((a) => (a < 0 ? (d > 0 ? 0 : actions.length - 1) : (a + d + actions.length) % actions.length));
        } else if ((e.key === "Enter" || e.key === " ") && active >= 0) {
          e.preventDefault();
          run(active);
        }
      }}
    >
      {items.map((item, i) => {
        if (!item) return <div key={`sep${i}`} className="rv-menu-sep" role="separator" />;
        const k = ++n;
        return (
          <div
            key={item.label}
            role="menuitem"
            className={`rv-menu-item${k === active ? " active" : ""}`}
            onMouseEnter={() => setActive(k)}
            onMouseLeave={() => setActive(-1)}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => run(k)}
          >
            {item.label}
          </div>
        );
      })}
    </div>
  );
}
