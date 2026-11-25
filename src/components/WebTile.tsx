import type { AgentView } from "../types";
import { shortUrl } from "../mockWeb";
import { Icon } from "./Icon";
import "./webTile.css";

/**
 * A web tile beside the agent's terminal, as the desktop app's Preview opens
 * one. In the web demo the page is drawn here, made up and static: no network,
 * no iframe, nothing to sign in to (mockWeb.ts).
 */
export function WebTile({ agent, url, onClose }: { agent: AgentView; url: string; onClose(): void }) {
  return (
    <section className="side-tile web-tile" aria-label={`${agent.name}'s preview`}>
      <div className="web-bar">
        <button className="icon-btn icon-btn-sm" disabled title="Back" aria-label="Back">
          <Icon name="back" size={13} />
        </button>
        <button className="icon-btn icon-btn-sm" disabled title="Forward" aria-label="Forward">
          <Icon name="chevron" size={13} />
        </button>
        <button className="icon-btn icon-btn-sm" title="Reload" aria-label="Reload">
          <Icon name="refresh" size={13} />
        </button>
        <span className="web-addr mono" title={url}>
          {shortUrl(url)}
        </span>
        <span className="web-title">{agent.name}'s preview</span>
        <span className="spacer" />
        <button className="icon-btn icon-btn-sm" disabled title="Open in your browser (not in the web demo)" aria-label="Open in your browser">
          <Icon name="expand" size={13} />
        </button>
        <button className="icon-btn icon-btn-sm" data-tile-close onClick={onClose} title="Close" aria-label="Close preview">
          <Icon name="x" size={13} />
        </button>
      </div>
      <div className="web-page">
        <HandbookPage />
      </div>
    </section>
  );
}

/** The made-up handbook the docs agent rewrote, as its local server would show it. */
function HandbookPage() {
  return (
    <div className="hb">
      <header className="hb-nav">
        <b>Handbook</b>
        <span>Onboarding</span>
        <span>Services</span>
        <span>On call</span>
      </header>
      <div className="hb-body">
        <nav className="hb-side" aria-label="Handbook pages">
          <span>Welcome</span>
          <span className="on">Onboarding</span>
          <span>Dev env checklist</span>
          <span>Accounts</span>
          <span>Troubleshooting</span>
        </nav>
        <article className="hb-main">
          <h1>Onboarding</h1>
          <p className="hb-lead">A dev environment that runs in 15 minutes. Work down the list; each step says how to check it worked.</p>
          <ol className="hb-steps">
            <li>
              <b>Install the tools</b>
              <span>Git, Node 22 and pnpm. <code>pnpm -v</code> prints 9 or later.</span>
            </li>
            <li>
              <b>Clone the repositories</b>
              <span>
                <code>orders-api</code> and <code>checkout-web</code> into <code>~/code</code>.
              </span>
            </li>
            <li>
              <b>Copy the local settings</b>
              <span>
                <code>cp .env.example .env.local</code> in each.
              </span>
            </li>
            <li>
              <b>Start the database</b>
              <span>
                <code>pnpm db:up</code>, then <code>pnpm db:seed</code>.
              </span>
            </li>
            <li>
              <b>Run both apps</b>
              <span>
                <code>pnpm dev</code>; the shop opens on <code>localhost:5173</code>.
              </span>
            </li>
            <li>
              <b>Place a test order</b>
              <span>Any card from the checklist page goes through.</span>
            </li>
          </ol>
          <p className="hb-note">Stuck? See Troubleshooting at the end of this page.</p>
        </article>
      </div>
      <footer className="hb-foot">Made-up page for the Pitwall web demo</footer>
    </div>
  );
}
