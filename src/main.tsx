import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/barlow-condensed/600.css";
import "@fontsource/barlow-condensed/700.css";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "@xterm/xterm/css/xterm.css";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/layout.css";
import "./styles/agents.css";
import "./styles/panel.css";
import "./styles/overlays.css";
import "./styles/space.css";
import "./styles/wall.css";
import "./styles/terminals.css";
import "./lib/demoBridge"; // website demo only: inert unless browser mock + ?demo=1
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { api } from "./api";
import { loadHost } from "./lib/host";

// Shortcut labels and handlers depend on the desktop (lib/host.ts): ask
// first (bounded; the macOS defaults apply if the backend doesn't answer).
void loadHost(() => api.hostInfo()).then(() => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <ErrorBoundary where="app" variant="root">
        <App />
      </ErrorBoundary>
    </React.StrictMode>,
  );
});
