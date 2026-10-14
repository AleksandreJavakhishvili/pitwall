// The accounts (more than one Claude Code / Codex login), shared by every
// component that shows them: read once, then kept current by accounts-changed.
import { useEffect, useState } from "react";
import { api } from "../api";
import type { Account, AccountsView } from "../types";

/** "default": the user's own login (an agent without an account). */
export const DEFAULT_ACCOUNT = "default";

let current: AccountsView | null = null;
let started = false;
const subs = new Set<(v: AccountsView) => void>();

function start() {
  if (started) return;
  started = true;
  const set = (v: AccountsView) => {
    current = v;
    subs.forEach((cb) => cb(v));
  };
  void api.listAccounts().then(set, () => {});
  void api.onAccountsChanged(set);
}

/** The accounts now (null until first read). */
export function useAccounts(): AccountsView | null {
  const [v, setV] = useState(current);
  useEffect(() => {
    start();
    subs.add(setV);
    if (current) setV(current);
    return () => {
      subs.delete(setV);
    };
  }, []);
  return v;
}

/** An agent's account key (`default`: its own login). */
export const accountKey = (a: { account?: string | null }) => a.account || DEFAULT_ACCOUNT;

export function accountOf(v: AccountsView | null, id: string | null | undefined): Account | undefined {
  return id ? v?.accounts.find((a) => a.id === id) : undefined;
}

/** A kind's accounts besides Default. */
export const accountsOfKind = (v: AccountsView | null, kind: string) => v?.accounts.filter((a) => a.kind === kind) ?? [];

/** The account `project`'s new agents of `kind` use (undefined: Default). */
export const projectAccount = (v: AccountsView | null, project: string, kind: string): string | undefined =>
  v?.projects[project]?.[kind];

/** The projects whose new agents use account `id`. */
export const projectsOf = (v: AccountsView | null, id: string): string[] =>
  Object.entries(v?.projects ?? {})
    .filter(([, m]) => Object.values(m).includes(id))
    .map(([p]) => p);

export const folderName = (path: string) => path.replace(/\/+$/, "").split("/").pop() || path;
