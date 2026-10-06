import { useState } from "react";
import { errorText } from "../../api";
import { useActions } from "../../lib/actions";
import { rulesApi } from "../../rules/api";
import { refreshAgentRules, useAgentRules } from "../../rules/useAgentRules";
import { Icon } from "../Icon";
import "./rules.css";

/** Shown in a pane header when the agent's rules changed (or failed) since its session started. */
export function RulesStaleButton({ agentId }: { agentId: string }) {
  const info = useAgentRules(agentId);
  const { run, restart } = useActions();
  const [busy, setBusy] = useState(false);
  if (!info || !(info.stale || info.error)) return null;

  const go = async () => {
    setBusy(true);
    try {
      const res = await run(rulesApi.apply(agentId), "re-apply rules");
      if (res) restart(agentId);
    } finally {
      setBusy(false);
      void refreshAgentRules();
    }
  };

  const title = info.error
    ? `Rules not applied: ${info.error}\nClick to try again and restart.`
    : "Rules changed since this session started. Rules only reach new sessions: re-apply and restart (the conversation resumes when the agent supports it).";
  return (
    <button
      className="rules-stale-btn"
      data-error={!!info.error}
      onClick={(e) => {
        e.stopPropagation();
        void go().catch((err) => console.error(errorText(err)));
      }}
      disabled={busy}
      title={title}
    >
      <Icon name="restart" size={12} />
      {busy ? "Applying…" : "Re-apply rules & restart"}
    </button>
  );
}
