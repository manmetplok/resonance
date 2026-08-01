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
    report_clips_without_audio(r, &save);
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

/// Surface any clip whose audio file is **not** in the bundle the save
/// just wrote.
///
/// `build_project_file` records `audio_file: "audio/clip_<id>.wav"` for
/// every clip in `r.clips`, unconditionally — it has no way to know
/// whether that file exists. The engine, which does, reports back the
/// clips it actually wrote (`AudioEvent::ClipsSavedToProjectDir` ->
/// [`SaveCollector::clip_files`]); until now that list was collected and
/// never read.
///
/// The gap is reachable and self-perpetuating: `replay_audio_clips`
/// pushes a `ClipState` for every `ProjectClip` even when the engine's
/// `LoadClipFromWav` fails on a missing WAV, so a project that has lost
/// an audio file loads with a clip the engine doesn't have, the next
/// save writes the same dangling `audio_file` again, and every save
/// after that repeats it. Nothing downstream complains — the clip simply
/// plays silence, which on a vocal track reads as a section that "didn't
/// render" (ba doc #271).
///
/// Reported, not fatal: the rest of the project is on disk and refusing
/// the save would strand the user's work. An autosave only logs — it must
/// never interrupt.
///
/// [`SaveCollector::clip_files`]: crate::project::SaveCollector::clip_files
fn report_clips_without_audio(r: &mut Resonance, save: &crate::project::SaveCollector) {
    let missing: Vec<ClipId> = r
        .clips
        .iter()
        .map(|c| c.id)
        .filter(|id| !save.clip_files.contains_key(id))
        // The engine reports only the clips it touched this pass, so
        // confirm against the bundle itself before crying wolf.
        .filter(|id| {
            !save
                .path
                .join("audio")
                .join(format!("clip_{id}.wav"))
                .exists()
        })
        .collect();
    if missing.is_empty() {
        return;
    }

    let names: Vec<String> = missing
        .iter()
        .map(|id| {
            r.clips
                .iter()
                .find(|c| c.id == *id)
                .map(|c| format!("{:?}", c.name))
                .unwrap_or_else(|| id.to_string())
        })
        .collect();
    let detail = format!(
        "{} clip(s) have no audio file in {}: {} — they will be silent when the \
         project is reopened.",
        missing.len(),
        save.path.display(),
        names.join(", ")
    );
    eprintln!("[save] {detail}");
    if !save.autosave {
        r.error_message = Some(detail);
    }
}

pub(super) fn all_cleared(r: &mut Resonance) {
    if let Some(loaded) = r.io.pending_load.take() {
        // Extract project_path before replay (replay clears it)
        let path = r.io.project_path.clone();
        // A pending undo/redo extras bundle marks this clear/replay as a
        // history restore rather than a fresh disk load. Capture the
        // per-track freeze refs now, before `replay_loaded_project`
        // consumes `loaded`, so a disk load can re-attach frozen caches
        // afterwards (ba todo #577). Undo/redo restores reconcile freeze
        // via `apply_freeze_restore` in `finalize_undo_restore` instead.
        let freeze_rehydrate = r.io.pending_undo_extras.is_none().then(|| {
            (
                loaded.project_dir.clone(),
                loaded
                    .file
                    .tracks
                    .iter()
                    .map(|t| (t.id, t.freeze.clone()))
                    .collect::<Vec<_>>(),
            )
        });
        crate::update::replay_loaded_project(r, loaded);
        r.io.project_path = path;
        r.io.loading = false;
        // If this clear/replay came from an undo or redo, apply the
        // runtime-only state that replay can't recover (currently: the
        // compose derived-clip cache + freeze status). Otherwise it's a
        // disk load: re-attach each frozen track's cache so reopening
        // replays the cache without re-rendering.
        if let Some(extras) = r.io.pending_undo_extras.take() {
            r.finalize_undo_restore(extras);
        } else {
            // Disk load (not an undo): re-attach each frozen track's cache
            // so reopening replays the cache without re-rendering (ba todo
            // #577).
            if let Some((dir, freezes)) = freeze_rehydrate {
                r.rehydrate_frozen_tracks(&dir, &freezes);
            }

            // Fresh project load (not an undo): re-send Bank Select +
            // Program Change for every external-instrument track from its
            // restored config, so a freshly-powered synth lands on its saved
            // patch and any offline MIDI output is reported. Undo deliberately
            // skips this (see `restore_external_instruments`) so it never
            // re-fires MIDI; here, replaying the saved project, we want it.
            let _ = r
                .engine
                .send(AudioCommand::ResendExternalInstrumentPatches);

            // A genuine project load (not an undo/redo replay) whose media
            // pool references files that aren't on disk: surface the
            // missing-files relink modal so the user can locate them (doc
            // #175, todo #607). Undo/redo replays skip this — reopening the
            // modal on every history step would be noise.
            if r.pool.has_missing() {
                let targets: Vec<resonance_audio::types::AssetId> =
                    r.pool.missing_assets().map(|a| a.id).collect();
                r.relink.open_modal(targets);
            }

            // A control-initiated `project.new` (doc #265, todo #1151)
            // resolves here: the engine confirmed the clear and the
            // fresh project replayed. Untitled case only — a template
            // instantiation always lands with no path, while disk loads
            // (which restore one) resolve their own `ProjectLoad` token
            // in the `ProjectLoaded` arm instead. No-op when no control
            // job carries the token.
            if r.io.project_path.is_none() {
                r.control.jobs.complete_token(
                    &crate::control_jobs::JobToken::ProjectNew,
                    serde_json::json!({ "path": null, "revision": r.revision() }),
                );
            }
        }
    }
}
