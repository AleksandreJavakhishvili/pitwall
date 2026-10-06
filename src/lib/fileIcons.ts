// VS Code–style file/folder icons (docs/spec/diff-ui.md). The SVGs come from
// Material Icon Theme (MIT, see NOTICE); only the icons in fileIcons.map.json
// are bundled — regenerate both with `pnpm icons`. Each SVG is its own asset
// file (never inlined into JS), so the webview fetches only the icons on screen.
import map from "./fileIcons.map.json";

const urls = import.meta.glob<string>("../assets/file-icons/*.svg", {
  eager: true,
  query: "?no-inline",
  import: "default",
});

const fileNames: Record<string, string> = map.fileNames;
const extensions: Record<string, string> = map.extensions;
const folders: Record<string, string[]> = map.folders;

export function baseName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}

/** Icon name for a file: exact file name, then the longest known extension, then generic. */
export function fileIconName(path: string): string {
  const name = baseName(path).toLowerCase();
  if (fileNames[name]) return fileNames[name];
  for (let i = name.indexOf("."); i >= 0; i = name.indexOf(".", i + 1)) {
    const ext = name.slice(i + 1);
    if (ext && extensions[ext]) return extensions[ext];
  }
  return map.file;
}

/** Icon name for a folder (by its last segment), open or closed. */
export function folderIconName(path: string, open: boolean): string {
  const pair = folders[baseName(path).toLowerCase()] ?? map.folder;
  return pair[open ? 1 : 0];
}

export function iconUrl(name: string): string {
  return urls[`../assets/file-icons/${name}.svg`] ?? urls[`../assets/file-icons/${map.file}.svg`];
}
