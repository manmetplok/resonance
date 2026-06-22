//! Update handlers for the reference-track (A/B) feature. Each
//! [`ReferenceMessage`] is turned into the matching [`AudioCommand`] and
//! mutates [`crate::reference::ReferenceState`] optimistically; the engine
//! echoes authoritative values back through `crate::engine_events::reference`.

use std::path::PathBuf;

use iced::Task;
use resonance_audio::types::{ABSource, AudioCommand, ReferenceId, SamplePos};

use crate::message::Message;
use crate::reference::ReferenceMessage;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: ReferenceMessage) -> Task<Message> {
    match m {
        // The only arm that spawns an async side effect (the OS file
        // picker); every other arm mutates state + sends a command
        // synchronously and falls through to `Task::none()` below.
        ReferenceMessage::PickFile => return pick_file_dialog(),
        ReferenceMessage::FilePicked(picked) => {
            if let Some(path) = picked {
                load_requested(r, path);
            }
        }
        ReferenceMessage::LoadRequested(path) => load_requested(r, path),
        ReferenceMessage::Remove(id) => remove(r, id),
        ReferenceMessage::SetActive(id) => set_active(r, id),
        ReferenceMessage::ToggleAbSource => toggle_ab_source(r),
        ReferenceMessage::MomentaryAudition(pressed) => momentary_audition(r, pressed),
        ReferenceMessage::ToggleLoudnessMatch => toggle_loudness_match(r),
        ReferenceMessage::TrimChanged(db) => trim_changed(r, db),
        ReferenceMessage::AddMarker {
            ref_id,
            position_samples,
            label,
        } => add_marker(r, ref_id, position_samples, label),
        ReferenceMessage::RemoveMarker { ref_id, marker_id } => remove_marker(r, ref_id, marker_id),
        ReferenceMessage::Scrub {
            ref_id,
            position_samples,
        } => scrub(r, ref_id, position_samples),
        ReferenceMessage::ToggleLoopToMix => toggle_loop_to_mix(r),
        ReferenceMessage::DismissError => {
            r.reference.last_error = None;
        }
    }
    Task::none()
}

/// Audio container extensions the reference loader accepts, shared by the
/// file picker filter and the window file-drop subscription so both honour
/// the same set.
pub const REFERENCE_AUDIO_EXTENSIONS: &[&str] = &["wav", "flac", "mp3", "ogg"];

/// Open the OS file picker filtered to [`REFERENCE_AUDIO_EXTENSIONS`]. The
/// chosen path (or `None` on cancel) comes back as
/// [`ReferenceMessage::FilePicked`].
fn pick_file_dialog() -> Task<Message> {
    Task::perform(
        async move {
            rfd::AsyncFileDialog::new()
                .set_title("Add Reference Track")
                .add_filter("Audio", REFERENCE_AUDIO_EXTENSIONS)
                .pick_file()
                .await
                .map(|f| f.path().to_path_buf())
        },
        |picked| Message::Reference(ReferenceMessage::FilePicked(picked)),
    )
}

fn load_requested(r: &mut Resonance, path: PathBuf) {
    // The engine allocates the id, so we can't register an entry yet —
    // queue the path so the first analysis event for the new id can
    // recover its name. Clear any stale load-error notice.
    r.reference.last_error = None;
    r.reference
        .pending_loads
        .push_back(path.to_string_lossy().into_owned());
    let _ = r.engine.send(AudioCommand::LoadReferenceTrack {
        id_hint: None,
        path,
    });
}

fn remove(r: &mut Resonance, id: ReferenceId) {
    if let Some(idx) = r.reference.index_of(id) {
        r.reference.entries.remove(idx);
    }
    // The engine clears the active selection itself, but mirror it now so
    // the optimistic view is consistent before the echo arrives.
    if r.reference.active_id == Some(id) {
        r.reference.active_id = None;
    }
    let _ = r.engine.send(AudioCommand::RemoveReferenceTrack { id });
}

fn set_active(r: &mut Resonance, id: ReferenceId) {
    if r.reference.index_of(id).is_some() {
        r.reference.active_id = Some(id);
        let _ = r.engine.send(AudioCommand::SetActiveReference { id });
    }
}

fn set_ab_source(r: &mut Resonance, source: ABSource) {
    r.reference.ab_source = source;
    let _ = r.engine.send(AudioCommand::SetABSource { source });
}

fn toggle_ab_source(r: &mut Resonance) {
    let next = match r.reference.ab_source {
        ABSource::Mix => ABSource::Reference,
        ABSource::Reference => ABSource::Mix,
    };
    set_ab_source(r, next);
}

fn momentary_audition(r: &mut Resonance, pressed: bool) {
    if pressed {
        // Remember the source to return to, then audition the reference.
        // Guard against a double-press leaking the restore target.
        if r.reference.momentary_restore.is_none() {
            r.reference.momentary_restore = Some(r.reference.ab_source);
        }
        set_ab_source(r, ABSource::Reference);
    } else {
        let restore = r.reference.momentary_restore.take().unwrap_or_default();
        set_ab_source(r, restore);
    }
}

fn toggle_loudness_match(r: &mut Resonance) {
    let enabled = !r.reference.loudness_match;
    r.reference.loudness_match = enabled;
    let _ = r
        .engine
        .send(AudioCommand::SetRefLoudnessMatch { enabled });
}

fn trim_changed(r: &mut Resonance, db: f32) {
    r.reference.trim_db = db;
    let _ = r.engine.send(AudioCommand::SetRefTrim { db });
}

fn add_marker(r: &mut Resonance, ref_id: ReferenceId, position_samples: SamplePos, label: String) {
    // The engine allocates the marker id and echoes `RefMarkerAdded`, so
    // we only dispatch here — no optimistic entry without a stable id.
    if r.reference.index_of(ref_id).is_some() {
        let _ = r.engine.send(AudioCommand::AddRefMarker {
            ref_id,
            position_samples,
            label,
        });
    }
}

fn remove_marker(r: &mut Resonance, ref_id: ReferenceId, marker_id: u32) {
    if let Some(entry) = r.reference.entry_mut(ref_id) {
        entry.markers.retain(|mk| mk.id != marker_id);
    }
    let _ = r
        .engine
        .send(AudioCommand::RemoveRefMarker { ref_id, marker_id });
}

fn scrub(r: &mut Resonance, ref_id: ReferenceId, position_samples: SamplePos) {
    if let Some(entry) = r.reference.entry_mut(ref_id) {
        entry.position_samples = position_samples;
    }
    let _ = r.engine.send(AudioCommand::SetRefPosition {
        ref_id,
        position_samples,
    });
}

fn toggle_loop_to_mix(r: &mut Resonance) {
    let enabled = !r.reference.loop_to_mix;
    r.reference.loop_to_mix = enabled;
    let _ = r.engine.send(AudioCommand::SetRefLoopToMix { enabled });
}
