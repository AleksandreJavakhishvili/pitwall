// Find, laid out like Monaco's find widget: a floating box at the top right
// with the query, Aa / ab / .* toggles, "n of m", previous / next, find in
// selection and close. Typing moves to the first match from where Find opened.
import { EditorSelection } from "@codemirror/state";
import { EditorView, runScopeHandlers, type Panel, type ViewUpdate } from "@codemirror/view";
import { SearchQuery, closeSearchPanel, findNext, findPrevious, getSearchQuery, setSearchQuery } from "@codemirror/search";
import { mergeViewSiblings } from "@codemirror/merge";

const MAX_COUNT = 19999;
const WIDTH = 419;

/** Space Monaco keeps above the first line while Find is open (its height). */
const ZONE = 33;

// Shapes after Monaco's codicons (16px).
const ICONS: Record<string, string> = {
  up: '<path d="M8 2.3 13.7 8l-.7.7-4.5-4.5V14h-1V4.2L3 8.7 2.3 8z"/>',
  down: '<path d="M8 13.7 2.3 8l.7-.7 4.5 4.5V2h1v9.8L13 7.3l.7.7z"/>',
  selection: '<path d="M2 3.5h12v1H2zm0 4h12v1H2zm0 4h9v1H2z"/>',
  close: '<path d="m8 8.7 4.6 4.6.7-.7L8.7 8l4.6-4.6-.7-.7L8 7.3 3.4 2.7l-.7.7L7.3 8l-4.6 4.6.7.7z"/>',
};

function icon(name: string) {
  return `<svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">${ICONS[name]}</svg>`;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, attrs: Record<string, string> = {}) {
  const e = document.createElement(tag);
  e.className = cls;
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
}

export function findPanel(view: EditorView): Panel {
  return new FindPanel(view);
}

class FindPanel implements Panel {
  dom: HTMLElement;
  top = true;
  private input: HTMLInputElement;
  private count: HTMLElement;
  private toggles: Record<"case" | "word" | "regexp" | "inSel", HTMLElement>;
  private nav: HTMLElement[];
  /** Where typing starts looking (the selection when Find opened). */
  private origin: number;
  private inSel: { from: number; to: number } | null = null;

  constructor(readonly view: EditorView) {
    const q = getSearchQuery(view.state);
    this.origin = view.state.selection.main.from;
    this.dom = el("div", "rv-find", { role: "dialog", "aria-label": "Find" });
    const part = this.dom.appendChild(el("div", "rv-find-part"));
    const box = part.appendChild(el("div", "rv-find-input"));
    this.input = box.appendChild(
      el("input", "", { placeholder: "Find", "aria-label": "Find", "main-field": "true", spellcheck: "false", autocomplete: "off" }),
    );
    this.input.value = q.search;
    const toggle = (cls: string, label: string, text: string) => {
      const b = box.appendChild(el("div", `rv-find-toggle ${cls}`, { role: "checkbox", "aria-label": label, title: label, tabindex: "0" }));
      b.innerHTML = text;
      return b;
    };
    this.toggles = {
      case: toggle("rv-find-case", "Match Case", "Aa"),
      word: toggle("rv-find-word", "Match Whole Word", "<u>ab</u>"),
      regexp: toggle("rv-find-re", "Use Regular Expression", ".*"),
      inSel: el("div", ""),
    };
    this.count = part.appendChild(el("div", "rv-find-count"));
    const actions = part.appendChild(el("div", "rv-find-actions"));
    const button = (parent: HTMLElement, cls: string, name: string, label: string) => {
      const b = parent.appendChild(el("div", cls, { role: "button", "aria-label": label, title: label, tabindex: "0" }));
      b.innerHTML = icon(name);
      return b;
    };
    this.nav = [button(actions, "rv-find-btn", "up", "Previous Match (⇧Enter)"), button(actions, "rv-find-btn", "down", "Next Match (Enter)")];
    this.toggles.inSel = button(actions, "rv-find-btn rv-find-insel", "selection", "Find in Selection");
    const close = button(this.dom, "rv-find-btn rv-find-close", "close", "Close (Escape)");
    this.toggles.case.classList.toggle("on", q.caseSensitive);
    this.toggles.word.classList.toggle("on", q.wholeWord);
    this.toggles.regexp.classList.toggle("on", q.regexp);

    this.input.addEventListener("input", () => this.commit(true));
    this.input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        (e.shiftKey ? findPrevious : findNext)(view);
      } else if (runScopeHandlers(view, e, "search-panel")) e.preventDefault();
    });
    const press = (b: HTMLElement, fn: () => void) => {
      b.addEventListener("mousedown", (e) => e.preventDefault());
      b.addEventListener("click", fn);
      b.addEventListener("keydown", (e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          fn();
        }
      });
    };
    for (const k of ["case", "word", "regexp"] as const)
      press(this.toggles[k], () => {
        this.toggles[k].classList.toggle("on");
        this.commit(true);
      });
    press(this.toggles.inSel, () => {
      const sel = view.state.selection.main;
      this.inSel = this.inSel || sel.empty ? null : { from: sel.from, to: sel.to };
      this.toggles.inSel.classList.toggle("on", !!this.inSel);
      this.commit(false);
    });
    press(this.nav[0], () => findPrevious(view));
    press(this.nav[1], () => findNext(view));
    press(close, () => closeSearchPanel(view));
    this.updateCount();
  }

  mount() {
    this.layout();
    this.input.focus();
    this.input.select();
    requestAnimationFrame(() => this.dom.classList.add("visible"));
    this.zone(true);
  }

  destroy() {
    this.zone(false);
  }

  /**
   * Monaco keeps the widget's height free above the first line while Find is open
   * (hatched in the other editor), and shifts a scrolled view so nothing jumps.
   */
  private zone(open: boolean) {
    const root = this.view.dom.closest<HTMLElement>(".rv-cm");
    if (!root) return;
    const views = [this.view, ...(mergeViewSiblings(this.view) ? Object.values(mergeViewSiblings(this.view)!) : [])].filter(
      (v, i, all) => all.indexOf(v) === i,
    );
    // data-find-a / data-find-b: which editor has it open (the inline view counts as "b").
    const key = this.view.dom.classList.contains("cm-merge-a") ? "findA" : "findB";
    const anyOpen = () => root.dataset.findA !== undefined || root.dataset.findB !== undefined;
    const was = anyOpen();
    if (open) root.dataset[key] = "";
    else delete root.dataset[key];
    if (was === anyOpen()) return;
    for (const v of views) {
      if (v.scrollDOM.scrollTop > 0) v.scrollDOM.scrollTop += open ? ZONE : -ZONE;
      v.requestMeasure();
    }
  }

  /** Monaco's sizes: 419 px, narrower in a small editor, and no count when very small. */
  private layout() {
    const w = this.view.dom.clientWidth;
    this.dom.style.width = WIDTH + 28 >= w ? `${Math.max(0, w - 84)}px` : "";
    this.dom.classList.toggle("narrow", WIDTH + 28 - 69 >= w);
  }

  private commit(jump: boolean) {
    const range = this.inSel;
    const query = new SearchQuery({
      search: this.input.value,
      caseSensitive: this.toggles.case.classList.contains("on"),
      wholeWord: this.toggles.word.classList.contains("on"),
      regexp: this.toggles.regexp.classList.contains("on"),
      test: range ? (_m, _s, from, to) => from >= range.from && to <= range.to : undefined,
    });
    if (query.eq(getSearchQuery(this.view.state)) && !range) return;
    const effects = [setSearchQuery.of(query)];
    const first = jump && query.valid && query.search ? this.firstMatch(query) : null;
    this.view.dispatch(
      first
        ? { effects: [...effects, EditorView.scrollIntoView(first.from, { y: "nearest" })], selection: EditorSelection.range(first.from, first.to) }
        : { effects },
    );
  }

  private firstMatch(query: SearchQuery) {
    const { state } = this.view;
    let next = query.getCursor(state, this.origin).next();
    if (next.done) next = query.getCursor(state, 0, this.origin).next();
    return next.done ? null : next.value;
  }

  update(u: ViewUpdate) {
    if (u.geometryChanged) this.layout();
    for (const tr of u.transactions)
      for (const e of tr.effects)
        if (e.is(setSearchQuery) && e.value.search !== this.input.value) this.input.value = e.value.search;
    if (u.docChanged || u.selectionSet || u.transactions.some((tr) => tr.effects.some((e) => e.is(setSearchQuery)))) this.updateCount();
  }

  private updateCount() {
    const q = getSearchQuery(this.view.state);
    let total = 0;
    let current = 0;
    if (q.valid && q.search) {
      const sel = this.view.state.selection.main;
      const cur = q.getCursor(this.view.state);
      for (let m = cur.next(); !m.done && total < MAX_COUNT; m = cur.next()) {
        total++;
        if (m.value.from === sel.from && m.value.to === sel.to) current = total;
      }
    }
    this.count.textContent = total ? `${current || "?"} of ${total}${total >= MAX_COUNT ? "+" : ""}` : "No results";
    this.count.classList.toggle("none", !!q.search && !total);
    for (const b of this.nav) b.classList.toggle("disabled", !total);
  }
}
