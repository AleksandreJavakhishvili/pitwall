// Monaco, bundled locally (no CDN: the desktop app must work offline).
// Only the editor core, the diff editor, a few read-only conveniences and the
// Monarch syntax highlighters are included — no language services/workers
// beyond the base editor worker (used for diff computation).
import * as monaco from "monaco-editor/editor/editor.api";
import "monaco-editor/features/diffEditor/register";
import "monaco-editor/features/codicon/register";
import "monaco-editor/features/find/register";
import "monaco-editor/features/folding/register";
import "monaco-editor/features/hover/register";
import "monaco-editor/features/clipboard/register";
import "monaco-editor/features/contextmenu/register";
import "monaco-editor/features/bracketMatching/register";
import "monaco-editor/features/readOnlyMessage/register";
import "monaco-editor/basic-languages/monaco.contribution";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { loader } from "@monaco-editor/react";
import { currentScheme } from "../../lib/theme";
import { onMonacoRelease } from "./monacoLifecycle";

(self as unknown as { MonacoEnvironment: unknown }).MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
};
loader.config({ monaco });

onMonacoRelease(() => {
  for (const ed of monaco.editor.getDiffEditors()) ed.dispose();
  for (const ed of monaco.editor.getEditors()) ed.dispose();
  for (const m of monaco.editor.getModels()) m.dispose();
});

const css = (name: string, fallback: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;

/** Editor colours from the Pitwall tokens; colour stays quiet except for +/−. */
export function defineThemes() {
  const dark = currentScheme() === "dark";
  const bg = css("--term-bg", dark ? "#0d0f12" : "#fbfbfc");
  const line = css("--line", dark ? "#1e2227" : "#dfe2e6");
  const text = css("--text", dark ? "#e6e8eb" : "#15181c");
  const muted = css("--text-4", dark ? "#474d55" : "#a9aeb5");
  const sub = css("--text-3", dark ? "#6c737d" : "#7d848e");
  const surface = css("--surface-2", dark ? "#15181c" : "#eef0f2");
  const name = dark ? "pitwall-dark" : "pitwall-light";
  monaco.editor.defineTheme(name, {
    base: dark ? "vs-dark" : "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": bg,
      "editor.foreground": text,
      "editorGutter.background": bg,
      "editorLineNumber.foreground": muted,
      "editorLineNumber.activeForeground": sub,
      "editor.lineHighlightBackground": "#00000000",
      "editor.lineHighlightBorder": "#00000000",
      "editorWidget.background": surface,
      "editorWidget.border": line,
      "editorHoverWidget.background": surface,
      "editorHoverWidget.border": line,
      "scrollbarSlider.background": dark ? "#ffffff14" : "#00000014",
      "scrollbarSlider.hoverBackground": dark ? "#ffffff24" : "#00000024",
      "diffEditor.insertedTextBackground": dark ? "#3fd07f26" : "#14a05224",
      "diffEditor.removedTextBackground": dark ? "#ff5f5f26" : "#d63a3a22",
      "diffEditor.insertedLineBackground": dark ? "#3fd07f14" : "#14a05214",
      "diffEditor.removedLineBackground": dark ? "#ff5f5f14" : "#d63a3a12",
      "diffEditorGutter.insertedLineBackground": dark ? "#3fd07f22" : "#14a05222",
      "diffEditorGutter.removedLineBackground": dark ? "#ff5f5f22" : "#d63a3a20",
      "diffEditor.unchangedRegionBackground": surface,
      "diffEditor.unchangedRegionForeground": sub,
      "diffEditor.border": line,
      "editorOverviewRuler.border": "#00000000",
    },
  });
  return name;
}

/** Monaco language id for a path (by extension or file name). */
export function languageFor(path: string): string {
  const name = path.split("/").pop() ?? path;
  const lower = name.toLowerCase();
  const dot = lower.lastIndexOf(".");
  const ext = dot >= 0 ? lower.slice(dot) : "";
  for (const l of monaco.languages.getLanguages()) {
    if (l.filenames?.some((f) => f.toLowerCase() === lower)) return l.id;
  }
  if (ext) {
    for (const l of monaco.languages.getLanguages()) {
      if (l.extensions?.some((e) => e.toLowerCase() === ext)) return l.id;
    }
  }
  return "plaintext";
}

export { monaco };
