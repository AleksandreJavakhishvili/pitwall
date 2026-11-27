//! Limits on untrusted input (hla.md §8, FR-2, FR-21). All *proposals*,
//! to be set from the measurements of plan.md phase B.

/// Boxes (or participants) per diagram; past it, the fallback.
pub const MAX_BOXES: usize = 300;
/// Arrows (or sequence steps) per diagram; past it, the fallback.
pub const MAX_ARROWS: usize = 600;
/// Subgraphs nested in one another.
pub const MAX_DEPTH: usize = 16;
/// Bytes per Markdown file the Scanner reads (phase B).
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Markdown files per project the Scanner lists (phase B).
pub const MAX_FILES: usize = 2000;
