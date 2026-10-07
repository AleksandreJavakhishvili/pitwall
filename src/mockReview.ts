// Mock backend for the review screen (browser `pnpm dev`). See reviewTypes.ts.
import type { AgentView, FileChange } from "./types";
import type { FileVersions, MergeStatus, ReviewApi, Task } from "./reviewTypes";

interface Deps {
  find(id: string): AgentView;
  /** The agent's mutable change list (all changes since start). */
  changes(id: string): FileChange[];
  update(id: string, patch: Partial<AgentView>): AgentView;
}

const wait = <T>(v: T, ms = 90) => new Promise<T>((r) => setTimeout(() => r(v), ms));
const min = 60_000;

const ORIGINAL = `import { loadStore } from "../store";
import { priceItems } from "./pricing";
import { log } from "../log";

export async function createOrder(req: OrderRequest, reply: Reply) {
  const { items, customerId } = req.body;
  const store = await loadStore(req.storeId);

  const first = items[0].sku;
  const currency = items[0].currency;
  const total = priceItems(items, currency);

  log.info("order", first);
  const order = await db.orders.insert({
    customerId,
    total,
    currency,
  });
  return reply.code(201).send(order);
}

export async function cancelOrder(req: CancelRequest, reply: Reply) {
  const order = await db.orders.find(req.params.id);
  if (!order) return reply.code(404).send();
  await db.orders.update(order.id, { status: "cancelled" });
  return reply.code(204).send();
}
`;

const MODIFIED = ORIGINAL.replace(
  `  const first = items[0].sku;
  const currency = items[0].currency;`,
  `  if (items.length === 0) {
    return reply.code(422).send({ error: "cart_empty" });
  }

  const first = items[0].sku;
  const currency = items[0]?.currency ?? store.currency;`,
)
  .replace(`  log.info("order", first);`, `  log.info({ sku: first, customerId }, "order.create");`)
  .replace(`  if (!order) return reply.code(404).send();`, `  if (!order) return reply.code(404).send({ error: "not_found" });`);

const NEW_FILE = `import { describe, expect, it } from "vitest";
import { buildApp } from "../helpers";

describe("POST /v2/orders with an empty cart", () => {
  it("returns 422 instead of 500", async () => {
    const app = await buildApp();
    const res = await app.inject({ method: "POST", url: "/v2/orders", payload: { items: [] } });
    expect(res.statusCode).toBe(422);
    expect(res.json()).toEqual({ error: "cart_empty" });
  });
});
`;

const MD_ORIGINAL = `# Onboarding

Welcome! This page explains how to get set up.

## Vagrant

Install Vagrant and VirtualBox, then run \`vagrant up\`.

## Troubleshooting

Ask in #dev-help.
`;
const MD_MODIFIED = `# Onboarding

Get a dev environment running in 15 minutes.

1. Install the toolchain: \`brew bundle\`
2. Clone the repo and run \`pnpm install\`
3. Copy \`.env.example\` to \`.env\`
4. Start everything: \`pnpm dev\`
5. Open http://localhost:3000
6. Run the tests: \`pnpm test\`

See [the checklist](dev-env/checklist.md) for details.

## Troubleshooting

Ask in #dev-help.
`;

export function versionsFor(f: FileChange | undefined, path: string): FileVersions {
  if (!f) return { original: null, modified: null, binary: false };
  if (f.binary) return { original: null, modified: null, binary: true };
  const md = path.endsWith(".md");
  if (f.status === "D") return { original: md ? MD_ORIGINAL : ORIGINAL, modified: null, binary: false };
  if (f.untracked) return { original: null, modified: md ? MD_MODIFIED.replace("# Onboarding", "# Checklist") : NEW_FILE, binary: false };
  return md ? { original: MD_ORIGINAL, modified: MD_MODIFIED, binary: false } : { original: ORIGINAL, modified: MODIFIED, binary: false };
}

export function createReviewMock(d: Deps): ReviewApi {
  const tasks = new Map<string, Task[]>();
  const tasksFor = (id: string): Task[] => {
    let t = tasks.get(id);
    if (!t) {
      const a = d.find(id);
      const base = a.createdAt;
      t = a.lastSent
        ? [
            { id: `${id}-t1`, prompt: "Read the code around this area and tell me what you'd change.", startedAt: base + 2 * min, endedAt: base + 5 * min, startTree: "t0", endTree: "t1" },
            {
              id: `${id}-t2`,
              prompt: a.lastSent,
              startedAt: a.lastSentAt ?? base + 6 * min,
              endedAt: a.status === "working" || a.status === "blocked" ? null : (a.lastSentAt ?? base) + 3 * min,
              startTree: "t1",
              endTree: a.status === "working" || a.status === "blocked" ? null : "t2",
            },
          ]
        : [];
      tasks.set(id, t);
    }
    return t;
  };
  const totals = (id: string) => {
    const c = d.changes(id);
    d.update(id, {
      added: c.reduce((s, f) => s + f.added, 0),
      removed: c.reduce((s, f) => s + f.removed, 0),
      filesChanged: c.length,
    });
  };
  const committed = new Set<string>();

  return {
    listTasks: (id) => wait(tasksFor(id).map((t) => ({ ...t }))),
    getTaskChanges(id, taskId) {
      const all = d.changes(id);
      if (!taskId) return wait(all.map((f) => ({ ...f })), 120);
      // The first task only "touched" the first file; the latest everything.
      const ts = tasksFor(id);
      const idx = ts.findIndex((t) => t.id === taskId);
      if (idx < 0) return Promise.reject("task not found");
      return wait((idx === 0 ? all.slice(0, 1) : all).map((f) => ({ ...f })), 120);
    },
    getFileVersions: (id, path) => wait(versionsFor(d.changes(id).find((f) => f.path === path), path), 120),
    async discardFile(id, path) {
      await wait(null, 150);
      const list = d.changes(id);
      const i = list.findIndex((f) => f.path === path);
      if (i >= 0) list.splice(i, 1);
      totals(id);
    },
    async commitAgent(id, message) {
      await wait(null, 300);
      if (!message.trim()) throw "commit message is empty";
      if (committed.has(id) || d.changes(id).length === 0) throw "nothing to commit";
      committed.add(id);
      return Math.random().toString(16).slice(2, 9);
    },
    async mergeAgent(id) {
      await wait(null, 400);
      const a = d.find(id);
      if (!a.worktree) throw "This agent works in the main checkout: there is no branch to merge. Commit instead.";
      if (a.name === "api-fix") {
        return {
          merged: false,
          conflict: true,
          branch: "main",
          message: `Merging ${a.branch} into main conflicts in src/orders/handler.ts. The merge was aborted; nothing changed.`,
        };
      }
      d.changes(id).splice(0);
      committed.delete(id);
      totals(id);
      return { merged: true, conflict: false, branch: "main", message: `Merged ${a.branch} into main.` };
    },
    async getMergeStatus(id): Promise<MergeStatus> {
      const a = d.find(id);
      const n = d.changes(id).length;
      const isCommitted = committed.has(id);
      return wait({
        worktree: a.worktree,
        branch: a.branch,
        target: a.worktree ? "main" : null,
        targetDirty: false,
        uncommitted: isCommitted ? 0 : n,
        ahead: isCommitted ? 1 : 0,
      });
    },
  };
}
