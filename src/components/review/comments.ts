// Draft review comments, per agent. Kept for the app session so leaving the
// review screen doesn't lose them. Nothing is sent until the user confirms the
// composed prompt (which they can edit freely).
import { useSyncExternalStore } from "react";

export interface ReviewComment {
  id: string;
  path: string;
  /** Line in the modified (new) version of the file. */
  line: number;
  text: string;
}

let byAgent: Record<string, ReviewComment[]> = {};
const subs = new Set<() => void>();
let seq = 0;

function emit() {
  subs.forEach((f) => f());
}

export const commentStore = {
  subscribe(f: () => void) {
    subs.add(f);
    return () => {
      subs.delete(f);
    };
  },
  get: () => byAgent,
  add(agentId: string, c: Omit<ReviewComment, "id">) {
    const list = byAgent[agentId] ?? [];
    byAgent = { ...byAgent, [agentId]: [...list, { ...c, id: `c${++seq}` }] };
    emit();
  },
  remove(agentId: string, id: string) {
    byAgent = { ...byAgent, [agentId]: (byAgent[agentId] ?? []).filter((c) => c.id !== id) };
    emit();
  },
  clear(agentId: string) {
    const rest = { ...byAgent };
    delete rest[agentId];
    byAgent = rest;
    emit();
  },
};

export function useComments(): Record<string, ReviewComment[]> {
  return useSyncExternalStore(commentStore.subscribe, commentStore.get);
}

/** Comments in file/line order. */
export function sortComments(list: ReviewComment[]): ReviewComment[] {
  return [...list].sort((a, b) => (a.path === b.path ? a.line - b.line : a.path < b.path ? -1 : 1));
}

/**
 * The one visible template Pitwall uses (docs/spec/review.md). Comment text is
 * inserted verbatim; the user sees and can edit the whole prompt before sending.
 */
export function composePrompt(list: ReviewComment[]): string {
  return ["Review comments:", ...sortComments(list).map((c) => `- ${c.path}:${c.line} — ${c.text}`)].join("\n");
}

/** Prefilled (editable) prompt offered after a merge conflict. */
export function conflictPrompt(branch: string | null): string {
  return `Rebase onto ${branch ?? "the main branch"} and resolve conflicts.`;
}
