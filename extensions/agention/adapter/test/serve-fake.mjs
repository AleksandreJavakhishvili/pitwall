// Runs the fake Agention server for a test outside Node (Pitwall's Rust
// test that drives main.mjs over the adapter protocol): one thread with a
// blocking ask, then one line on stdout, {"url","key","thread"}. It stops
// when its stdin closes. Listens on 127.0.0.1 only.

import { FakeAgention, KEYS } from "./fake-agention.js";

const fake = await new FakeAgention().start();
const thread = fake.addThread({
  title: "Which store?",
  asks: [{ question: "Which store?", impact: "blocking", suggestions: [{ label: "Redis" }, { label: "SQLite", description: "a file" }] }],
});
process.stdout.write(`${JSON.stringify({ url: fake.url, key: KEYS.full, thread })}\n`);
process.stdin.resume();
process.stdin.on("end", async () => {
  await fake.close();
  process.exit(0);
});
