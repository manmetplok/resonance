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
    try_finish_save(r)
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
        super::presets::finish_preset_save(r, track_id);
    }
    if let Some(ref mut save) = r.io.save_state {
        save.plugin_states = states;
        save.plugins_done = true;
    }
    try_finish_save(r)
}

/// Emit the project-save `Task` once both the clip-save and plugin-state
/// branches of the save have reported in. Lives here because the two
/// `Saved` event handlers above are the only callers.
pub(super) fn try_finish_save(r: &mut Resonance) -> Task<Message> {
    let both_done = r
        .io
        .save_state
        .as_ref()
        .map(|s| s.clips_done && s.plugins_done)
        .unwrap_or(false);

    if !both_done {
        return Task::none();
    }

    let save = r
        .io
        .save_state
        .take()
        .expect("save_state present when both_done");
    let project_file = crate::update::build_project_file(r);
    let path = save.path.clone();
    let plugin_states = save.plugin_states;
    let autosave = save.autosave;
    // Snapshot a versioned backup after each successful *manual* save. The
    // retention count comes from the persisted autosave settings (#462).
    // Autosaves don't snapshot: they write `project.autosave.json`, not
    // the canonical `project.json` that `write_backup` archives.
    let backup_retention = if autosave {
        0
    } else {
        r.autosave_settings().backup_retention
    };

    let midi_clips: Vec<(ClipId, Vec<MidiNote>)> = r
        .midi_clips
        .iter()
        .map(|mc| (mc.id, mc.notes.clone()))
        .collect();

    Task::perform(
        async move {
            if autosave {
                crate::project::save_autosave(&path, &project_file, &plugin_states, &midi_clips)?;
            } else {
                crate::project::save_project(&path, &project_file, &plugin_states, &midi_clips)?;
            }
            // Snapshot the just-written project.json into backups/. A
            // failed backup must not fail the save — the project is
            // already safely on disk — so it's logged, not propagated.
            if backup_retention > 0 {
                let timestamp = crate::project::backup_timestamp_now();
                if let Err(e) = crate::project::write_backup(&path, &timestamp, backup_retention) {
                    eprintln!("Versioned backup failed: {e}");
                }
            }
            Ok(())
        },
        move |r| Message::ProjectIo(ProjectIoMessage::ProjectSaved(r, autosave)),
    )
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
