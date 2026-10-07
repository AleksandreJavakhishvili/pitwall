// What every Review editor shares: the read-only setup, the look (made to
// match the Monaco editor Review used before: same fonts, colours, guides and
// bracket colours), syntax highlighting and Find. Languages load on demand
// (their own chunks, bundled — no CDN), only for files Monaco highlighted.
import { EditorSelection, EditorState, Prec, StateEffect, StateField, type Extension, type Range } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  ViewPlugin,
  drawSelection,
  keymap,
  showTooltip,
  type DecorationSet,
  type Tooltip,
  type ViewUpdate,
} from "@codemirror/view";
import { defaultKeymap } from "@codemirror/commands";
import {
  HighlightStyle,
  LanguageDescription,
  bracketMatching,
  indentUnit,
  syntaxHighlighting,
  syntaxTree,
  type LanguageSupport,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { closeSearchPanel, findNext, findPrevious, openSearchPanel, search } from "@codemirror/search";
import { styleTags, tags as t } from "@lezer/highlight";
import { indentationMarkers } from "@replit/codemirror-indentation-markers";
import { findPanel } from "./findPanel";

// ── languages ────────────────────────────────────────────────────────────────

/** Known to the language data but plain text in Monaco's build: keep them plain. */
const PLAIN = new Set(["JSON", "JSON-LD", "TOML", "diff"]);

export interface Lang {
  name: string;
  support: LanguageSupport;
}

/** The language for a path (by file name, then extension), loaded; null for plain text. */
export async function languageFor(path: string): Promise<Lang | null> {
  const name = path.split("/").pop() ?? path;
  const desc = LanguageDescription.matchFilename(languages, name);
  if (!desc || PLAIN.has(desc.name)) return null;
  try {
    if (desc.name === "Markdown") return { name: desc.name, support: await markdownSupport() };
    return { name: desc.name, support: await desc.load() };
  } catch {
    return null; // a chunk that failed to load: show the file plain
  }
}

// Monaco's Markdown colours the markers (list bullets, quote bars, code fences, a link's
// brackets and URL), not whole blocks.
async function markdownSupport() {
  const { markdown } = await import("@codemirror/lang-markdown");
  return markdown({
    extensions: [
      {
        props: [
          styleTags({
            ListMark: t.keyword,
            QuoteMark: t.quote,
            "FencedCode/CodeMark FencedCode/CodeInfo": t.string,
            "InlineCode/CodeMark": t.monospace,
            Link: t.content,
            "Link/LinkMark Link/URL": t.link,
          }),
        ],
      },
    ],
  });
}

// ── highlighting (Monaco's vs / vs-dark token colours, via --rv-tk-* in review.css) ──

const tk = (name: string) => `var(--rv-tk-${name})`;
const baseSpecs = [
  {
    tag: [t.keyword, t.controlKeyword, t.definitionKeyword, t.moduleKeyword, t.operatorKeyword, t.modifier, t.self, t.null, t.bool, t.atom, t.heading],
    color: tk("keyword"),
  },
  { tag: [t.string, t.special(t.string), t.character, t.docString, t.link, t.url, t.attributeValue], color: tk("string") },
  { tag: [t.number, t.integer, t.float], color: tk("number") },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment, t.quote], color: tk("comment") },
  { tag: [t.typeName, t.className], color: tk("type") },
  { tag: [t.operator, t.punctuation], color: tk("delimiter") },
  { tag: t.regexp, color: tk("regexp") },
  { tag: t.monospace, color: tk("variable") },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strong, fontWeight: "bold" },
];
const markupSpecs = [
  { tag: t.tagName, color: tk("tag") },
  { tag: t.attributeName, color: tk("attr") },
];
// Monaco's TypeScript/JavaScript tokenizer has no JSX: tag and attribute names stay plain there.
const JS = new Set(["JavaScript", "TypeScript", "JSX", "TSX"]);
const styles = {
  js: HighlightStyle.define(baseSpecs),
  markup: HighlightStyle.define([...baseSpecs, ...markupSpecs]),
};

function highlighting(lang: Lang | null): Extension {
  if (!lang) return [];
  return [lang.support, syntaxHighlighting(JS.has(lang.name) ? styles.js : styles.markup), bracketColors];
}

// ── bracket pair colours (Monaco's bracketPairColorization) ────────────────────

const OPEN = "([{";
const CLOSE = ")]}";
const levelMark = [0, 1, 2].map((i) => Decoration.mark({ class: `rv-br${i}` }));
const strayMark = Decoration.mark({ class: "rv-br-x" });

interface Bracket {
  pos: number;
  /** Nesting level, or -1 for a closer with no opener. */
  level: number;
}

/** Brackets of the whole document, outside strings and comments, with their nesting level. */
function scanBrackets(state: EditorState): Bracket[] {
  const tree = syntaxTree(state);
  const found: { pos: number; ch: string }[] = [];
  const skip: [number, number][] = [];
  tree.iterate({
    enter(n) {
      const name = n.name;
      if (name.length === 1 && (OPEN.includes(name) || CLOSE.includes(name))) found.push({ pos: n.from, ch: name });
      else if (/string|comment|regexp|template|^(Link|URL)$/i.test(name)) {
        skip.push([n.from, n.to]);
        return false;
      }
    },
  });
  // Grammars without bracket tokens (the legacy modes): scan the text instead.
  if (!found.length) {
    const text = state.doc.toString();
    let s = 0;
    for (let i = 0; i < text.length; i++) {
      while (s < skip.length && skip[s][1] <= i) s++;
      if (s < skip.length && skip[s][0] <= i) {
        i = skip[s][1] - 1;
        continue;
      }
      const ch = text[i];
      if (OPEN.includes(ch) || CLOSE.includes(ch)) found.push({ pos: i, ch });
    }
  }
  const out: Bracket[] = [];
  const stack: string[] = [];
  for (const { pos, ch } of found) {
    const o = OPEN.indexOf(ch);
    if (o >= 0) {
      out.push({ pos, level: stack.length });
      stack.push(ch);
    } else if (stack.length && stack[stack.length - 1] === OPEN[CLOSE.indexOf(ch)]) {
      stack.pop();
      out.push({ pos, level: stack.length });
    } else out.push({ pos, level: -1 });
  }
  return out;
}

const bracketColors = ViewPlugin.fromClass(
  class {
    tree: ReturnType<typeof syntaxTree>;
    brackets: Bracket[];
    decorations: DecorationSet;
    constructor(readonly view: EditorView) {
      this.tree = syntaxTree(view.state);
      this.brackets = scanBrackets(view.state);
      this.decorations = this.build();
    }
    update(u: ViewUpdate) {
      const tree = syntaxTree(u.state);
      if (tree !== this.tree || u.docChanged) {
        this.tree = tree;
        this.brackets = scanBrackets(u.state);
      } else if (!u.viewportChanged) return;
      this.decorations = this.build();
    }
    build() {
      const out: Range<Decoration>[] = [];
      for (const { from, to } of this.view.visibleRanges) {
        let lo = 0;
        let hi = this.brackets.length;
        while (lo < hi) {
          const mid = (lo + hi) >> 1;
          if (this.brackets[mid].pos < from) lo = mid + 1;
          else hi = mid;
        }
        for (let i = lo; i < this.brackets.length && this.brackets[i].pos < to; i++) {
          const b = this.brackets[i];
          out.push((b.level < 0 ? strayMark : levelMark[b.level % 3]).range(b.pos, b.pos + 1));
        }
      }
      return Decoration.set(out);
    }
  },
  { decorations: (p) => p.decorations },
);

// ── indentation (Monaco detects it per file; guides follow it) ────────────────

function detectIndent(text: string): string {
  let tabs = 0;
  let spaces = 0;
  let prev = 0;
  const deltas = new Map<number, number>();
  for (const line of text.split("\n", 5000)) {
    if (!line.trim()) continue;
    const lead = /^[ \t]*/.exec(line)![0];
    if (lead.startsWith("\t")) tabs++;
    else if (lead) spaces++;
    const n = lead.length;
    const d = Math.abs(n - prev);
    if (d >= 2 && d <= 8 && !lead.includes("\t")) deltas.set(d, (deltas.get(d) ?? 0) + 1);
    prev = n;
  }
  if (tabs > spaces) return "\t";
  let best = 4;
  let count = 0;
  for (const [d, c] of deltas) if (c > count || (c === count && d < best)) [best, count] = [d, c];
  return " ".repeat(best);
}

function indentation(doc: string): Extension {
  const unit = detectIndent(doc);
  return [
    indentUnit.of(unit),
    EditorState.tabSize.of(unit === "\t" ? 4 : unit.length),
    indentationMarkers({
      markerType: "fullScope",
      thickness: 1,
      colors: { light: "var(--rv-guide)", dark: "var(--rv-guide)", activeLight: "var(--rv-guide-active)", activeDark: "var(--rv-guide-active)" },
    }),
  ];
}

// ── read-only message (what Monaco says when you type into the diff) ─────────

const showReadOnly = StateEffect.define<number | null>();

function readOnlyMessage(text: string): Extension {
  const field = StateField.define<Tooltip | null>({
    create: () => null,
    update(tip, tr) {
      for (const e of tr.effects) if (e.is(showReadOnly)) tip = e.value === null ? null : messageTip(e.value, text);
      if (tip && tr.selection && !tr.effects.some((e) => e.is(showReadOnly))) tip = null;
      return tip;
    },
    provide: (f) => showTooltip.from(f),
  });
  const show = (view: EditorView) => {
    view.dispatch({ effects: showReadOnly.of(view.state.selection.main.head) });
    return true;
  };
  const hide = (view: EditorView) => {
    if (view.state.field(field)) view.dispatch({ effects: showReadOnly.of(null) });
    return false;
  };
  return [
    field,
    EditorView.domEventHandlers({
      keydown(e, view) {
        if (e.key === "Escape") return hide(view);
        if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing) {
          if ((e.metaKey || e.ctrlKey) && (e.key === "x" || e.key === "v")) show(view);
          return false;
        }
        if (e.key.length === 1 || e.key === "Backspace" || e.key === "Delete" || e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          return show(view);
        }
        return false;
      },
      paste: (e, view) => (e.preventDefault(), show(view)),
      drop: (e, view) => (e.preventDefault(), show(view)),
      blur: (_e, view) => hide(view),
      mousedown: (_e, view) => hide(view),
    }),
  ];
}

function messageTip(pos: number, text: string): Tooltip {
  return {
    pos,
    above: true,
    strictSide: false,
    create() {
      const dom = document.createElement("div");
      dom.className = "rv-ro";
      const msg = dom.appendChild(document.createElement("div"));
      msg.className = "rv-ro-msg";
      msg.textContent = text;
      dom.appendChild(document.createElement("div")).className = "rv-ro-anchor";
      return { dom, offset: { x: -6, y: 0 } };
    },
  };
}

// ── find (Monaco's find widget, top right) ───────────────────────────────────

/** Opens Find seeded with the selection, or the word at the cursor (as Monaco does). */
function openFind(view: EditorView) {
  const sel = view.state.selection.main;
  if (sel.empty) {
    const word = view.state.wordAt(sel.head);
    if (word) view.dispatch({ selection: EditorSelection.range(word.from, word.to) });
  }
  return openSearchPanel(view);
}

const findKeys = keymap.of([
  { key: "Mod-f", run: openFind, scope: "editor search-panel", preventDefault: true },
  { key: "F3", run: findNext, shift: findPrevious, scope: "editor search-panel", preventDefault: true },
  { key: "Mod-g", run: findNext, shift: findPrevious, scope: "editor search-panel", preventDefault: true },
  { key: "Escape", run: closeSearchPanel, scope: "editor search-panel" },
]);

// ── the editor ───────────────────────────────────────────────────────────────

export interface EditorOptions {
  doc: string;
  lang: Lang | null;
  readOnlyText: string;
}

/** Extensions for one read-only editor in the diff. */
export function editorExtensions({ doc, lang, readOnlyText }: EditorOptions): Extension {
  return [
    EditorState.readOnly.of(true),
    EditorState.changeFilter.of(() => false),
    EditorState.phrases.of({ "$ unchanged lines": "$ hidden lines" }),
    drawSelection({ cursorBlinkRate: 1000 }),
    bracketMatching(),
    highlighting(lang),
    indentation(doc),
    search({ top: true, createPanel: findPanel }),
    Prec.high(findKeys),
    keymap.of(defaultKeymap),
    readOnlyMessage(readOnlyText),
    EditorView.contentAttributes.of({ "aria-readonly": "true" }),
  ];
}
