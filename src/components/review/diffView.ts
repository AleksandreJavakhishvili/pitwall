// The Review diff on CodeMirror 6 + @codemirror/merge, laid out like the
// Monaco diff editor it replaces: side by side (MergeView) or inline
// (unifiedMergeView); a margin per editor with comment glyph, line number(s)
// and +/− sign; hatched filler where the other side has lines; collapsed
// unchanged regions; overlay scrollbars with a cursor lane; and the diff
// overview ruler on the right. Comments live in the modified file's lines.
import { StateEffect, StateField, type Extension, type Range, type Text } from "@codemirror/state";
import { BlockInfo, BlockType, Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType, gutter, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { MergeView, getChunks, getOriginalDoc, mergeViewSiblings, unifiedMergeView, type Chunk } from "@codemirror/merge";
import { getSearchQuery, searchPanelOpen } from "@codemirror/search";
import { editorExtensions, type Lang } from "./editorSetup";
import { lineDiff } from "./lineDiff";

/** Monaco's hideUnchangedRegions: 3 lines of context, collapse runs of 4 or more. */
const COLLAPSE = { margin: 3, minSize: 4 };
const DIFF = { scanLimit: 500, timeout: 2000 };
const DIFF_CONFIG = { ...DIFF, override: lineDiff(DIFF) };

type Side = "a" | "b" | "unified";

export interface MenuRequest {
  x: number;
  y: number;
  view: EditorView;
  /** The line it was opened on, in the modified file (null on the original side). */
  line: number | null;
}

export interface DiffViewOptions {
  parent: HTMLElement;
  original: string;
  modified: string;
  lang: Lang | null;
  sideBySide: boolean;
  /** Comments can be added (clicking the margin, the context menu). */
  commentable: boolean;
  readOnlyText: string;
  onComment(line: number): void;
  onMenu(req: MenuRequest): void;
}

// ── per-editor state: comments and the hovered line ─────────────────────────

const setComments = StateEffect.define<ReadonlyMap<number, string>>();
const commentsField = StateField.define<ReadonlyMap<number, string>>({
  create: () => new Map(),
  update: (v, tr) => tr.effects.reduce((acc, e) => (e.is(setComments) ? e.value : acc), v),
});
const setHover = StateEffect.define<number | null>();
const hoverField = StateField.define<number | null>({
  create: () => null,
  update: (v, tr) => tr.effects.reduce((acc, e) => (e.is(setHover) ? e.value : acc), v),
});

// ── what changed, per line ───────────────────────────────────────────────────

interface LineInfo {
  /** Line is part of a change on this side. */
  changed: Uint8Array;
  /** The whole line is inserted/deleted (Monaco paints it like changed text). */
  full: Uint8Array;
  /** Unified: the original line number of each modified line (0 when inserted). */
  orig: Int32Array | null;
}

const infoCache = new WeakMap<readonly Chunk[], Map<Side, LineInfo>>();

function changedLines(doc: Text, chunks: readonly Chunk[], isA: boolean, mark: (line: number, full: boolean) => void) {
  for (const c of chunks) {
    const from = isA ? c.fromA : c.fromB;
    const to = isA ? c.toA : c.toB;
    if (from >= to) continue;
    const end = isA ? c.endA : c.endB;
    for (let n = doc.lineAt(from).number, last = doc.lineAt(end).number; n <= last; n++) {
      const line = doc.line(n);
      // Whole line: the change covers its text and a newline next to it (after, or before when
      // the change runs from the end of the previous line).
      const full = c.changes.some((ch) => {
        const s = from + (isA ? ch.fromA : ch.fromB);
        const e = from + (isA ? ch.toA : ch.toB);
        return e > s && ((s <= line.from && e > line.to) || (s < line.from && e >= line.to));
      });
      mark(n, full);
    }
  }
}

function lineInfo(view: EditorView, side: Side): LineInfo | null {
  const res = getChunks(view.state);
  if (!res) return null;
  let bySide = infoCache.get(res.chunks);
  if (!bySide) infoCache.set(res.chunks, (bySide = new Map()));
  let info = bySide.get(side);
  if (info) return info;
  const doc = view.state.doc;
  const changed = new Uint8Array(doc.lines + 2);
  const full = new Uint8Array(doc.lines + 2);
  changedLines(doc, res.chunks, side === "a", (n, f) => {
    changed[n] = 1;
    if (f) full[n] = 1;
  });
  // Monaco: when one side is empty (a new or deleted file), every line of both sides is changed.
  // (While MergeView is still building its second editor there is no sibling yet: don't cache.)
  const other = side === "unified" ? originalOf(view) : mergeViewSiblings(view)?.[side === "a" ? "b" : "a"]?.state.doc;
  const whole = res.chunks.length > 0 && (doc.length === 0 || other?.length === 0);
  if (whole) changed.fill(1, 1, doc.lines + 1);
  let orig: Int32Array | null = null;
  if (side === "unified" && whole) orig = new Int32Array(doc.lines + 2);
  else if (side === "unified") {
    // Walk unchanged stretches between chunks: they map 1:1, offset by what came before.
    orig = new Int32Array(doc.lines + 2);
    const a = originalOf(view);
    let lineB = 1;
    let lineA = 1;
    for (const c of [...res.chunks, null]) {
      const stopB = c ? doc.lineAt(c.fromB).number : doc.lines + 1;
      for (; lineB < stopB; lineB++, lineA++) orig[lineB] = lineA;
      if (!c) break;
      if (c.fromB < c.toB) lineB = doc.lineAt(c.endB).number + 1;
      if (c.fromA < c.toA) lineA = a.lineAt(c.endA).number + 1;
    }
  }
  info = { changed, full, orig };
  if (other) bySide.set(side, info);
  return info;
}

/** Inline: the original text (merge keeps it in the state). */
const originalOf = (view: EditorView): Text => getOriginalDoc(view.state);

// Whole-line backgrounds: changed lines (merge marks most; this adds the empty-side case),
// fully inserted/deleted lines and commented lines.
const changedLine = Decoration.line({ class: "cm-changedLine" });
const fullLine = Decoration.line({ class: "rv-full" });
const commentedLine = Decoration.line({ class: "rv-line-commented" });

/** Monaco marks where the other side inserted text inside a changed line with a thin bar. */
class EmptyRange extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "rv-empty-range";
    return el;
  }
}
const emptyRange = Decoration.widget({ widget: new EmptyRange(), side: -1 });

function emptyRanges(view: EditorView, isA: boolean, from: number, to: number) {
  const res = getChunks(view.state);
  const out: Range<Decoration>[] = [];
  if (!res) return out;
  const doc = view.state.doc;
  for (const c of res.chunks) {
    const [cFrom, cTo, oFrom, oTo] = isA ? [c.fromA, c.toA, c.fromB, c.toB] : [c.fromB, c.toB, c.fromA, c.toA];
    if (cTo < from || cFrom > to) continue;
    // A new or deleted file: one bar on the empty side.
    if (cFrom >= cTo) {
      if (doc.length === 0 && oFrom < oTo) out.push(emptyRange.range(0));
      continue;
    }
    if (oFrom >= oTo) continue;
    for (const ch of c.changes) {
      const [s, e] = isA ? [ch.fromA, ch.toA] : [ch.fromB, ch.toB];
      if (s === e && (isA ? ch.fromB < ch.toB : ch.fromA < ch.toA)) out.push(emptyRange.range(Math.min(doc.length, cFrom + s)));
    }
  }
  return out;
}

function lineClasses(side: Side) {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = this.build(view);
      }
      update(u: ViewUpdate) {
        if (u.docChanged || u.viewportChanged || u.transactions.some((tr) => tr.effects.some((e) => e.is(setComments))) || getChunks(u.startState)?.chunks !== getChunks(u.state)?.chunks)
          this.decorations = this.build(u.view);
      }
      build(view: EditorView) {
        const info = lineInfo(view, side);
        const comments = view.state.field(commentsField);
        const out: Range<Decoration>[] = [];
        // The viewport, not visibleRanges: those are empty for an empty document.
        const { from, to } = view.viewport;
        for (let pos = from; pos <= to; ) {
          const line = view.state.doc.lineAt(pos);
          if (info?.changed[line.number]) out.push(changedLine.range(line.from));
          if (info?.full[line.number]) out.push(fullLine.range(line.from));
          if (side !== "a" && comments.has(line.number)) out.push(commentedLine.range(line.from));
          pos = line.to + 1;
        }
        if (side !== "unified") out.push(...emptyRanges(view, side === "a", from, to));
        return Decoration.set(out, true);
      }
    },
    { decorations: (p) => p.decorations },
  );
}

// Unified: mark the fully deleted rows inside the deleted-chunk widgets (their DOM is merge's own).
const deletedRows = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {
      this.schedule();
    }
    update(u: ViewUpdate) {
      if (u.viewportChanged || u.docChanged || u.geometryChanged) this.schedule();
    }
    schedule() {
      this.view.requestMeasure({
        read: () => null,
        write: () => {
          const res = getChunks(this.view.state);
          if (!res) return;
          const a = originalOf(this.view);
          // A deleted file: Monaco highlights the text of its rows, not whole rows.
          const whole = this.view.state.doc.length === 0;
          for (const dom of this.view.contentDOM.querySelectorAll<HTMLElement>(".cm-deletedChunk:not([data-rv])")) {
            const pos = this.view.posAtDOM(dom);
            const c = res.chunks.find((ch) => ch.fromB === pos && ch.fromA < ch.toA);
            if (!c) continue;
            dom.dataset.rv = "1";
            const rows = dom.querySelectorAll<HTMLElement>(".cm-deletedLine");
            const first = a.lineAt(c.fromA).number;
            changedLines(a, [c], true, (n, full) => rows[n - first]?.classList.toggle("rv-full", full && !whole));
          }
        },
      });
    }
  },
);

// ── the margin: glyph · line number(s) · sign ─────────────────────────────────

const ROW = 19;

class LineMarker extends GutterMarker {
  constructor(
    readonly num: number | null,
    readonly orig: number | null | undefined,
    readonly glyph: "add" | "comment" | null,
    readonly active: boolean,
    readonly cls: string,
  ) {
    super();
    this.elementClass = cls;
  }
  eq(o: GutterMarker) {
    return o instanceof LineMarker && o.num === this.num && o.orig === this.orig && o.glyph === this.glyph && o.active === this.active && o.cls === this.cls;
  }
  toDOM() {
    return row(this.orig, this.num, this.glyph, this.active);
  }
}

function row(orig: number | null | undefined, num: number | null, glyph: "add" | "comment" | null, active = false) {
  const dom = document.createElement("div");
  dom.className = "rv-gm";
  if (orig !== undefined) {
    const o = dom.appendChild(document.createElement("span"));
    o.className = "rv-gm-orig";
    o.textContent = orig ? String(orig) : "";
  }
  const g = dom.appendChild(document.createElement("span"));
  g.className = "rv-gm-glyph";
  if (glyph) g.appendChild(document.createElement("span")).className = glyph === "add" ? "rv-glyph-add" : "rv-glyph-comment";
  const n = dom.appendChild(document.createElement("span"));
  n.className = "rv-gm-num" + (active ? " active" : "");
  n.textContent = num === null ? "" : String(num);
  dom.appendChild(document.createElement("span")).className = "rv-gm-sign";
  return dom;
}

class DeletedRowsMarker extends GutterMarker {
  elementClass = "rv-g-del rv-g-rows";
  constructor(readonly first: number, readonly count: number) {
    super();
  }
  eq(o: GutterMarker) {
    return o instanceof DeletedRowsMarker && o.first === this.first && o.count === this.count;
  }
  toDOM() {
    const dom = document.createElement("div");
    for (let i = 0; i < this.count; i++) dom.appendChild(row(this.first + i, null, null)).style.height = `${ROW}px`;
    return dom;
  }
}

class FoldMarker extends GutterMarker {
  elementClass = "rv-g-fold";
  eq(o: GutterMarker) {
    return o instanceof FoldMarker;
  }
  toDOM() {
    const dom = document.createElement("div");
    dom.className = "rv-unfold";
    dom.title = "Show Unchanged Region";
    return dom;
  }
}
const foldMarker = new FoldMarker();

const isCollapse = (w: unknown) => (w as { type?: string } | null)?.type === "collapsed-unchanged-code";

/** The text line under a y position (null over widgets: deleted rows, filler, collapsed regions). */
function lineAtY(view: EditorView, clientY: number): number | null {
  const h = clientY - view.documentTop;
  let block: BlockInfo = view.lineBlockAtHeight(h);
  if (Array.isArray(block.type)) block = (block.type as readonly BlockInfo[]).find((b) => h >= b.top && h < b.bottom) ?? block;
  if (block.type !== BlockType.Text) return null;
  return view.state.doc.lineAt(block.from).number;
}

/** The line's own block (without filler or deleted rows drawn next to it). */
function textBlock(view: EditorView, pos: number): BlockInfo {
  const block = view.lineBlockAt(pos);
  if (!Array.isArray(block.type)) return block;
  return (block.type as readonly BlockInfo[]).find((b) => b.type === BlockType.Text && b.from <= pos && b.to >= pos) ?? block;
}

/** Expand the collapsed region drawn by this block (merge's widget also expands the other side). */
function expand(view: EditorView, block: BlockInfo) {
  for (const el of view.contentDOM.querySelectorAll<HTMLElement>(".cm-collapsedLines")) {
    if (view.posAtDOM(el) === block.from) {
      el.click();
      return;
    }
  }
}

function margin(side: Side, opts: DiffViewOptions): Extension {
  const commentable = side !== "a" && opts.commentable;
  return gutter({
    class: `rv-gutter rv-gutter-${side}`,
    lineMarker(view, line) {
      const info = lineInfo(view, side);
      const num = view.state.doc.lineAt(line.from).number;
      const changed = !!info?.changed[num];
      let glyph: "add" | "comment" | null = null;
      if (commentable) {
        if (view.state.field(commentsField).has(num)) glyph = "comment";
        else if (view.state.field(hoverField) === num) glyph = "add";
      }
      const active = view.state.doc.lineAt(view.state.selection.main.head).number === num;
      const cls = changed ? (side === "a" ? "rv-g-del" : side === "unified" ? "rv-g-ins rv-g-uni" : "rv-g-ins") : "";
      return new LineMarker(num, side === "unified" ? (info?.orig?.[num] ?? 0) : undefined, glyph, active, cls);
    },
    widgetMarker(view, widget, block) {
      if (isCollapse(widget)) return foldMarker;
      if (side !== "unified") return null;
      const c = getChunks(view.state)?.chunks.find((ch) => ch.fromB === block.from && ch.fromA < ch.toA);
      if (!c) return null;
      const a = originalOf(view);
      const first = a.lineAt(c.fromA).number;
      return new DeletedRowsMarker(first, a.lineAt(c.endA).number - first + 1);
    },
    lineMarkerChange: (u) =>
      u.selectionSet || u.transactions.some((tr) => tr.effects.some((e) => e.is(setHover) || e.is(setComments))),
    domEventHandlers: {
      mousedown(view, block, event) {
        const e = event as MouseEvent;
        if (e.button !== 0) return false;
        const target = e.target as HTMLElement;
        if (target.closest(".rv-g-fold")) {
          const h = e.clientY - view.documentTop;
          const b = Array.isArray(block.type) ? (block.type as readonly BlockInfo[]).find((x) => h >= x.top && h < x.bottom) : block;
          if (b) expand(view, b);
          e.preventDefault();
          return true;
        }
        if (target.closest(".rv-gm-orig")) return false;
        const line = lineAtY(view, e.clientY);
        if (line === null) return false;
        // A click on the number puts the cursor on the line (⌘C then copies it, as in Monaco); any click in the margin comments.
        if (target.closest(".rv-gm-num")) view.dispatch({ selection: { anchor: view.state.doc.line(line).from } });
        if (commentable) {
          e.preventDefault();
          opts.onComment(line);
          return true;
        }
        return false;
      },
    },
  });
}

// ── hover "+" and the comment hover ───────────────────────────────────────────

type Listeners = { [K in keyof HTMLElementEventMap]?: (e: HTMLElementEventMap[K]) => void };

/** Listeners on the whole editor, margin included (CodeMirror's own handlers only see the text). */
function editorListeners(make: (view: EditorView) => { on: Listeners; destroy?(): void }) {
  return ViewPlugin.define((view) => {
    const { on, destroy } = make(view);
    const entries = Object.entries(on) as [string, EventListener][];
    for (const [type, fn] of entries) view.dom.addEventListener(type, fn);
    return {
      destroy() {
        for (const [type, fn] of entries) view.dom.removeEventListener(type, fn);
        destroy?.();
      },
    };
  });
}

function hoverTracking(): Extension {
  return editorListeners((view) => {
    const set = (line: number | null) => {
      if (line !== view.state.field(hoverField)) view.dispatch({ effects: setHover.of(line) });
    };
    return {
      on: {
        mousemove: (e) => set((e.target as HTMLElement).closest(".cm-panels, .rv-sb-v, .rv-sb-h") ? null : lineAtY(view, e.clientY)),
        mouseleave: () => set(null),
      },
    };
  });
}

/** Monaco shows a comment's text when hovering its glyph. */
function commentHover(host: HTMLElement): Extension {
  return editorListeners((view) => {
    let timer = 0;
    let tip: HTMLElement | null = null;
    const hide = () => {
      clearTimeout(timer);
      tip?.remove();
      tip = null;
    };
    view.scrollDOM.addEventListener("scroll", hide);
    return {
      on: {
        mouseover(e) {
          const glyph = (e.target as HTMLElement).closest(".rv-glyph-comment");
          if (!glyph) return hide();
          if (tip) return;
          clearTimeout(timer);
          timer = window.setTimeout(() => {
            const line = lineAtY(view, e.clientY);
            const text = line === null ? undefined : view.state.field(commentsField).get(line);
            if (!text || !glyph.isConnected) return;
            const g = glyph.getBoundingClientRect();
            const h = host.getBoundingClientRect();
            tip = host.appendChild(document.createElement("div"));
            tip.className = "rv-hover";
            tip.textContent = text;
            tip.style.left = `${g.right - h.left + 4}px`;
            tip.style.top = `${(g.top + g.bottom) / 2 - h.top}px`;
          }, 300);
        },
        mouseleave: hide,
      },
      destroy() {
        hide();
        view.scrollDOM.removeEventListener("scroll", hide);
      },
    };
  });
}

// ── context menu ──────────────────────────────────────────────────────────────

function contextMenu(side: Side, opts: DiffViewOptions): Extension {
  return editorListeners((view) => ({
    on: {
      contextmenu(e) {
        if ((e.target as HTMLElement).closest(".cm-panels")) return;
        e.preventDefault();
        const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
        const sel = view.state.selection.main;
        if (pos !== null && (pos < sel.from || pos > sel.to)) view.dispatch({ selection: { anchor: pos } });
        view.focus();
        const line = side === "a" ? null : view.state.doc.lineAt(view.state.selection.main.head).number;
        opts.onMenu({ x: e.clientX, y: e.clientY, view, line });
      },
    },
  }));
}

// ── overlay scrollbars (Monaco's: drawn over the text, shown while the pointer is in) ──

const SB_V = 14;
const SB_H = 10;
const MIN_SLIDER = 20;

const scrollbars = ViewPlugin.fromClass(
  class {
    dom: HTMLElement;
    vTrack: HTMLElement;
    vSlider: HTMLElement;
    hTrack: HTMLElement;
    hSlider: HTMLElement;
    lane: HTMLElement;
    hideTimer = 0;
    ro: ResizeObserver;
    constructor(readonly view: EditorView) {
      const mk = (cls: string, parent: HTMLElement) => parent.appendChild(Object.assign(document.createElement("div"), { className: cls }));
      this.dom = mk("rv-sb", view.dom);
      this.vTrack = mk("rv-sb-v", this.dom);
      this.lane = mk("rv-sb-lane", this.vTrack);
      this.vSlider = mk("rv-sb-slider", this.vTrack);
      this.hTrack = mk("rv-sb-h", this.dom);
      this.hSlider = mk("rv-sb-slider", this.hTrack);
      this.drag(this.vSlider, this.vTrack, "y");
      this.drag(this.hSlider, this.hTrack, "x");
      view.scrollDOM.addEventListener("scroll", this.onScroll);
      view.dom.addEventListener("mouseenter", this.show);
      view.dom.addEventListener("mouseleave", this.fade);
      this.ro = new ResizeObserver(() => this.layout());
      this.ro.observe(view.scrollDOM);
      this.layout();
    }
    onScroll = () => {
      this.layout();
      this.show();
      if (!this.view.dom.matches(":hover")) this.fade();
    };
    show = () => {
      clearTimeout(this.hideTimer);
      this.dom.classList.add("visible");
    };
    fade = () => {
      clearTimeout(this.hideTimer);
      this.hideTimer = window.setTimeout(() => this.dom.classList.remove("visible"), 800);
    };
    drag(slider: HTMLElement, track: HTMLElement, axis: "x" | "y") {
      const s = this.view.scrollDOM;
      track.addEventListener("mousedown", (e) => {
        e.preventDefault();
        e.stopPropagation();
        const r = track.getBoundingClientRect();
        const sr = slider.getBoundingClientRect();
        const along = axis === "y" ? e.clientY : e.clientX;
        const onSlider = e.target === slider;
        if (!onSlider) {
          // Monaco: a click on the track pages toward it.
          const before = along < (axis === "y" ? sr.top : sr.left);
          if (axis === "y") s.scrollTop += (before ? -1 : 1) * s.clientHeight;
          else s.scrollLeft += (before ? -1 : 1) * s.clientWidth;
          return;
        }
        const start = along;
        const startScroll = axis === "y" ? s.scrollTop : s.scrollLeft;
        const ratio = axis === "y" ? s.scrollHeight / r.height : s.scrollWidth / r.width;
        slider.classList.add("active");
        const move = (m: MouseEvent) => {
          const d = ((axis === "y" ? m.clientY : m.clientX) - start) * ratio;
          if (axis === "y") s.scrollTop = startScroll + d;
          else s.scrollLeft = startScroll + d;
        };
        const up = () => {
          slider.classList.remove("active");
          window.removeEventListener("mousemove", move);
          window.removeEventListener("mouseup", up);
        };
        window.addEventListener("mousemove", move);
        window.addEventListener("mouseup", up);
      });
    }
    update(u: ViewUpdate) {
      if (u.geometryChanged || u.heightChanged || u.selectionSet || u.docChanged || u.viewportChanged || u.transactions.some((tr) => tr.effects.length))
        this.view.requestMeasure({ read: () => null, write: () => this.layout() });
    }
    layout() {
      const s = this.view.scrollDOM;
      const gutterW = (this.view.dom.querySelector(".cm-gutters") as HTMLElement | null)?.offsetWidth ?? 0;
      const H = s.clientHeight;
      const W = s.clientWidth;
      // For the collapsed-region bars: as wide as the editor, not the (wider) content.
      this.view.dom.style.setProperty("--rv-gutter-w", `${gutterW}px`);
      this.view.dom.style.setProperty("--rv-view-w", `${W}px`);
      // vertical
      const vh = s.scrollHeight > H ? Math.max(MIN_SLIDER, (H * H) / s.scrollHeight) : 0;
      this.vTrack.style.height = `${H}px`;
      this.vSlider.style.display = vh ? "" : "none";
      if (vh) {
        this.vSlider.style.height = `${vh}px`;
        this.vSlider.style.top = `${(s.scrollTop / (s.scrollHeight - H)) * (H - vh)}px`;
      }
      // horizontal
      const trackW = W - gutterW - SB_V;
      const hw = s.scrollWidth > W ? Math.max(MIN_SLIDER, (trackW * W) / s.scrollWidth) : 0;
      this.hTrack.style.left = `${gutterW}px`;
      this.hTrack.style.width = `${Math.max(0, trackW)}px`;
      this.hTrack.style.top = `${H - SB_H}px`;
      this.hTrack.style.display = hw ? "" : "none";
      if (hw) {
        this.hSlider.style.width = `${hw}px`;
        this.hSlider.style.left = `${(s.scrollLeft / (s.scrollWidth - W)) * (trackW - hw)}px`;
      }
      this.drawLane(H);
    }
    /** The editor's overview lane: the cursor, and Find's matches. */
    drawLane(H: number) {
      const { view } = this;
      const total = Math.max(view.contentHeight, view.scrollDOM.clientHeight);
      const ticks: string[] = [];
      const tick = (pos: number, cls: string) => {
        const b = textBlock(view, pos);
        ticks.push(`<div class="${cls}" style="top:${Math.round(((b.top + b.bottom) / 2 / total) * H) - 1}px"></div>`);
      };
      if (searchPanelOpen(view.state)) {
        const q = getSearchQuery(view.state);
        if (q.valid && q.search) {
          const cur = q.getCursor(view.state);
          for (let m = cur.next(), n = 0; !m.done && n < 2000; m = cur.next(), n++) tick(m.value.from, "rv-lane-find");
        }
      }
      tick(view.state.selection.main.head, "rv-lane-cursor");
      this.lane.innerHTML = ticks.join("");
    }
    destroy() {
      clearTimeout(this.hideTimer);
      this.ro.disconnect();
      this.view.scrollDOM.removeEventListener("scroll", this.onScroll);
      this.view.dom.removeEventListener("mouseenter", this.show);
      this.view.dom.removeEventListener("mouseleave", this.fade);
      this.dom.remove();
    }
  },
);

// ── the diff overview ruler (right edge: deletions left half, insertions right half) ──

class Overview {
  dom: HTMLElement;
  private marks: HTMLElement;
  private viewport: HTMLElement;
  private raf = 0;
  constructor(
    host: HTMLElement,
    readonly mod: EditorView,
    readonly orig: EditorView | null,
  ) {
    this.dom = host.appendChild(Object.assign(document.createElement("div"), { className: "rv-overview" }));
    this.viewport = this.dom.appendChild(Object.assign(document.createElement("div"), { className: "rv-ov-viewport" }));
    this.marks = this.dom.appendChild(Object.assign(document.createElement("div"), { className: "rv-ov-marks" }));
    this.dom.addEventListener("mousedown", this.onDown);
    mod.scrollDOM.addEventListener("scroll", this.schedule);
  }
  private onDown = (e: MouseEvent) => {
    e.preventDefault();
    const s = this.mod.scrollDOM;
    const r = this.dom.getBoundingClientRect();
    const scale = r.height / Math.max(s.scrollHeight, s.clientHeight);
    const vp = this.viewport.getBoundingClientRect();
    let offset = e.clientY - vp.top;
    if (e.clientY < vp.top || e.clientY > vp.bottom) {
      s.scrollTop = (e.clientY - r.top) / scale - s.clientHeight / 2;
      offset = vp.height / 2;
    }
    this.viewport.classList.add("active");
    const move = (m: MouseEvent) => (s.scrollTop = (m.clientY - offset - r.top) / scale);
    const up = () => {
      this.viewport.classList.remove("active");
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  schedule = () => {
    cancelAnimationFrame(this.raf);
    this.raf = requestAnimationFrame(() => this.draw());
  };
  draw() {
    const { mod, orig } = this;
    const s = mod.scrollDOM;
    const H = this.dom.clientHeight;
    const total = Math.max(s.scrollHeight, s.clientHeight);
    const scale = H / total;
    this.viewport.style.top = `${s.scrollTop * scale}px`;
    this.viewport.style.height = `${Math.min(H, s.clientHeight * scale)}px`;
    const out: string[] = [];
    const mark = (cls: string, top: number, bottom: number) =>
      out.push(`<div class="${cls}" style="top:${Math.floor(top * scale)}px;height:${Math.max(2, Math.ceil((bottom - top) * scale))}px"></div>`);
    // Runs of changed lines, from the same per-line info the margin uses.
    const runs = (view: EditorView, side: Side, cls: string) => {
      const info = lineInfo(view, side);
      const doc = view.state.doc;
      for (let n = 1; info && n <= doc.lines; n++) {
        if (!info.changed[n]) continue;
        const start = n;
        while (n < doc.lines && info.changed[n + 1]) n++;
        mark(cls, textBlock(view, doc.line(start).from).top, textBlock(view, doc.line(n).from).bottom);
      }
    };
    runs(mod, orig ? "b" : "unified", "rv-ov-ins");
    if (orig) runs(orig, "a", "rv-ov-del");
    else {
      // Inline: the deleted rows are the widget block above a chunk's first line.
      for (const c of getChunks(mod.state)?.chunks ?? []) {
        if (c.fromA >= c.toA) continue;
        const blk = mod.lineBlockAt(c.fromB);
        const parts = Array.isArray(blk.type) ? (blk.type as readonly BlockInfo[]) : [blk];
        const w = parts.find((b) => b.type === BlockType.WidgetBefore && !isCollapse(b.widget));
        if (w) mark("rv-ov-del", w.top, w.bottom);
      }
    }
    this.marks.innerHTML = out.join("");
  }

  destroy() {
    cancelAnimationFrame(this.raf);
    this.mod.scrollDOM.removeEventListener("scroll", this.schedule);
    this.dom.remove();
  }
}

// ── the view ──────────────────────────────────────────────────────────────────

export class DiffView {
  /** The modified file's editor (where comments and reveal apply). */
  readonly modified: EditorView;
  private merge: MergeView | null = null;
  private overview: Overview;
  private cleanup: (() => void)[] = [];

  constructor(readonly opts: DiffViewOptions) {
    const host = document.createElement("div");
    host.className = "rv-cm";
    opts.parent.appendChild(host);
    const base = (doc: string) => editorExtensions({ doc, lang: opts.lang, readOnlyText: opts.readOnlyText });
    const shared = (side: Side): Extension => [
      commentsField,
      hoverField,
      margin(side, opts),
      lineClasses(side),
      scrollbars,
      contextMenu(side, opts),
      EditorView.updateListener.of((u) => {
        if (u.geometryChanged || u.heightChanged || u.viewportChanged) this.overview?.schedule();
      }),
      side === "a" ? [] : [hoverTracking(), commentHover(opts.parent)],
    ];

    if (opts.sideBySide) {
      this.merge = new MergeView({
        a: { doc: opts.original, extensions: [base(opts.original), shared("a")] },
        b: { doc: opts.modified, extensions: [base(opts.modified), shared("b")] },
        parent: host,
        highlightChanges: true,
        gutter: false,
        collapseUnchanged: COLLAPSE,
        diffConfig: DIFF_CONFIG,
      });
      this.modified = this.merge.b;
      this.syncScroll(this.merge.a, this.merge.b);
      // The original's margin and lines were drawn before its sibling existed: redraw them.
      this.merge.a.dispatch({ effects: setComments.of(new Map()) });    } else {
      this.modified = new EditorView({
        doc: opts.modified,
        parent: host,
        extensions: [
          base(opts.modified),
          unifiedMergeView({
            original: opts.original,
            highlightChanges: true,
            gutter: false,
            syntaxHighlightDeletions: true,
            mergeControls: false,
            collapseUnchanged: COLLAPSE,
            diffConfig: DIFF_CONFIG,
          }),
          shared("unified"),
          deletedRows,
        ],
      });
    }
    this.overview = new Overview(opts.parent, this.modified, this.merge?.a ?? null);
    const ro = new ResizeObserver(() => this.overview.schedule());
    ro.observe(opts.parent);
    this.cleanup.push(() => ro.disconnect(), () => host.remove());
    this.overview.schedule();
  }

  /** Side by side: both editors scroll together, vertically and horizontally (as Monaco's). */
  private syncScroll(a: EditorView, b: EditorView) {
    let busy = false;
    const link = (from: EditorView, to: EditorView) => () => {
      if (busy) return;
      busy = true;
      to.scrollDOM.scrollTop = from.scrollDOM.scrollTop;
      to.scrollDOM.scrollLeft = from.scrollDOM.scrollLeft;
      requestAnimationFrame(() => (busy = false));
    };
    const ab = link(a, b);
    const ba = link(b, a);
    a.scrollDOM.addEventListener("scroll", ab);
    b.scrollDOM.addEventListener("scroll", ba);
    this.cleanup.push(() => {
      a.scrollDOM.removeEventListener("scroll", ab);
      b.scrollDOM.removeEventListener("scroll", ba);
    });
  }

  setComments(comments: { line: number; text: string }[]) {
    const map = new Map<number, string>();
    for (const c of comments) map.set(c.line, map.has(c.line) ? `${map.get(c.line)}\n\n${c.text}` : c.text);
    this.modified.dispatch({ effects: setComments.of(map) });
  }

  /** Scroll the modified file's line to the middle and put the cursor there; false if it isn't there. */
  reveal(line: number): boolean {
    const view = this.modified;
    if (line < 1 || line > view.state.doc.lines) return false;
    const pos = view.state.doc.line(line).from;
    const blk = view.lineBlockAt(pos);
    if (blk.type === BlockType.WidgetRange && isCollapse(blk.widget)) expand(view, blk);
    view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
    return true;
  }

  destroy() {
    this.overview.destroy();
    if (this.merge) this.merge.destroy();
    else this.modified.destroy();
    for (const f of this.cleanup) f();
  }
}
