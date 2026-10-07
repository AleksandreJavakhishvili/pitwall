# Better git diff UI (VS Code feel)

Goal: a better git diff, with file-type icons like in VS Code.
Frontend only; applies to the right-panel CHANGES list, the Review screen's
file tree and the Diff view header.

- **File-type icons** like VS Code: use an MIT-licensed icon set bundled
  locally (no CDN), e.g. Material Icon Theme's SVGs (`material-icon-theme`
  npm package) — verify the licence of whatever is chosen, credit it in
  NOTICE, and only ship the icons actually mapped (tree-shake / small manifest)
  to keep the bundle small. Map by exact filename first (package.json,
  Dockerfile, .gitignore, Cargo.toml, README.md…), then by extension, then a
  generic file icon. Folder icons in the Review tree (open/closed).
- **Git status letter** per file like VS Code's SCM view: M modified, A added,
  D deleted, U untracked, R renamed — coloured (status colours only; keep the
  Pitwall rule that colour carries meaning), right-aligned with +/− counts.
- **Folder grouping**: Review shows a collapsible tree (compact folders like
  VS Code: `src/components/review` on one row when single-child); the
  right panel keeps a flat list with dimmed directory + bold file name.
- **Diff header**: icon + file name + dimmed path + status letter + +/−.
- Keyboard: ↑/↓ moves through files in Review, Enter opens.
- Backend may need to report deleted/renamed status: if `FileChange` lacks it,
  add an optional `status: "M"|"A"|"D"|"R"|"U"` (from `git diff --name-status`)
  — coordinate with the backend migration (it's mid-refactor); keep the field
  optional so the UI degrades to M/U.
