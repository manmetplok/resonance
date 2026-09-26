//! Update handlers for the reference-track (A/B) feature. Each
//! [`ReferenceMessage`] is turned into the matching [`AudioCommand`] and
//! mutates [`crate::reference::ReferenceState`] optimistically; the engine
//! echoes authoritative values back through `crate::engine_events::reference`.

use std::path::PathBuf;

use iced::Task;
use resonance_audio::types::{
    ABSource, AudioCommand, ReferenceAnalysisStage, ReferenceId, SamplePos,
};

use crate::message::Message;
use crate::reference::{ReferenceEntry, ReferenceMarkerState, ReferenceMessage};
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
        ReferenceMessage::SetAbSource(source) => set_ab_source(r, source),
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

/// A reference's display name until the engine reports one: the file
/// stem, as the engine derives it.
pub(crate) fn reference_name(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
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
    // The app picks the id (FU-A5b) and lists the reference at once, as
    // analysing (FU-A5c): the load is then part of the project — and of
    // any undo snapshot taken from here on — before the engine's first
    // echo, so an undo that predates it drops it and one that does not
    // keeps it. Clear any stale load-error notice.
    r.reference.last_error = None;
    let id = r.reference.alloc_engine_id();
    r.reference.entries.push(ReferenceEntry::analyzing(
        id,
        reference_name(&path),
        path.to_string_lossy().into_owned(),
        ReferenceAnalysisStage::Decoding,
    ));
    let _ = r.engine.send(AudioCommand::LoadReferenceTrack {
        id,
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
    // Removing a reference (e.g. dismissing a missing entry) resolves any
    // outstanding load-failure notice — otherwise, once the last entry is
    // gone, the stale `last_error` would resurface as the full-panel error
    // body instead of returning to the empty drop-zone.
    r.reference.last_error = None;
    let _ = r.engine.send(AudioCommand::RemoveReferenceTrack { id });
}

fn set_active(r: &mut Resonance, id: ReferenceId) {
    if r.reference.index_of(id).is_some() {
        r.reference.active_id = Some(id);
        let _ = r.engine.send(AudioCommand::SetActiveReference { id });
    }
}

fn set_ab_source(r: &mut Resonance, source: ABSource) {
    r.reference.monitor.ab_source = source;
    let _ = r.engine.send(AudioCommand::SetABSource { source });
}

fn toggle_ab_source(r: &mut Resonance) {
    let next = match r.reference.monitor.ab_source {
        ABSource::Mix => ABSource::Reference,
        ABSource::Reference => ABSource::Mix,
    };
    set_ab_source(r, next);
}

fn momentary_audition(r: &mut Resonance, pressed: bool) {
    if pressed {
        // Remember the source to return to, then audition the reference.
        // Guard against a double-press leaking the restore target.
        if r.reference.monitor.momentary_restore.is_none() {
            r.reference.monitor.momentary_restore = Some(r.reference.monitor.ab_source);
        }
        set_ab_source(r, ABSource::Reference);
    } else if let Some(restore) = r.reference.monitor.momentary_restore.take() {
        // Only a release whose press was seen restores anything. The press
        // is focus-gated (typing "b" into a field never auditions) but the
        // release cannot be, and "restoring" the default would knock a
        // manually chosen Reference source back to the mix (UPD-11).
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
    // The app allocates the marker id (FU-A5a) — restored markers exist
    // only here, so the engine cannot pick one that misses them — and
    // lists the marker now; the engine's `RefMarkerAdded` echo is a no-op.
    if r.reference.index_of(ref_id).is_none() {
        return;
    }
    let marker_id = r.reference.alloc_marker_id(ref_id);
    if let Some(entry) = r.reference.entry_mut(ref_id) {
        entry.markers.push(ReferenceMarkerState {
            id: marker_id,
            position_samples,
            label: label.clone(),
        });
    }
    let _ = r.engine.send(AudioCommand::AddRefMarker {
        ref_id,
        marker_id,
        position_samples,
        label,
    });
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
    let enabled = !r.reference.monitor.loop_to_mix;
    r.reference.monitor.loop_to_mix = enabled;
    let _ = r.engine.send(AudioCommand::SetRefLoopToMix { enabled });
}
