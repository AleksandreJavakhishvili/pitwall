//! The limits of runtime.md §2.5, in one place.

use std::time::Duration;

/// How often the ticker bumps the engine's epoch; each tick checks the
/// running call's wall-clock deadline (runtime.md §2.5, README §10.3 item
/// 19), so a deadline is overshot by at most about one tick.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// `init`, and instantiating the component (its start functions).
pub const INIT_DEADLINE: Duration = Duration::from_secs(2);
/// `handle`.
pub const HANDLE_DEADLINE: Duration = Duration::from_secs(1);
/// `render`.
pub const RENDER_DEADLINE: Duration = Duration::from_millis(250);
/// `save-state`.
pub const SAVE_STATE_DEADLINE: Duration = Duration::from_millis(50);

/// A `handle` slower than this is noted in the tile's Log.
pub const HANDLE_WARN: Duration = Duration::from_millis(50);
/// A `render` slower than this is noted in the tile's Log.
pub const RENDER_WARN: Duration = Duration::from_millis(16);

/// Linear memory a tile gets unless the `memory` permission grants more.
pub const MEMORY: usize = (pitwall_core::extensions::TILE_MEMORY_MIB as usize) << 20;
/// Instances, tables and memories a component may create.
pub const INSTANCES: usize = 16;
pub const TABLES: usize = 64;
pub const MEMORIES: usize = 16;
/// Elements per table.
pub const TABLE_ELEMENTS: usize = 10_000;

/// Nodes in one frame.
pub const MAX_NODES: usize = 20_000;
/// A frame's encoded size (estimated from its strings and lists).
pub const MAX_FRAME_BYTES: usize = 2 << 20;
/// A node's key.
pub const MAX_KEY_BYTES: usize = 128;
/// A text field's value.
pub const MAX_INPUT_BYTES: usize = 1 << 20;

/// Host calls a second (each import call counts; over it, calls give
/// `busy` and log lines are dropped).
pub const HOST_CALLS_PER_SECOND: u32 = 200;
/// Timers at once.
pub const MAX_TIMERS: usize = 32;
/// The shortest timer.
pub const MIN_TIMER: Duration = Duration::from_millis(100);

/// The key-value store: keys and values together.
pub const STORAGE_BYTES: u64 = 10 << 20;
/// A storage key.
pub const MAX_STORAGE_KEY: usize = 256;
/// The most keys one `storage.get-many` reads.
pub const MAX_GET_MANY: usize = 1024;

/// What `save-state` may return.
pub const MAX_STATE_BYTES: usize = 1 << 20;
/// A log line (longer ones are cut).
pub const MAX_LOG_LINE: usize = 4096;
/// A toast's text (longer is cut).
pub const MAX_TOAST: usize = 200;
/// Toasts a minute.
pub const TOASTS_PER_MINUTE: usize = 3;
/// What stdout and stderr keep between two calls (more is dropped).
pub const MAX_STDIO_BYTES: usize = 64 << 10;

/// Draw ops in one canvas (ui.md §3.7).
pub const MAX_CANVAS_OPS: usize = 20_000;
/// Clickable regions in one canvas.
pub const MAX_CANVAS_REGIONS: usize = 5_000;
/// Lengths in a dash pattern.
pub const MAX_DASH: usize = 16;
/// A code view's text, or each side of a diff view (or its unified diff).
pub const MAX_CODE_BYTES: usize = 1 << 20;
/// A drag's data.
pub const MAX_DRAG_DATA: usize = 64 << 10;

/// Choices in a radio group or a segmented control (ui.md §3.10).
pub const MAX_CHOICES: usize = 64;
/// Levels of breadcrumbs.
pub const MAX_CRUMBS: usize = 32;
/// Lines of a key-value list.
pub const MAX_KEY_VALUES: usize = 500;
/// Series in a bar or line chart.
pub const MAX_SERIES: usize = 8;
/// Labels of a bar chart (and values in each of its series).
pub const MAX_CHART_LABELS: usize = 200;
/// Labels of a line chart, values in each of its series, and values of a
/// sparkline.
pub const MAX_CHART_POINTS: usize = 1000;
/// Segments of a status timeline.
pub const MAX_SEGMENTS: usize = 500;
/// A copyable's text.
pub const MAX_COPY_BYTES: usize = 64 << 10;
/// A badge's text.
pub const MAX_SHORT_TEXT: usize = 32;
/// A link's address.
pub const MAX_URL_BYTES: usize = 2048;

/// A component file.
pub const MAX_WASM_BYTES: u64 = 20 << 20;

/// Rows of a virtual list, table or tree.
pub const MAX_LIST_ROWS: u32 = 1_000_000;
/// A row's height, in logical pixels.
pub const MIN_ROW_HEIGHT: f32 = 12.;
pub const MAX_ROW_HEIGHT: f32 = 200.;
/// Rows one `guest.rows` call may return.
pub const MAX_ROWS_PER_CALL: u32 = 256;
/// The page of rows Pitwall asks for at a time.
pub const ROWS_PAGE: u32 = 64;
/// Nodes in one row.
pub const MAX_ROW_NODES: usize = 64;
/// A table's columns.
pub const MAX_COLUMNS: usize = 64;
/// Tabs, a select's options, a menu's entries.
pub const MAX_ITEMS: usize = 1000;
/// Selected rows a frame names.
pub const MAX_SELECTED: usize = 10_000;
/// A Markdown node's text.
pub const MAX_MARKDOWN_BYTES: usize = 1 << 20;
/// One image handed over with `ui.put-image`.
pub const MAX_IMAGE_BYTES: usize = 8 << 20;
/// All of a tile's images from `ui.put-image` at once.
pub const MAX_IMAGES_BYTES: usize = 64 << 20;
/// An image file in the extension's folder.
pub const MAX_ASSET_BYTES: u64 = 16 << 20;
/// `net`: requests waiting at once.
pub const MAX_FETCHES: usize = 8;
/// `net`: a request's body.
pub const MAX_REQUEST_BODY: usize = 8 << 20;
/// `net`: an answer's body.
pub const MAX_RESPONSE_BODY: usize = 32 << 20;
/// `net`: how long a request may take unless it asks for less…
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// …or for more, up to this.
pub const MAX_FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// `net`: redirects followed.
pub const MAX_REDIRECTS: u32 = 5;

/// `sql`: the database every extension gets, unless the `sql` permission
/// grants more (MiB).
pub const SQL_MIB: u64 = pitwall_core::extensions::TILE_SQL_MIB as u64;
/// `sql`: rows one query returns.
pub const MAX_SQL_ROWS: usize = 10_000;
/// `sql`: bytes one query returns, and the longest text or blob.
pub const MAX_SQL_BYTES: usize = 8 << 20;

/// `notify`: notifications a minute.
pub const NOTIFY_PER_MINUTE: usize = 3;
/// `notify`: the same title and text again within this is shown once.
pub const NOTIFY_SAME_WITHIN: Duration = Duration::from_secs(10 * 60);
/// `notify`: a title's and a text's length (longer is cut).
pub const MAX_NOTIFY_TITLE: usize = 80;
pub const MAX_NOTIFY_BODY: usize = 200;

/// `toolbar`: a tooltip's length (longer is cut).
pub const MAX_TOOLBAR_TOOLTIP: usize = 80;
/// `toolbar`: the least time between two `set-shown` changes.
pub const TOOLBAR_SHOWN_EVERY: Duration = Duration::from_secs(5);
/// `toolbar`: badge changes a second the app draws (newer ones replace
/// the one waiting).
pub const TOOLBAR_BADGES_PER_SECOND: u32 = 4;

/// `files`: the most a file read or written may hold.
pub const MAX_FILE_BYTES: usize = 8 << 20;
/// `files.list`: the most entries one call gives.
pub const MAX_LIST_ENTRIES: usize = 10_000;
/// `files.watch`: the most watches a tile has at once.
pub const MAX_WATCHES: usize = 8;
/// `files`: the most the extension's own files folder holds.
pub const FILES_DATA_BYTES: u64 = 100 << 20;
/// `files.watch`: changes are told this long after the last one…
pub const WATCH_QUIET: Duration = Duration::from_millis(250);
/// …or this long after the first, while they keep coming.
pub const WATCH_MOST: Duration = Duration::from_secs(1);
