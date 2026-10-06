//! Plain terminals and agents started by hand (docs/spec/terminals.md).
//! A terminal is created with `create_agent` (kind `shell`); agents started
//! inside one show up through `agents-changed`. This adds the sidebar's live
//! "Elsewhere" group: agents running in other terminal apps (read-only).

use std::sync::Arc;

use tauri::State;

use pitwall_core::onboarding::elsewhere::Elsewhere;
use pitwall_core::onboarding::scan::RunningAgent;
use pitwall_core::Shared;

use super::{blocking, Res};

#[tauri::command]
pub async fn list_elsewhere(core: State<'_, Shared>, elsewhere: State<'_, Arc<Elsewhere>>) -> Res<Vec<RunningAgent>> {
    let (core, elsewhere) = (core.inner().clone(), elsewhere.inner().clone());
    blocking(move || elsewhere.list(&core)).await
}
