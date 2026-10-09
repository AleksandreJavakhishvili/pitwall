// The Race Engineer (docs/spec/engineer.md) in the web demo: what it runs
// on (Settings → Agents, `engineer.agent`) and its first prompt
// (`engineer.greeting`), as pitwall-proto's settings registry has them.

export const ENGINEER_GREETING =
  "Introduce yourself in two lines and offer: set up agw, arrange spaces, create agents per project, set up rules, tune settings.";

/** `engineer.agent`: "auto" (Claude Code if installed, else the first installed agent), a kind id, or "custom". */
export const ENGINEER_AUTO = "auto";
