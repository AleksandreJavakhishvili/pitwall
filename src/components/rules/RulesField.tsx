import { useEffect, useState } from "react";
import { rulesApi, type RuleSet, type RulesStatus } from "../../rules/api";
import { refreshAgentRules } from "../../rules/useAgentRules";
import "./rules.css";

export interface RulesChoice {
  /** The agent's own extra set (on top of the project default). */
  ruleSetId: string | null;
  /** Explicit OK to write rule files into the main checkout. Default off. */
  applyToMainCheckout: boolean;
}

export const NO_RULES: RulesChoice = { ruleSetId: null, applyToMainCheckout: false };

/** Fields to spread into CreateAgentRequest. */
export function rulesRequest(c: RulesChoice, worktree: boolean): { ruleSetId?: string; applyToMainCheckout?: boolean } {
  return {
    ...(c.ruleSetId ? { ruleSetId: c.ruleSetId } : {}),
    ...(!worktree && c.applyToMainCheckout ? { applyToMainCheckout: true } : {}),
  };
}

/** After create: rule problems never block the agent, so surface them as a toast. */
export async function reportRulesAfterCreate(agentId: string, run: <T>(p: Promise<T>, what: string) => Promise<T | undefined>) {
  await refreshAgentRules();
  try {
    const info = (await rulesApi.agentRules()).find((r) => r.agentId === agentId);
    if (info?.error) await run(Promise.reject(info.error), "apply rules");
  } catch {
    // ignore: older backend without rules
  }
}

interface Props {
  /** The chosen kind can take rules here (`KindView.caps.rules`). */
  canRules: boolean;
  projectPath: string;
  worktree: boolean;
  value: RulesChoice;
  onChange(v: RulesChoice): void;
}

export function RulesField({ canRules, projectPath, worktree, value, onChange }: Props) {
  const [sets, setSets] = useState<RuleSet[] | null>(null);
  const [status, setStatus] = useState<RulesStatus | null>(null);
  const [projectSet, setProjectSet] = useState<string | null>(null);

  useEffect(() => {
    rulesApi.sets().then(setSets, () => setSets([]));
    rulesApi.status().then(setStatus, () => setStatus(null));
  }, []);

  useEffect(() => {
    if (!projectPath) return setProjectSet(null);
    let live = true;
    const t = setTimeout(() => {
      rulesApi.getProjectRules(projectPath).then(
        (id) => live && setProjectSet(id),
        () => live && setProjectSet(null),
      );
    }, 250);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [projectPath]);

  // Nothing to offer until there are rule sets.
  if (!sets || sets.length === 0 || !canRules) return null;

  const projectSetName = sets.find((s) => s.id === projectSet)?.name;
  const anyRules = !!value.ruleSetId || !!projectSet;
  const unavailable = anyRules && status && !status.available;

  return (
    <div className="field">
      <label className="field-label" htmlFor="na-rules">
        Rules
      </label>
      <select
        id="na-rules"
        className="input"
        value={value.ruleSetId ?? ""}
        onChange={(e) => onChange({ ...value, ruleSetId: e.target.value || null })}
      >
        <option value="">{projectSetName ? "Project rules only" : "No extra rules"}</option>
        {sets
          .filter((s) => s.id !== projectSet)
          .map((s) => (
            <option key={s.id} value={s.id}>
              {s.name} ({s.ruleIds.length})
            </option>
          ))}
      </select>
      <span className="hint">
        {projectSetName ? (
          <>
            Project default: <b>{projectSetName}</b>.{" "}
          </>
        ) : null}
        {value.ruleSetId ? "This agent's own rules go to local-only files. " : null}
        Generated with rulesync, kept out of git via .git/info/exclude.
        {worktree ? " Written into the agent's worktree once it has made one; used from its next session." : null}
      </span>
      {unavailable && (
        <span className="field-error">rulesync isn't available — install it or allow npx in Settings → Rules.</span>
      )}
      {anyRules && !worktree && (
        <label className="check rules-main-check">
          <input
            type="checkbox"
            checked={value.applyToMainCheckout}
            onChange={(e) => onChange({ ...value, applyToMainCheckout: e.target.checked })}
          />
          <span>
            Write rule files into my main checkout
            <span className="hint block">
              Without its own worktree this agent works in your checkout. Off: no rules for this agent.
            </span>
          </span>
        </label>
      )}
    </div>
  );
}
