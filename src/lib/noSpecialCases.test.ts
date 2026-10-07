// architecture.md §3: the UI turns features on and off from capabilities
// (`AgentView.caps`, `KindView.caps`). It never asks which kind or provider an
// agent has. This test fails the build when UI code compares against an agent
// kind id or a provider name. Mocks and tests may name kinds (they fake data).
import { describe, expect, it } from "vitest";

const sources = import.meta.glob(["../**/*.ts", "../**/*.tsx"], { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;
const agentFiles = import.meta.glob("../../crates/pitwall-core/agents/*.toml", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

const exempt = (path: string) => /\.test\.tsx?$/.test(path) || /(^|\/)mock[A-Za-z]*\.ts$/.test(path) || /\/mock\.ts$/.test(path);

/** Kind ids from the built-in agent definitions, plus provider names. */
const kindIds = Object.values(agentFiles)
  .map((src) => /^id\s*=\s*"([^"]+)"/m.exec(src)?.[1])
  .filter((id): id is string => !!id);
const PROVIDERS = ["local", "agw", "ssh"];
const banned = [...kindIds, ...PROVIDERS];
const lit = `["'\`](?:${banned.map((b) => b.replace(/[-]/g, "\\-")).join("|")})["'\`]`;
const subject = String.raw`(?:\bkind|\bprovider|\blocation|\.kind|\.provider|\.location|\.machine\.provider)`;
const op = String.raw`\s*(?:===|!==|==|!=)\s*`;

const RULES: [string, RegExp][] = [
  ["kind/provider compared with an id", new RegExp(String.raw`${subject}${op}${lit}|${lit}${op}[\w.]*\b(?:kind|provider|location)\b`)],
  ["kind/provider compared with a constant", new RegExp(String.raw`${subject}${op}[A-Z][A-Z0-9_]+\b|\b[A-Z][A-Z0-9_]*_KIND${op}`)],
  ["membership test on a kind id", new RegExp(String.raw`\.(?:has|includes)\(\s*${lit}\s*\)`)],
  ["set of kind ids", new RegExp(String.raw`new Set\(\[[^\]]*${lit}`)],
  ["switch on kind/provider", new RegExp(String.raw`switch\s*\(\s*[\w.]*\.(?:kind|provider)\s*\)[\s\S]{0,200}case\s+${lit}`)],
];

function offenders(path: string, src: string): string[] {
  const out: string[] = [];
  src.split("\n").forEach((line, i) => {
    const code = line.replace(/\/\/.*$/, "");
    for (const [what, re] of RULES) if (re.test(code)) out.push(`${path}:${i + 1}: ${what}: ${line.trim()}`);
  });
  return out;
}

describe("UI decides by capabilities, never by kind or provider", () => {
  it("knows the kind ids it guards", () => {
    expect(kindIds).toEqual(expect.arrayContaining(["claude", "codex", "shell"]));
  });

  it("catches the special cases it is meant to", () => {
    const bad = [
      `if (a.kind === "claude") x();`,
      `const t = a.terminal && a.kind !== SHELL_KIND;`,
      `if (agent.provider == 'agw') {}`,
      `const NO_TARGET = new Set(["custom", "shell"]);`,
      `installed.has("codex")`,
      `p.sources.includes("claude")`,
      `"local" === a.location`,
    ];
    for (const b of bad) expect(offenders("x.ts", b), b).not.toEqual([]);
    const fine = [
      `if (a.caps.resume) x();`,
      `sp.kind === "custom"`,
      `l.kind === "hunk"`,
      `const req = { kind: SHELL_KIND };`,
      `s.kind === "git"`,
    ];
    for (const f of fine) expect(offenders("x.ts", f), f).toEqual([]);
  });

  it("has no kind or provider special cases in src/", () => {
    const found = Object.entries(sources)
      .filter(([path]) => !exempt(path))
      .flatMap(([path, src]) => offenders(path, src));
    expect(found).toEqual([]);
  });
});
