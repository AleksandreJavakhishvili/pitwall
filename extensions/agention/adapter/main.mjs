#!/usr/bin/env node
// Pitwall's Agention adapter. Pitwall runs it as `node adapter/main.mjs`
// from the extension's folder and speaks its adapter protocol, version 1,
// over stdin and stdout (docs/spec/extensions.md, "Adapter protocol"). The
// server address and API key arrive only in the first stdin line, `hello`;
// nothing is read from argv or the environment. Logs go to stderr only.
// Node 24 or later; no dependencies.

import { Adapter } from "./lib/adapter.js";

const [major] = process.versions.node.split(".").map(Number);
if (major < 24) {
  process.stderr.write(`The Agention adapter needs Node.js 24 or later; this is ${process.versions.node}.\n`);
}

const adapter = new Adapter({
  output: process.stdout,
  log: (line) => process.stderr.write(line),
});

process.stdout.on("error", () => {
  // Pitwall closed its end: nothing more can be said.
  adapter.shutdown();
});

await adapter.run(process.stdin);
// stdin closed: everything is stopped; give stdout a moment to drain.
setTimeout(() => process.exit(0), 1500).unref();
