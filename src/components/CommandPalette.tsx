import { useMemo, useRef, useState, type ReactNode } from "react";
import type { AgentView } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { STATUS_WORD } from "../lib/status";
import { Modal } from "./Modal";
import { StatusGlyph } from "./StatusGlyph";
import { Kbd } from "./Kbd";
import { PRESET_LABEL, PRESETS, type Preset } from "../layout/tree";
import { DENSITIES, DENSITY_CELLS, DENSITY_LABEL, type Density } from "../layout/density";

interface Commands {
  newAgent(): void;
  nextBlocked(): void;
  toggleSidebar(): void;
  toggleRight(): void;
  toggleWall(): void;
  toggleReview?(): void;
  moveToWindow(): void;
  preset(p: Preset): void;
  /** Presets that fit the current space (default: all). */
  presets?: Preset[];
  /** Set the global density, or the current space's (`null` = follow global). */
  density(d: Density | null, scope: "global" | "space"): void;
  settings(): void;
}

interface Item {
  id: string;
  label: ReactNode;
  search: string;
  icon?: ReactNode;
  hint?: ReactNode;
  /** Return false to keep the palette open. */
  run(): boolean | void;
}

interface Props {
  agents: AgentView[];
  selectedId: string | null;
  commands: Commands;
  onClose(): void;
}

/** "api-fix: do the thing" or "queue api-fix: do the thing" → target + verbatim text. */
function parseQueue(q: string, agents: AgentView[]): { agent: AgentView; text: string } | null {
  const m = /^(?:queue\s+(?:for\s+)?)?([a-z][a-z0-9_-]{0,31})\s*:\s?([\s\S]+)$/i.exec(q);
  if (!m) return null;
  const agent = agents.find((a) => a.name === m[1]);
  if (!agent || !m[2].trim()) return null;
  return { agent, text: m[2] };
}

function matches(search: string, q: string): boolean {
  const s = search.toLowerCase();
  return q
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => s.includes(w));
}

export function CommandPalette({ agents, selectedId, commands, onClose }: Props) {
  const { showAgent, run, patch } = useActions();
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const items: Item[] = useMemo(() => {
    const queue = parseQueue(q, agents);
    if (queue) {
      return [
        {
          id: "queue",
          icon: <span className="pal-icon">↳</span>,
          label: (
            <>
              Queue for <strong>{queue.agent.name}</strong>: <span className="pal-quote">{queue.text}</span>
            </>
          ),
          search: "",
          hint: <Kbd>↵</Kbd>,
          run: () => {
            run(api.queueAdd(queue.agent.id, queue.text), "queue prompt").then((a) => a && patch(a));
          },
        },
      ];
    }

    const blocked = agents.filter((a) => a.status === "blocked");
    const base: Item[] = [
      ...agents.map<Item>((a) => ({
        id: `go-${a.id}`,
        icon: <StatusGlyph status={a.status} size="sm" />,
        label: (
          <>
            {a.name} <span className="pal-sub">{STATUS_WORD[a.status]} · {a.projectDisplay}</span>
          </>
        ),
        search: `go jump ${a.name} ${a.status} ${a.projectDisplay}`,
        hint: a.id === selectedId ? <span className="pal-sub">current</span> : undefined,
        run: () => showAgent(a.id),
      })),
      ...(blocked.length
        ? [
            {
              id: "next-blocked",
              icon: <span className="pal-icon pal-icon-blocked">▲</span>,
              label: "Jump to next blocked",
              search: "jump next blocked needs you",
              hint: <Kbd>⌘J</Kbd>,
              run: commands.nextBlocked,
            },
          ]
        : []),
      { id: "new", icon: <span className="pal-icon">+</span>, label: "New agent", search: "new agent create start", hint: <Kbd>⌘N</Kbd>, run: commands.newAgent },
      ...agents.map<Item>((a) => ({
        id: `q-${a.id}`,
        icon: <span className="pal-icon">↳</span>,
        label: (
          <>
            Queue for {a.name}: <span className="pal-sub">…</span>
          </>
        ),
        search: `queue prompt for ${a.name}`,
        run: () => {
          setQ(`${a.name}: `);
          inputRef.current?.focus();
          return false;
        },
      })),
      { id: "wall", icon: <span className="pal-icon">▦</span>, label: "Toggle Wall (all terminals)", search: "wall overview all terminals expose", hint: <Kbd>⌘E</Kbd>, run: commands.toggleWall },
      ...(commands.toggleReview
        ? [{ id: "review", icon: <span className="pal-icon">±</span>, label: "Review changes (diffs, comments, merge)", search: "review changes diff merge commit comments", hint: <Kbd>⌘R</Kbd>, run: commands.toggleReview }]
        : []),
      { id: "window", icon: <span className="pal-icon">⧉</span>, label: "Move space to new window", search: "move space new window monitor", hint: <Kbd>⌘⇧N</Kbd>, run: commands.moveToWindow },
      ...(commands.presets ?? PRESETS).map<Item>((p) => ({
        id: `preset-${p}`,
        icon: <span className="pal-icon">▤</span>,
        label: `Tile layout: ${PRESET_LABEL[p]}`,
        search: `layout tile preset ${p} ${PRESET_LABEL[p]} split grid`,
        run: () => commands.preset(p),
      })),
      ...DENSITIES.flatMap<Item>((d) => {
        const size = `${DENSITY_CELLS[d].cols}×${DENSITY_CELLS[d].rows}`;
        return [
          {
            id: `density-${d}`,
            icon: <span className="pal-icon">▦</span>,
            label: `Density: ${DENSITY_LABEL[d]} (${size}, default for all spaces)`,
            search: `density tiles small tile size ${d} global default`,
            run: () => commands.density(d, "global"),
          },
          {
            id: `density-space-${d}`,
            icon: <span className="pal-icon">▦</span>,
            label: `Density for this space: ${DENSITY_LABEL[d]} (${size})`,
            search: `density tiles small tile size ${d} this space`,
            run: () => commands.density(d, "space"),
          },
        ];
      }),
      {
        id: "density-space-default",
        icon: <span className="pal-icon">▦</span>,
        label: "Density for this space: use default",
        search: "density tiles this space default reset",
        run: () => commands.density(null, "space"),
      },
      { id: "sidebar", icon: <span className="pal-icon">⇤</span>, label: "Toggle agents sidebar", search: "toggle sidebar agents panel hide show", hint: <Kbd>⌘B</Kbd>, run: commands.toggleSidebar },
      { id: "right", icon: <span className="pal-icon">⇥</span>, label: "Toggle details panel", search: "toggle right panel details queue changes hide show", hint: <Kbd>⌘.</Kbd>, run: commands.toggleRight },
      { id: "settings", icon: <span className="pal-icon">⚙</span>, label: "Settings", search: "settings codex hooks preferences", run: commands.settings },
    ];
    if (!q.trim()) return base.filter((i) => !i.id.startsWith("q-") && !i.id.startsWith("preset-") && !i.id.startsWith("density-"));
    return base.filter((i) => matches(i.search, q));
  }, [q, agents, selectedId, commands, showAgent, run, patch]);

  const activeIdx = Math.min(active, Math.max(0, items.length - 1));

  const exec = (item: Item | undefined) => {
    if (!item) return;
    const keepOpen = item.run() === false;
    if (!keepOpen) onClose();
  };

  return (
    <Modal onClose={onClose} variant="palette" width={600}>
      <div className="palette">
        <input
          ref={inputRef}
          className="palette-input"
          placeholder="Jump to an agent, or type  name: prompt  to queue it"
          value={q}
          spellCheck={false}
          data-autofocus
          onChange={(e) => {
            setQ(e.target.value);
            setActive(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setActive((i) => Math.min(items.length - 1, i + 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setActive((i) => Math.max(0, i - 1));
            } else if (e.key === "Enter") {
              e.preventDefault();
              exec(items[activeIdx]);
            }
          }}
        />
        <ul className="palette-list" ref={listRef} role="listbox">
          {items.length === 0 && <li className="palette-empty">No matches. Try “name: your prompt” to queue.</li>}
          {items.map((it, i) => (
            <li
              key={it.id}
              role="option"
              aria-selected={i === activeIdx}
              className="palette-item"
              onMouseMove={() => setActive(i)}
              onClick={() => exec(it)}
              ref={(el) => {
                if (el && i === activeIdx) el.scrollIntoView({ block: "nearest" });
              }}
            >
              <span className="pal-lead">{it.icon}</span>
              <span className="pal-label">{it.label}</span>
              {it.hint && <span className="pal-hint">{it.hint}</span>}
            </li>
          ))}
        </ul>
      </div>
    </Modal>
  );
}
