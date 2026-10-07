import type { FileChange } from "../types";
import { fileIconName, folderIconName, iconUrl } from "../lib/fileIcons";
import { STATUS_TITLE, fileStatus } from "../lib/fileTree";

export function FileIcon({ path, size = 16 }: { path: string; size?: number }) {
  return <img className="file-icon" src={iconUrl(fileIconName(path))} width={size} height={size} alt="" draggable={false} loading="lazy" />;
}

export function FolderIcon({ path, open, size = 16 }: { path: string; open: boolean; size?: number }) {
  return <img className="file-icon" src={iconUrl(folderIconName(path, open))} width={size} height={size} alt="" draggable={false} loading="lazy" />;
}

/** VS Code SCM-style status letter; colour only where it means something (A/U green, D red). */
export function StatusLetter({ file }: { file: FileChange }) {
  const s = fileStatus(file);
  return (
    <span className="status-letter" data-status={s} title={STATUS_TITLE[s]} aria-label={STATUS_TITLE[s]}>
      {s}
    </span>
  );
}

/** +/− (or "bin") then the status letter, right-aligned in a file row. */
export function FileStats({ file }: { file: FileChange }) {
  return (
    <span className="file-stats">
      {file.binary ? <span className="chip chip-subtle">bin</span> : <DiffStatInline file={file} />}
      <StatusLetter file={file} />
    </span>
  );
}

function DiffStatInline({ file }: { file: FileChange }) {
  if (!file.added && !file.removed) return null;
  return (
    <span className="diffstat">
      {file.added > 0 && <span className="add">+{file.added}</span>}
      {file.removed > 0 && <span className="del">−{file.removed}</span>}
    </span>
  );
}
