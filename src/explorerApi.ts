// The read-only code explorer (docs/spec/explorer.md). Shapes from
// pitwall-proto (src/gen). Paths are relative to the agent's folder, `/`-separated;
// offered only when `AgentView.caps.explorer`. Strictly read-only: nothing here writes.
import type { DirListing } from "./gen/DirListing";
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

/** A search with VS Code's defaults: plain text, case-insensitive, hidden files included. */
export function searchQuery(query: string, opts: Partial<SearchQuery> = {}): SearchQuery {
  return { query, regex: false, caseSensitive: false, wholeWord: false, include: [], exclude: [], maxResults: null, hidden: null, defaultExcludes: null, ...opts };
}

/** Text files up to this size open at once; bigger ones offer "Load anyway". */
export const TEXT_CAP = 2 * 1024 * 1024;
/** "Load anyway" reads files up to this size. */
export const LARGE_CAP = 10 * 1024 * 1024;

/** The error a search answers with when a newer one (or `cancelSearch`) replaced it. */
export const SEARCH_CANCELLED = "cancelled";

export interface ExplorerApi {
  /** One folder's children ("" or omitted: the agent's folder); git's view when it is a repository.
   * `ignored`: what git ignores too (marked `ignored`). */
  listFiles(agentId: string, dir?: string, ignored?: boolean): Promise<DirListing>;
  /** Every file, for quick open (⌘P); filtered client-side. */
  listAllFiles(agentId: string): Promise<FileIndex>;
  /** Text up to 2 MiB (`large`, "Load anyway": up to 10 MiB); binary and larger files come without contents. */
  readFile(agentId: string, path: string, large?: boolean): Promise<FileView>;
  /** ripgrep, else git grep. A newer search for the same agent cancels this one. */
  searchFiles(agentId: string, query: SearchQuery): Promise<SearchResult>;
  cancelSearch(agentId: string): Promise<void>;
}
