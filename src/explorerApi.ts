// The read-only code explorer (docs/spec/explorer.md). Shapes from
// pitwall-proto (src/gen). Paths are relative to the agent's folder, `/`-separated;
// offered only when `AgentView.caps.explorer` (and "Open in editor" when `caps.openInEditor`).
import type { DirListing } from "./gen/DirListing";
import type { EditorSettings } from "./gen/EditorSettings";
import type { FileIndex } from "./gen/FileIndex";
import type { FileView } from "./gen/FileView";
import type { SearchQuery } from "./gen/SearchQuery";
import type { SearchResult } from "./gen/SearchResult";

export type { DirListing } from "./gen/DirListing";
export type { FileEntry } from "./gen/FileEntry";
export type { EntryKind } from "./gen/EntryKind";
export type { FileIndex } from "./gen/FileIndex";
export type { FileView } from "./gen/FileView";
export type { ContentKind } from "./gen/ContentKind";
export type { SearchQuery } from "./gen/SearchQuery";
export type { SearchResult } from "./gen/SearchResult";
export type { SearchMatch } from "./gen/SearchMatch";
export type { MatchRange } from "./gen/MatchRange";
export type { EditorSettings } from "./gen/EditorSettings";
export type { EditorChoice } from "./gen/EditorChoice";

/** A search with VS Code's defaults: plain text, case-insensitive, hidden files included. */
export function searchQuery(query: string, opts: Partial<SearchQuery> = {}): SearchQuery {
  return { query, regex: false, caseSensitive: false, wholeWord: false, include: [], exclude: [], maxResults: null, hidden: null, ...opts };
}

/** The error a search answers with when a newer one (or `cancelSearch`) replaced it. */
export const SEARCH_CANCELLED = "cancelled";

export interface ExplorerApi {
  /** One folder's children ("" or omitted: the agent's folder); git's view when it is a repository. */
  listFiles(agentId: string, dir?: string): Promise<DirListing>;
  /** Every file, for quick open (⌘P); filtered client-side. */
  listAllFiles(agentId: string): Promise<FileIndex>;
  /** Text up to 2 MiB; binary and larger files come without contents. */
  readFile(agentId: string, path: string): Promise<FileView>;
  /** ripgrep, else git grep. A newer search for the same agent cancels this one. */
  searchFiles(agentId: string, query: SearchQuery): Promise<SearchResult>;
  cancelSearch(agentId: string): Promise<void>;
  /** Local files only (`caps.openInEditor`); `line`/`column` are 1-based. */
  openInEditor(agentId: string, path: string, line?: number, column?: number): Promise<void>;
  getEditor(): Promise<EditorSettings>;
  /** `null`: back to the first editor found. */
  setEditor(command: string | null): Promise<EditorSettings>;
}

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

export function tauriExplorerApi(invoke: Invoke): ExplorerApi {
  return {
    listFiles: (agentId, dir) => invoke("list_files", { agentId, dir: dir ?? null }),
    listAllFiles: (agentId) => invoke("list_all_files", { agentId }),
    readFile: (agentId, path) => invoke("read_file", { agentId, path }),
    searchFiles: (agentId, query) => invoke("search_files", { agentId, query }),
    cancelSearch: (agentId) => invoke("cancel_search", { agentId }),
    openInEditor: (agentId, path, line, column) => invoke("open_in_editor", { agentId, path, line: line ?? null, column: column ?? null }),
    getEditor: () => invoke("get_editor"),
    setEditor: (command) => invoke("set_editor", { command }),
  };
}
