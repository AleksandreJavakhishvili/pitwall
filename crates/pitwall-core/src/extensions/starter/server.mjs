#!/usr/bin/env node
// __NAME__'s server: a stand-in for the program your extension's agents
// need running on this computer. Pitwall runs it as the extension's
// service (`service` in extension.json) once the user confirms its
// command, keeps it running while the extension is on, and stops it (and
// whatever it started) when the extension is turned off, updated or
// removed, or Pitwall quits. Its output shows in Settings → Extensions →
// Log. No dependencies; Node 18 or later.
//
//   node server.mjs --listen 127.0.0.1:8790 --data DIR   serve
//   node server.mjs --version                            print its version
//
// It answers every request with one line and keeps a count in DIR, the
// private folder Pitwall gives it (`{data}`), which updates never touch.
// Replace it with your own server, or point `service.run` at a program
// your users install (an `install` action).

import { createServer } from "node:http";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const VERSION = "0.1.0";
const NAME = __NAME_JSON__;

const args = process.argv.slice(2);
if (args.includes("--version")) {
  console.log(`${NAME} server ${VERSION}`);
  process.exit(0);
}
const option = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : fallback;
};
const listen = option("--listen", "127.0.0.1:8790");
const data = option("--data", ".");
const at = listen.lastIndexOf(":");
const host = listen.slice(0, at).replace(/^\[|\]$/g, "");
const port = Number(listen.slice(at + 1));

const countFile = join(data, "count.txt");
let count = existsSync(countFile) ? Number(readFileSync(countFile, "utf8")) || 0 : 0;

const server = createServer((req, res) => {
  count += 1;
  writeFileSync(countFile, String(count));
  res.writeHead(200, { "content-type": "text/plain" });
  res.end(`Hello from ${NAME}'s server (${count} requests so far)\n`);
});
server.on("error", (e) => {
  console.error(`can't listen on ${listen}: ${e.message}`);
  process.exit(1);
});
server.listen(port, host, () => console.log(`listening on ${listen}`));

// Pitwall asks it to stop with SIGTERM, then stops it if it doesn't.
process.on("SIGTERM", () => server.close(() => process.exit(0)));
