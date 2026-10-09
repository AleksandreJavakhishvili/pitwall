import { useEffect, useState, type KeyboardEvent } from "react";
import type { HooksStatus, KindView } from "../types";
import { ENGINEER_AUTO, ENGINEER_GREETING } from "../lib/engineer";
import { api } from "../api";
import { useActions } from "../lib/actions";
import { Modal } from "./Modal";
import { RulesSettings } from "./rules/RulesSettings";
import { LayoutSettings } from "./LayoutSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { CliSettings } from "./CliSettings";
import { PermissionsSettings } from "./PermissionsSettings";

/** The pages, in the app's order (`pitwall_proto::settings::Page`). */
export const SETTINGS_PAGES = [
  { id: "general", label: "General" },
  { id: "agents", label: "Agents" },
  { id: "rules", label: "Rules" },
  { id: "appearance", label: "Appearance" },
  { id: "folderAccess", label: "Folder access" },
  { id: "about", label: "About" },
] as const;
export type SettingsPage = (typeof SETTINGS_PAGES)[number]["id"];

const PAGE_KEY = "pitwall.settingsPage";

function savedPage(): SettingsPage {
  try {
    const v = localStorage.getItem(PAGE_KEY);
    if (SETTINGS_PAGES.some((p) => p.id === v)) return v as SettingsPage;
  } catch {
    // no storage: start on General
  }
  return "general";
}

/** The page before or after `p` (wrapping), for ↑/↓ in the page list. */
export function stepPage(p: SettingsPage, delta: number): SettingsPage {
  const i = SETTINGS_PAGES.findIndex((x) => x.id === p);
  const n = SETTINGS_PAGES.length;
  return SETTINGS_PAGES[(((i + delta) % n) + n) % n].id;
}

export function SettingsDialog({
  onClose,
  onScanAgain,
  initialPage,
}: {
  onClose(): void;
  onScanAgain?(): void;
  initialPage?: SettingsPage;
}) {
  const [page, setPageState] = useState<SettingsPage>(initialPage ?? savedPage);
  const setPage = (p: SettingsPage) => {
    setPageState(p);
    try {
      localStorage.setItem(PAGE_KEY, p);
    } catch {
      // not remembered; fine
    }
  };
  const onNavKey = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const next = stepPage(page, e.key === "ArrowDown" ? 1 : -1);
    setPage(next);
    (e.currentTarget.querySelector(`[data-page="${next}"]`) as HTMLElement | null)?.focus();
  };
  const label = SETTINGS_PAGES.find((p) => p.id === page)?.label ?? "";
  return (
    <Modal title="Settings" onClose={onClose} width={720} className="settings-modal">
      <div className="settings-split">
        <nav className="settings-nav" role="tablist" aria-orientation="vertical" aria-label="Settings pages" onKeyDown={onNavKey}>
          {SETTINGS_PAGES.map((p) => (
            <button
              key={p.id}
              role="tab"
              data-page={p.id}
              aria-selected={p.id === page}
              tabIndex={p.id === page ? 0 : -1}
              data-autofocus={p.id === page ? "" : undefined}
              className="settings-nav-item"
              data-on={p.id === page}
              onClick={() => setPage(p.id)}
            >
              {p.label}
            </button>
          ))}
        </nav>
        <div className="settings-page" role="tabpanel" aria-label={label}>
          <span className="label">{label}</span>
          {page === "general" && <GeneralPage onScanAgain={onScanAgain} />}
          {page === "agents" && <AgentsPage />}
          {page === "rules" && <RulesSettings />}
          {page === "appearance" && (
            <>
              <AppearanceSettings />
              <LayoutSettings />
            </>
          )}
          {page === "folderAccess" && <PermissionsSettings />}
          {page === "about" && <AboutPage />}
        </div>
      </div>
    </Modal>
  );
}

function GeneralPage({ onScanAgain }: { onScanAgain?(): void }) {
  const { hideElsewhere, setHideElsewhere } = useActions();
  return (
    <>
      {onScanAgain && (
        <div className="setting">
          <div className="setting-head">
            <span className="setting-title">Projects & agents</span>
            <span className="spacer" />
            <button className="small-btn" onClick={onScanAgain}>
              Scan again
            </button>
          </div>
          <p className="muted">Look for new projects, conversations you can continue and installed agents. Read-only.</p>
        </div>
      )}
      <div className="setting">
        <label className="setting-check">
          <span>
            <span className="setting-title">Show agents running elsewhere</span>
            <span className="hint block">
              A sidebar group with Claude Code, Codex and other agents running in other terminal apps (checked every ~10 s,
              read-only), each with “Bring in”.
            </span>
          </span>
          <input type="checkbox" checked={!hideElsewhere} onChange={(e) => setHideElsewhere(!e.target.checked)} />
        </label>
      </div>
      <CliSettings />
    </>
  );
}

function AgentsPage() {
  const { run } = useActions();
  const [status, setStatus] = useState<HooksStatus | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    run(api.codexHooksStatus(), "read Codex hooks status").then((s) => s && setStatus(s));
  }, [run]);

  const install = async () => {
    setBusy(true);
    const s = await run(api.installCodexHooks(), "install Codex hooks");
    setBusy(false);
    if (s) {
      setStatus(s);
      setConfirming(false);
    }
  };

  return (
    <>
      <div className="setting">
        <div className="setting-head">
          <span className="setting-title">Claude Code</span>
          <span className="spacer" />
          <span className="chip chip-ok">automatic</span>
        </div>
        <p className="muted">Hooks are passed per launch. Your ~/.claude settings are never edited.</p>
      </div>
      <div className="setting">
        <div className="setting-head">
          <span className="setting-title">Exact Codex status (hooks)</span>
          <span className="spacer" />
          {status === null ? (
            <span className="muted-sm">checking…</span>
          ) : status.installed ? (
            <span className="chip chip-ok">installed</span>
          ) : (
            <span className="chip chip-subtle">not installed</span>
          )}
        </div>
        <p className="muted">
          Without hooks, Pitwall reads Codex's screen to tell working, blocked and done apart. Hooks make it exact.
        </p>
        {status && <p className="hint mono">{status.path}</p>}
        {status && !status.installed && !confirming && (
          <button className="small-btn" onClick={() => setConfirming(true)}>
            Install hooks…
          </button>
        )}
        {confirming && (
          <div className="confirm-box">
            <p>
              This edits <span className="mono">~/.codex/hooks.json</span>:
            </p>
            <ul>
              <li>A backup of the current file is made first.</li>
              <li>Pitwall entries are appended; your existing hooks stay.</li>
              <li>Outside Pitwall the hook does nothing and exits silently.</li>
            </ul>
            <div className="row gap">
              <button className="primary-btn" onClick={install} disabled={busy}>
                {busy ? "Installing…" : "Install"}
              </button>
              <button className="ghost-btn" onClick={() => setConfirming(false)}>
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
      <EngineerSetting />
    </>
  );
}

/** Agents → Race Engineer (`engineer.agent`, `engineer.greeting`; docs/spec/engineer.md). */
function EngineerSetting() {
  const { run } = useActions();
  const [kinds, setKinds] = useState<KindView[]>([]);
  const [agent, setAgent] = useState(ENGINEER_AUTO);
  const [greeting, setGreeting] = useState(ENGINEER_GREETING);
  useEffect(() => {
    run(api.listKinds(), "list agents").then((k) => k && setKinds(k.filter((x) => x.id !== "shell")));
  }, [run]);
  const auto = kinds.find((k) => k.id === "claude" && k.installed) ?? kinds.find((k) => k.installed && k.id !== "custom");
  return (
    <div className="setting">
      <div className="setting-head">
        <span className="setting-title">Race Engineer</span>
        <span className="spacer" />
        <span className="chip chip-subtle">optional</span>
      </div>
      <p className="muted">
        An assistant agent for Pitwall itself: agw setup, spaces, agents per project, rules and settings, through the
        pitwall CLI. Risky steps still wait for your OK here. It opens from the top bar or ⌘K; nothing runs until you open
        it.
      </p>
      <div className="setting-row">
        <span className="muted">Runs on</span>
        <span className="spacer" />
        <select className="input" value={agent} onChange={(e) => setAgent(e.target.value)}>
          <option value={ENGINEER_AUTO}>{auto ? `Automatic (${auto.name})` : "Automatic"}</option>
          {kinds.map((k) => (
            <option key={k.id} value={k.id}>
              {k.installed || k.id === "custom" ? k.name : `${k.name} (not installed)`}
            </option>
          ))}
        </select>
      </div>
      <div className="setting-row">
        <span className="muted">First prompt</span>
        <span className="spacer" />
        <input className="input" style={{ width: 380 }} value={greeting} onChange={(e) => setGreeting(e.target.value)} />
      </div>
      <p className="hint">Changes apply the next time it starts. It works in its own folder in Pitwall's data folder.</p>
    </div>
  );
}

const LINKS = {
  website: "https://aleksandrejavakhishvili.github.io/pitwall/",
  source: "https://github.com/AleksandreJavakhishvili/pitwall",
  issues: "https://github.com/AleksandreJavakhishvili/pitwall/issues",
  license: "https://github.com/AleksandreJavakhishvili/pitwall/blob/main/LICENSE",
  thirdParty: "https://github.com/AleksandreJavakhishvili/pitwall/tree/main/LICENSES",
};

function AboutPage() {
  return (
    <div className="setting settings-about">
      <div className="settings-about-brand">
        <span className="about-board">
          <span>PIT</span>
          <span>WALL</span>
        </span>
        <span>
          <strong>PITWALL</strong>
          <span className="hint block">You call the strategy. Agents drive.</span>
        </span>
      </div>
      <dl className="settings-facts">
        <dt>Version</dt>
        <dd>Web demo</dd>
        <dt>UI</dt>
        <dd>React (the desktop app is GPUI)</dd>
      </dl>
      <div className="row gap settings-links">
        <a className="retry-btn" href={LINKS.website} target="_blank" rel="noreferrer">
          Website
        </a>
        <a className="retry-btn" href={LINKS.source} target="_blank" rel="noreferrer">
          Source code
        </a>
        <a className="retry-btn" href={LINKS.issues} target="_blank" rel="noreferrer">
          Report a problem
        </a>
      </div>
      <div className="settings-licence">
        <p className="hint">Pitwall is open source under the Apache License 2.0. Copyright 2026 the Pitwall authors.</p>
        <p className="hint">
          Fonts: Inter, Barlow Condensed and JetBrains Mono (SIL OFL 1.1). File icons: Material Icon Theme (MIT).
        </p>
        <div className="row gap settings-links">
          <a className="retry-btn" href={LINKS.license} target="_blank" rel="noreferrer">
            Licence
          </a>
          <a className="retry-btn" href={LINKS.thirdParty} target="_blank" rel="noreferrer">
            Third-party licences
          </a>
        </div>
      </div>
    </div>
  );
}
