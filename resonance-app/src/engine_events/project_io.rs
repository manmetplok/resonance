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

/// The engine could not write the clips (`AudioEvent::ClipsSaveFailed`).
/// It still answers `SaveAllPluginStates` after this, so the collector
/// waits for that reply too — tearing it down now would hand that stale
/// reply to whatever save starts next — and [`try_finish_save`] then fails
/// the save (code review STATE2-02).
pub(super) fn clips_save_failed(r: &mut Resonance, error: String) -> Task<Message> {
    if let Some(ref mut save) = r.io.save_state {
        save.clips_error = Some(error);
        save.clips_done = true;
    } else {
        tracing::warn!("[save] clip save failed with no save collecting: {error}");
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
        r.plugin_mirror
            .state_cache
            .insert(*instance_id, std::sync::Arc::from(data.as_slice()));
    }
    // If a preset save was pending, build and save it now: the blobs
    // that just arrived are the only part of a track preset the app
    // cannot produce on its own (ba todo #1303).
    if let Some(pending) = r.presets.pending_preset_save.take() {
        super::presets::finish_preset_save(r, &pending);
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
    if let Some(error) = save.clips_error.clone() {
        return crate::update::project_io::fail_collected_save(r, save, error);
    }
    report_clips_without_audio(r, &save);
    let project_file = crate::update::build_project_file(r);
    let path = save.path.clone();
    // The engine reports blobs only for instances it actually created, so
    // this fills in the app-side copy for any slot it couldn't — a plugin
    // whose `.clap` is missing on this machine. Without it, Save As (and
    // autosave, which shares this path) wrote an empty `plugins/` entry
    // for that slot and the user's settings were gone (ba doc #275, P5).
    let plugin_states = crate::update::plugin_states_for_save(r, save.plugin_states);
    let autosave = save.autosave;
    if !autosave {
        r.io.save_capture_revision = Some(r.revision());
    }
    // Snapshot a versioned backup after each successful *manual* save. The
    // retention count comes from the persisted autosave settings (#462).
    // Autosaves don't snapshot: they write `project.autosave.json`, not
    // the canonical `project.json` that `write_backup` archives.
    let backup_retention = if autosave {
        0
    } else {
        r.autosave_settings().backup_retention
    };

    let clip_gc_keep = (!autosave)
        .then(|| r.clip_gc_keep(&project_file, save.clip_files.keys().copied()));

    let midi_clips: Vec<(ClipId, Vec<MidiNote>)> = r
        .midi_clips
        .iter()
        .map(|mc| (mc.id, mc.notes.as_ref().clone()))
        .collect();

    // A manual save requested while this one collected starts its own
    // engine round-trip now (code review STATE-11).
    crate::update::project_io::start_queued_save(r);

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
                    tracing::warn!("Versioned backup failed: {e}");
                }
            }
            // After the backup, so the reaper reads the snapshot it names.
            if let Some(keep) = clip_gc_keep.flatten() {
                match crate::project::clip_gc::reap_unreferenced_clip_wavs(&path, &keep) {
                    Ok(removed) if !removed.is_empty() => {
                        tracing::info!("[save] removed {} unreferenced clip WAV(s)", removed.len())
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("[save] clip WAV GC skipped: {e}"),
                }
            }
            Ok(())
        },
        move |r| Message::ProjectIo(ProjectIoMessage::ProjectSaved(r, autosave)),
    )
}

impl Resonance {
    /// The clip ids a manual save's WAV GC must keep (code review
    /// FU-V5a): the project being written, the engine's clip list the
    /// save collected, the app mirror and every undo/redo snapshot.
    /// `None` — skip the GC — while recording, whose take is written under
    /// an id nothing names yet. The bundle's own JSONs (autosave, backups)
    /// are read by the reaper itself.
    pub(crate) fn clip_gc_keep(
        &self,
        file: &crate::project::ProjectFile,
        engine_clips: impl IntoIterator<Item = ClipId>,
    ) -> Option<std::collections::BTreeSet<ClipId>> {
        use crate::project::clip_gc::collect_clip_ids;
        if self.transport.recording {
            return None;
        }
        let mut keep: std::collections::BTreeSet<ClipId> = engine_clips.into_iter().collect();
        collect_clip_ids(file, &mut keep);
        keep.extend(self.clips.iter().map(|c| c.id));
        for snapshot in self.session.undo.snapshots() {
            collect_clip_ids(&snapshot.project.file, &mut keep);
        }
        Some(keep)
    }
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
    tracing::warn!("[save] {detail}");
    if !save.autosave {
        r.banners.error_message = Some(detail);
    }
}

pub(super) fn all_cleared(r: &mut Resonance) -> Task<Message> {
    let mut task = Task::none();
    if let Some(loaded) = r.io.pending_load.take() {
        // Every `pending_load` is a disk load or template instantiate: an
        // undo/redo restores in place since ARCH-01 A-13j and never sends
        // `ClearAll`. Extract project_path before replay (replay clears it).
        let path = r.io.project_path.clone();
        crate::update::replay_loaded_project(r, loaded);
        r.io.project_path = path;
        if r.io.project_path.is_none() {
            // Untitled: anchor its clips (and its undo history) to the
            // session's scratch dir, not the template / recovered folder
            // the replay pointed the engine at (code review UX-03).
            crate::update::project_io::anchor_untitled_project(r);
        }
        r.io.loading = false;
        // Re-send Bank Select + Program Change for every
        // external-instrument track from its restored config, so a
        // freshly-powered synth lands on its saved patch and any offline
        // MIDI output is reported. An undo never re-fires MIDI: it does not
        // come here, and `restore_external_instruments` never sends it.
        let _ = r
            .engine
            .send(AudioCommand::ResendExternalInstrumentPatches);

        // A new project starts at its beginning. The horizontal
        // offset is the outer `Scrollable`'s, so scroll it for real
        // and mark the report that follows as the echo of this
        // `scroll_to` (not a manual scroll); state and widget then
        // agree from the next frame (code review FU-V3a).
        r.viewport.scroll_offset = 0.0;
        r.viewport.scroll_offset_y = 0.0;
        r.viewport.follow_pending_x = Some(0.0);
        task = iced::widget::operation::scroll_to(
            crate::state::ARRANGE_SCROLL_ID,
            iced::widget::scrollable::AbsoluteOffset {
                x: Some(0.0),
                y: None,
            },
        );

        // A project load whose media pool references files that aren't
        // on disk: surface the missing-files relink modal so the user can
        // locate them (doc #175, todo #607). An undo/redo does not come
        // here — reopening the modal on every history step would be
        // noise.
        if r.media.pool.has_missing() || r.has_missing_clips() {
            crate::update::relink::open_relink_modal(r);
        }

        // A control-initiated `project.new` (doc #265, todo #1151)
        // or `project.open` (todo #1149) resolves here: the engine
        // confirmed the clear and the project replayed, so a readback
        // right after `job.wait` sees it (code review UPD-02). A
        // template instantiation always lands with no path, while a
        // disk load restores one. No-op when no control job carries
        // the token.
        let recovery = crate::update::project_io::recovery::finish_load(r);
        match r.io.project_path.as_ref() {
            None => {
                r.control.jobs.complete_token(
                    &crate::control_jobs::JobToken::ProjectNew,
                    serde_json::json!({ "path": null, "revision": r.revision() }),
                );
            }
            Some(path) => {
                let path = path.display().to_string();
                let mut result = serde_json::json!({ "path": path, "revision": r.revision() });
                // Code review FU-M12a: say whether the autosave was
                // recovered, or merely exists (`recover_autosave`).
                if recovery.recovered {
                    result["recovered_autosave"] = true.into();
                }
                if recovery.autosave_available {
                    result["autosave_available"] = true.into();
                }
                r.control
                    .jobs
                    .complete_token(&crate::control_jobs::JobToken::ProjectLoad, result);
            }
        }
    }
    task
}
