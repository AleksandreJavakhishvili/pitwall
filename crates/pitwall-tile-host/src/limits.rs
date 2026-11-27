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

/// A component file.
pub const MAX_WASM_BYTES: u64 = 20 << 20;
