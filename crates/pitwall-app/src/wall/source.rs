//! Where tiles get their screens: the hosted engine's `watch_screen`
//! (Tauri: the `watch_screen` / `unwatch_screen` commands over a channel).
//! Behind a trait so tests feed made-up frames.

use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::UnboundedSender;
use gpui::{App, Global};

use pitwall_core::engine::input;
use pitwall_core::term::FrameSink;
use pitwall_core::Shared;
use pitwall_proto::ScreenFrame;

/// At most one frame per tile this often (`FRAME_GAP` in src-tauri).
pub const FRAME_GAP: Duration = Duration::from_millis(100);

/// Streams of agents' screens.
pub trait ScreenSource: Send + Sync + 'static {
    /// Start sending `agent_id`'s screen to `sink`; the watch id.
    fn watch(&self, agent_id: &str, sink: FrameSink) -> Result<u64, String>;
    fn unwatch(&self, agent_id: &str, watch_id: u64);
}

/// The engine hosted in this process.
pub struct EngineScreens(pub Shared);

impl ScreenSource for EngineScreens {
    fn watch(&self, agent_id: &str, sink: FrameSink) -> Result<u64, String> {
        input::watch_screen(&self.0, agent_id, sink, FRAME_GAP)
    }
    fn unwatch(&self, agent_id: &str, watch_id: u64) {
        input::unwatch_screen(&self.0, agent_id, watch_id);
    }
}

/// The app's screen source (absent when no engine is hosted).
#[derive(Clone)]
pub struct Screens(pub Arc<dyn ScreenSource>);

impl Global for Screens {}

impl Screens {
    pub fn get(cx: &App) -> Option<Arc<dyn ScreenSource>> {
        cx.try_global::<Screens>().map(|s| s.0.clone())
    }
}

/// A frame on its way to the main thread, tagged with the watch it came
/// from (frames of a dropped watch are ignored).
pub struct FrameMsg {
    pub agent: String,
    pub token: u64,
    pub frame: ScreenFrame,
}

/// A sink that forwards frames to `tx`; it ends the watch once the Wall is
/// gone.
pub fn forward(tx: UnboundedSender<FrameMsg>, agent: String, token: u64) -> FrameSink {
    Box::new(move |f: &ScreenFrame| {
        tx.unbounded_send(FrameMsg {
            agent: agent.clone(),
            token,
            frame: f.clone(),
        })
        .is_ok()
    })
}
