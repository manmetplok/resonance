//! Engine → app events for the project save / load lifecycle.
//! These are the only event handlers that return a `Task<Message>` —
//! save completion and post-clear replay both kick off async work.

use iced::Task;
use resonance_audio::types::*;

use crate::message::*;
use crate::Resonance;

pub(super) fn clips_saved(
    r: &mut Resonance,
    clip_files: Vec<(ClipId, String)>,
) -> Task<Message> {
    if let Some(ref mut save) = r.io.save_state {
        save.clip_files = clip_files.into_iter().collect();
        save.clips_done = true;
    }
    super::try_finish_save(r)
}

pub(super) fn all_plugin_states_saved(
    r: &mut Resonance,
    states: Vec<(PluginInstanceId, Vec<u8>)>,
) -> Task<Message> {
    // Refresh the undo cache first, then (if a save was in progress)
    // hand the states off to the SaveCollector.
    for (instance_id, data) in &states {
        r.plugin_state_cache.insert(*instance_id, data.clone());
    }
    // If a preset save was pending, build and save it now.
    if let Some(track_id) = r.pending_preset_save.take() {
        super::finish_preset_save(r, track_id);
    }
    if let Some(ref mut save) = r.io.save_state {
        save.plugin_states = states;
        save.plugins_done = true;
    }
    super::try_finish_save(r)
}

pub(super) fn all_cleared(r: &mut Resonance) {
    if let Some(loaded) = r.io.pending_load.take() {
        // Extract project_path before replay (replay clears it)
        let path = r.io.project_path.clone();
        crate::update::replay_loaded_project(r, loaded);
        r.io.project_path = path;
        r.io.loading = false;
        // If this clear/replay came from an undo or redo, apply the
        // runtime-only state that replay can't recover (currently: the
        // compose derived-clip cache).
        if let Some(extras) = r.io.pending_undo_extras.take() {
            r.finalize_undo_restore(extras);
        }
    }
}
