#!/usr/bin/env node
// Generates the VS Code–style file icons used by the Changes list, the Review
// tree and the diff header (docs/spec/diff-ui.md).
//
// Source: Material Icon Theme (npm `material-icon-theme`, MIT, see NOTICE).
// Only the icons reachable from the curated names below are copied, so the app
// ships a few dozen small SVGs instead of the whole 1,200-icon set.
//
//   pnpm icons        # after bumping material-icon-theme or editing the lists
//
// Output: src/assets/file-icons/*.svg (+ LICENSE) and src/lib/fileIcons.map.json.
import { copyFileSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = join(root, "node_modules/material-icon-theme");
const manifest = JSON.parse(readFileSync(join(pkg, "dist/material-icons.json"), "utf8"));
const outDir = join(root, "src/assets/file-icons");
const mapFile = join(root, "src/lib/fileIcons.map.json");

/** Exact file names (matched case-insensitively). */
const FILE_NAMES = [
  "package.json", "package-lock.json", "pnpm-lock.yaml", "pnpm-workspace.yaml", "yarn.lock", ".npmrc", ".nvmrc",
  "tsconfig.json", "tsconfig.node.json", "jsconfig.json", "vite.config.ts", "vite.config.js", "vitest.config.ts",
  "eslint.config.js", "eslint.config.mjs", ".eslintrc", ".eslintrc.json", ".prettierrc", ".prettierrc.json",
  "dockerfile", "docker-compose.yml", "docker-compose.yaml",
  ".gitignore", ".gitattributes", ".gitmodules", "readme.md", "license", "license.md", "changelog.md",
  "contributing.md", "makefile", ".env", ".env.local", ".env.example", "tailwind.config.js", "tailwind.config.ts",
  "favicon.ico", "go.mod", "go.sum", "requirements.txt", "pyproject.toml", "gemfile", "agents.md",
];
/** Overrides where the theme has no entry (VS Code resolves these via language ids). */
const FILE_NAME_OVERRIDES = {
  "cargo.toml": "rust", "cargo.lock": "lock", "rust-toolchain.toml": "rust", "notice": "license",
  "tauri.conf.json": "tauri", ".env": "tune",
};
/** Extensions, most specific first at lookup time (e.g. `test.ts` before `ts`). */
const EXTENSIONS = [
  "ts", "tsx", "mts", "cts", "d.ts", "js", "jsx", "mjs", "cjs", "json", "jsonc", "json5",
  "spec.ts", "test.ts", "spec.tsx", "test.tsx", "spec.js", "test.js", "spec.jsx", "test.jsx",
  "md", "mdx", "rst", "txt", "rs", "toml", "yaml", "yml", "lock", "ini", "cfg", "conf", "env",
  "html", "htm", "css", "scss", "sass", "less", "svg", "png", "jpg", "jpeg", "gif", "webp", "ico", "icns",
  "woff", "woff2", "ttf", "otf", "pdf", "zip", "gz", "tgz", "tar",
  "py", "go", "java", "kt", "kts", "swift", "c", "h", "cc", "cpp", "hpp", "m", "mm", "cs", "rb", "php", "lua",
  "sh", "bash", "zsh", "fish", "ps1", "sql", "db", "sqlite", "xml", "plist", "vue", "svelte", "astro",
  "graphql", "gql", "proto", "diff", "patch", "log", "csv", "tsv", "wasm", "dart", "ex", "exs", "zig", "nix",
  "hcl", "tf", "http", "tex", "ipynb", "r",
];
/** Language-id style fallbacks for extensions the theme maps via languageIds. */
const EXTENSION_OVERRIDES = {
  md: "markdown", mdx: "mdx", py: "python", go: "go", java: "java", kt: "kotlin", kts: "kotlin", swift: "swift",
  c: "c", h: "h", cc: "cpp", cpp: "cpp", hpp: "hpp", cs: "csharp", rb: "ruby", php: "php", lua: "lua",
  sh: "console", bash: "console", zsh: "console", fish: "console", ps1: "powershell", sql: "database",
  xml: "xml", html: "html", htm: "html", css: "css", scss: "sass", less: "less", js: "javascript", mjs: "javascript",
  cjs: "javascript", jsx: "react", ts: "typescript", mts: "typescript", cts: "typescript", tsx: "react_ts",
  json: "json", jsonc: "json", yaml: "yaml", yml: "yaml", toml: "toml", rs: "rust", txt: "document",
  diff: "diff", patch: "diff", vue: "vue", svelte: "svelte", dart: "dart", r: "r", tex: "tex", ini: "settings",
  cfg: "settings", conf: "settings", plist: "settings", log: "log",
};
/** Folder names for the Review tree; the theme provides `<icon>` and `<icon>-open`. */
const FOLDERS = [
  "src", "lib", "components", "docs", "spec", "test", "tests", "__tests__", "scripts", "assets", "public", "styles",
  "src-tauri", "config", "types", "utils", "hooks", "api", "dist", ".github", ".vscode", "images", "icons",
  "website", "server", "app", "store",
];

const exists = (icon) => icon && manifest.iconDefinitions[icon];
const used = new Set([manifest.file, manifest.folder, manifest.folderExpanded]);
const pick = (icon) => {
  if (!exists(icon)) return null;
  used.add(icon);
  return icon;
};

const fileNames = {};
for (const n of FILE_NAMES) {
  const icon = pick(manifest.fileNames[n]);
  if (icon) fileNames[n] = icon;
}
for (const [n, icon] of Object.entries(FILE_NAME_OVERRIDES)) {
  if (!fileNames[n] && pick(icon)) fileNames[n] = icon;
}

const extensions = {};
for (const e of EXTENSIONS) {
  const icon = pick(manifest.fileExtensions[e]) ?? pick(EXTENSION_OVERRIDES[e]);
  if (icon) extensions[e] = icon;
}

const folders = {};
for (const f of FOLDERS) {
  const closed = manifest.folderNames[f];
  const open = manifest.folderNamesExpanded[f];
  if (exists(closed) && exists(open)) {
    used.add(closed);
    used.add(open);
    folders[f] = [closed, open];
  }
}

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
let bytes = 0;
for (const icon of [...used].sort()) {
  const src = join(dirname(join(pkg, "dist/material-icons.json")), manifest.iconDefinitions[icon].iconPath);
  copyFileSync(src, join(outDir, `${icon}.svg`));
  bytes += readFileSync(src).length;
}
copyFileSync(join(pkg, "LICENSE"), join(outDir, "LICENSE"));

const { version } = JSON.parse(readFileSync(join(pkg, "package.json"), "utf8"));
const map = {
  source: `material-icon-theme@${version} (MIT)`,
  file: manifest.file,
  folder: [manifest.folder, manifest.folderExpanded],
  fileNames,
  extensions,
  folders,
};
writeFileSync(mapFile, JSON.stringify(map, null, 2) + "\n");
const missing = [...EXTENSIONS.filter((e) => !extensions[e]), ...FILE_NAMES.filter((n) => !fileNames[n])];
console.log(`file icons: ${readdirSync(outDir).length - 1} SVGs, ${(bytes / 1024).toFixed(1)} kB → ${outDir}`);
if (missing.length) console.log(`  (generic icon for: ${missing.join(", ")})`);
