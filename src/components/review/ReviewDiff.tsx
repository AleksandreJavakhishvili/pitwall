import { useEffect, useMemo, useRef, useState } from "react";
import { DiffEditor, type DiffOnMount } from "@monaco-editor/react";
import type { editor as MonacoEditor } from "monaco-editor/editor/editor.api";
import { errorText } from "../../api";
import type { FileVersions } from "../../reviewTypes";
import type { FileChange } from "../../types";
import type { ReviewComment } from "./comments";
import { defineThemes, languageFor, monaco } from "./monaco";
import { Kbd } from "../Kbd";
import { onSchemeChange } from "../../lib/theme";

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

const GUTTER = new Set<number>([
  monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN,
  monaco.editor.MouseTargetType.GUTTER_LINE_NUMBERS,
  monaco.editor.MouseTargetType.GUTTER_LINE_DECORATIONS,
]);

function useTheme(): string {
  const [theme, setTheme] = useState(defineThemes);
  useEffect(() => {
    // Settings → Appearance, or macOS while on System. Redefine from the (now switched) CSS tokens.
    return onSchemeChange(() => {
      const t = defineThemes();
      monaco.editor.setTheme(t);
      setTheme(t);
    });
  }, []);
  return theme;
}

export function ReviewDiff({ sourceKey, load, file, taskId, sideBySide, version, comments, onAddComment, reveal }: Props) {
  const [v, setV] = useState<FileVersions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [composer, setComposer] = useState<{ line: number } | null>(null);
  const [draft, setDraft] = useState("");
  const theme = useTheme();
  const editorRef = useRef<MonacoEditor.IStandaloneDiffEditor | null>(null);
  const commentDeco = useRef<MonacoEditor.IEditorDecorationsCollection | null>(null);
  const hoverDeco = useRef<MonacoEditor.IEditorDecorationsCollection | null>(null);
  const pendingReveal = useRef<number | null>(null);

  const loadRef = useRef(load);
  loadRef.current = load;
  const commentRef = useRef(onAddComment);
  commentRef.current = onAddComment;

  useEffect(() => {
    setV(null);
    setError(null);
    setComposer(null);
  }, [sourceKey, file.path, taskId]);

  useEffect(() => {
    let alive = true;
    loadRef
      .current(file.path)
      .then((x) => {
        if (!alive) return;
        setV(x);
        setError(null);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [sourceKey, file.path, taskId, version]);

  const language = useMemo(() => languageFor(file.path), [file.path]);

  // Fonts load after Monaco measures; remeasure once they're in.
  useEffect(() => {
    document.fonts?.ready.then(() => monaco.editor.remeasureFonts()).catch(() => {});
  }, []);

  const revealNow = (line: number) => {
    const ed = editorRef.current?.getModifiedEditor();
    if (!ed || !ed.getModel() || line > ed.getModel()!.getLineCount()) {
      pendingReveal.current = line;
      return;
    }
    pendingReveal.current = null;
    ed.revealLineInCenter(line);
    ed.setPosition({ lineNumber: line, column: 1 });
  };

  useEffect(() => {
    if (reveal) revealNow(reveal.line);
  }, [reveal?.nonce]); // eslint-disable-line react-hooks/exhaustive-deps

  // Commented lines: a marker in the glyph margin + a soft line tint; hover shows the text.
  useEffect(() => {
    const coll = commentDeco.current;
    if (!coll) return;
    coll.set(
      comments.map((c) => ({
        range: new monaco.Range(c.line, 1, c.line, 1),
        options: {
          isWholeLine: true,
          className: "rv-line-commented",
          glyphMarginClassName: "rv-glyph-comment",
          glyphMarginHoverMessage: { value: c.text.replace(/[\\`*_{}[\]()#+\-.!|<>]/g, "\\$&") },
        },
      })),
    );
    if (pendingReveal.current) revealNow(pendingReveal.current);
  }, [comments, v]); // eslint-disable-line react-hooks/exhaustive-deps

  const onMount: DiffOnMount = (editor) => {
    editorRef.current = editor;
    const mod = editor.getModifiedEditor();
    commentDeco.current = mod.createDecorationsCollection();
    hoverDeco.current = mod.createDecorationsCollection();
    mod.onMouseDown((e) => {
      if (commentRef.current && GUTTER.has(e.target.type) && e.target.position) {
        setComposer({ line: e.target.position.lineNumber });
        setDraft("");
      }
    });
    mod.onMouseMove((e) => {
      const line = commentRef.current ? e.target.position?.lineNumber : undefined;
      hoverDeco.current?.set(
        line ? [{ range: new monaco.Range(line, 1, line, 1), options: { glyphMarginClassName: "rv-glyph-add" } }] : [],
      );
    });
    mod.onMouseLeave(() => hoverDeco.current?.clear());
    mod.addAction({
      id: "pitwall.review.comment",
      label: "Add review comment",
      contextMenuGroupId: "navigation",
      contextMenuOrder: 0,
      run: (ed) => {
        const p = ed.getPosition();
        if (p && commentRef.current) {
          setComposer({ line: p.lineNumber });
          setDraft("");
        }
      },
    });
    editor.onDidUpdateDiff(() => {
      if (pendingReveal.current) revealNow(pendingReveal.current);
    });
    // Re-apply decorations now that the collection exists.
    setV((x) => (x ? { ...x } : x));
  };

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
    <div className="rv-diff">
      {body}
      {!body && v && (
        <DiffEditor
          height="100%"
          theme={theme}
          language={language}
          original={v.original ?? ""}
          modified={v.modified ?? ""}
          onMount={onMount}
          loading={<p className="hint pad">Loading editor…</p>}
          options={{
            readOnly: true,
            originalEditable: false,
            renderSideBySide: sideBySide,
            useInlineViewWhenSpaceIsLimited: false,
            hideUnchangedRegions: { enabled: true, contextLineCount: 3, minimumLineCount: 4, revealLineCount: 20 },
            glyphMargin: true,
            minimap: { enabled: false },
            scrollBeyondLastLine: false,
            renderLineHighlight: "none",
            stickyScroll: { enabled: false },
            fontFamily: '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, Menlo, monospace',
            fontSize: 12.5,
            lineHeight: 19,
            ignoreTrimWhitespace: false,
            renderOverviewRuler: true,
            fixedOverflowWidgets: true,
            automaticLayout: true,
            contextmenu: true,
            readOnlyMessage: { value: onAddComment ? "Read-only: click a line number to comment" : "Read-only" },
          }}
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
              Collected, not sent. <Kbd>⌘↵</Kbd> to add
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
