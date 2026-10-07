import { useEffect, useState } from "react";
import type { AgentView } from "../types";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { Icon } from "./Icon";
import { Kbd } from "./Kbd";

// Unsent drafts per agent survive switching agents.
const drafts = new Map<string, string>();

export function NextUp({ agent: a }: { agent: AgentView }) {
  const { run, patch } = useActions();
  const [text, setText] = useState(() => drafts.get(a.id) ?? "");

  useEffect(() => setText(drafts.get(a.id) ?? ""), [a.id]);
  const update = (v: string) => {
    drafts.set(a.id, v);
    setText(v);
  };

  const add = async () => {
    if (!text.trim()) return;
    const sent = text; // verbatim — never trimmed or rewritten
    update("");
    const r = await run(api.queueAdd(a.id, sent), "queue prompt");
    if (r) patch(r);
    else update(sent);
  };

  const act = async (p: Promise<AgentView>, what: string) => {
    const r = await run(p, what);
    if (r) patch(r);
  };

  return (
    <section className="panel-section">
      <div className="section-head">
        <span className="label">Next up</span>
        {a.queue.length > 0 && <span className="label-count">{a.queue.length}</span>}
        <span className="spacer" />
        <label className="toggle" title="Send the next item when the agent is idle or done and you're not typing">
          <input
            type="checkbox"
            checked={a.autoSend}
            onChange={(e) => act(api.setAutoSend(a.id, e.target.checked), "change auto-send")}
          />
          <span className="toggle-track" aria-hidden />
          <span className="toggle-label">Auto-send</span>
        </label>
      </div>

      {a.queue.length > 0 && (
        <ol className="queue">
          {a.queue.map((q, i) => (
            <li key={q.id} className="queue-item">
              <span className="queue-n">{i + 1}</span>
              <span className="queue-text">{q.text}</span>
              <span className="queue-actions">
                <button
                  className="icon-btn icon-btn-sm"
                  title="Send now"
                  disabled={!a.running}
                  onClick={() => act(api.queueSendNow(a.id, q.id), "send prompt")}
                >
                  <Icon name="send" size={14} />
                </button>
                <button
                  className="icon-btn icon-btn-sm"
                  title="Remove from queue"
                  onClick={() => act(api.queueRemove(a.id, q.id), "remove queue item")}
                >
                  <Icon name="x" size={14} />
                </button>
              </span>
            </li>
          ))}
        </ol>
      )}

      <div className="queue-compose">
        <textarea
          value={text}
          placeholder="queue a prompt…"
          rows={3}
          onChange={(e) => update(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              add();
            }
          }}
        />
        <div className="compose-foot">
          <span className="hint">
            {a.autoSend ? "Sends when the agent is free" : "Auto-send off — send items manually"}
          </span>
          <button className="small-btn" onClick={add} disabled={!text.trim()}>
            Queue <Kbd submit>⌘↵</Kbd>
          </button>
        </div>
      </div>
    </section>
  );
}
