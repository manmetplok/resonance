//! Clip warp ("follow tempo") edits: the inspector's warp section, the
//! on-canvas warp-marker gestures and tempo detection.
//!
//! Every edit mutates the [`ClipWarpState`] mirror on the clip first and
//! then sends the matching engine command (`SetClipWarp` /
//! `SetClipWarpMarkers`), like the fade/gain edits beside it in
//! `update::clips`. The engine echoes the stored values back
//! (`engine_events::clips::warp_changed` / `warp_markers_changed`).
//!
//! Undo: the discrete edits are one entry each; a marker drag is one
//! Begin/Commit gesture however far it travels (the markers are in the
//! project file, so the commit sees the change). Typing in the tempo
//! field and asking for a detection record nothing — the field commits
//! through [`ClipWarpMessage::SetWarp`] on Enter, and a detection only
//! becomes an edit when the user applies its result.

use iced::Task;
use resonance_audio::types::{AudioCommand, ClipId, WarpAlgorithm, WarpMarker};

use crate::message::{ClipMessage, Message};
use crate::state::{
    clamp_transpose, clamp_warp_bpm, sort_warp_markers, TempoDetectStatus, WarpBpmDraft,
    WarpMarkerDragState, MAX_WARP_MARKERS, MIN_WARP_MARKER_GAP_BEATS,
};
use crate::Resonance;

#[derive(Debug, Clone)]
pub enum ClipWarpMessage {
    /// Set all four warp scalars at once (the inspector sends the clip's
    /// current values with the one it changes). Tempo and transpose are
    /// clamped to the accepted ranges.
    SetWarp {
        clip_id: ClipId,
        enabled: bool,
        original_bpm: Option<f32>,
        transpose_semitones: f32,
        algorithm: WarpAlgorithm,
    },
    /// Replace the clip's whole marker set — add, remove and clear are all
    /// expressed this way, as the engine command is. Sorted on the way in.
    SetWarpMarkers {
        clip_id: ClipId,
        markers: Vec<WarpMarker>,
    },
    /// The inspector's source-tempo field changed (no edit yet).
    BpmDraftChanged { clip_id: ClipId, text: String },
    /// Enter in the source-tempo field: apply the draft (empty clears the
    /// tempo; an unparsable draft is dropped).
    CommitBpmDraft { clip_id: ClipId },
    /// Run the engine's tempo detector over the clip.
    DetectTempo { clip_id: ClipId },
    /// Begin dragging warp marker `index` of `clip_id`.
    StartMarkerDrag { clip_id: ClipId, index: usize },
    /// Move the dragged marker to pointer x (canvas content coordinates).
    UpdateMarkerDrag(f32),
    /// Release the dragged marker: push the marker set to the engine.
    EndMarkerDrag,
}

impl ClipWarpMessage {
    /// Undo classification (`undo::classify` via `ClipMessage`).
    /// Exhaustive on purpose (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            Self::SetWarp { .. } | Self::SetWarpMarkers { .. } => UndoAction::Record,
            // The draft is transient text; its commit re-enters `update`
            // as a `SetWarp`, which records — and only when the value
            // actually changed, so Enter on an untouched field is no edit.
            Self::BpmDraftChanged { .. } | Self::CommitBpmDraft { .. } => UndoAction::Skip,
            // Analysis only: the clip is unchanged until the result is
            // applied (a `SetWarp`).
            Self::DetectTempo { .. } => UndoAction::Skip,
            Self::StartMarkerDrag { .. } => UndoAction::Begin,
            Self::UpdateMarkerDrag(_) => UndoAction::Skip,
            Self::EndMarkerDrag => UndoAction::Commit,
        }
    }
}

pub fn handle(r: &mut Resonance, m: ClipWarpMessage) -> Task<Message> {
    match m {
        ClipWarpMessage::SetWarp {
            clip_id,
            enabled,
            original_bpm,
            transpose_semitones,
            algorithm,
        } => set_warp(
            r,
            clip_id,
            enabled,
            original_bpm,
            transpose_semitones,
            algorithm,
        ),
        ClipWarpMessage::SetWarpMarkers { clip_id, markers } => {
            set_warp_markers(r, clip_id, markers)
        }
        ClipWarpMessage::BpmDraftChanged { clip_id, text } => {
            r.ui.interaction.warp_bpm_draft = Some(WarpBpmDraft { clip_id, text });
        }
        ClipWarpMessage::CommitBpmDraft { clip_id } => return commit_bpm_draft(r, clip_id),
        ClipWarpMessage::DetectTempo { clip_id } => detect_tempo(r, clip_id),
        ClipWarpMessage::StartMarkerDrag { clip_id, index } => {
            start_marker_drag(r, clip_id, index)
        }
        ClipWarpMessage::UpdateMarkerDrag(x) => update_marker_drag(r, x),
        ClipWarpMessage::EndMarkerDrag => end_marker_drag(r),
    }
    Task::none()
}

/// Mirror and send the four warp scalars.
pub fn set_warp(
    r: &mut Resonance,
    clip_id: ClipId,
    enabled: bool,
    original_bpm: Option<f32>,
    transpose_semitones: f32,
    algorithm: WarpAlgorithm,
) {
    let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) else {
        return;
    };
    let warp = &mut clip.warp;
    warp.enabled = enabled;
    warp.original_bpm = original_bpm.and_then(clamp_warp_bpm);
    warp.transpose_semitones = clamp_transpose(transpose_semitones);
    warp.algorithm = algorithm;
    let cmd = AudioCommand::SetClipWarp {
        clip_id,
        warp_enabled: warp.enabled,
        original_bpm: warp.original_bpm,
        transpose_semitones: warp.transpose_semitones,
        warp_algorithm: warp.algorithm,
    };
    let _ = r.engine.send(cmd);
}

/// Mirror and send a full marker set: non-finite beats dropped, sorted,
/// capped at [`MAX_WARP_MARKERS`].
pub fn set_warp_markers(r: &mut Resonance, clip_id: ClipId, mut markers: Vec<WarpMarker>) {
    let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) else {
        return;
    };
    markers.retain(|m| m.timeline_beat.is_finite());
    sort_warp_markers(&mut markers);
    markers.truncate(MAX_WARP_MARKERS);
    clip.warp.markers = markers.clone();
    let _ = r
        .engine
        .send(AudioCommand::SetClipWarpMarkers { clip_id, markers });
}

/// Apply the tempo field's draft through `SetWarp` (so the edit records),
/// when it names a tempo different from the clip's.
fn commit_bpm_draft(r: &mut Resonance, clip_id: ClipId) -> Task<Message> {
    let Some(draft) = r.ui.interaction.warp_bpm_draft.take() else {
        return Task::none();
    };
    if draft.clip_id != clip_id {
        return Task::none();
    }
    let Some(warp) = r.clips.iter().find(|c| c.id == clip_id).map(|c| c.warp.clone()) else {
        return Task::none();
    };
    let text = draft.text.trim();
    let original_bpm = if text.is_empty() {
        None
    } else {
        match text.parse::<f32>().ok().and_then(clamp_warp_bpm) {
            Some(bpm) => Some(bpm),
            // Not a tempo: drop the draft, keep the clip as it was.
            None => return Task::none(),
        }
    };
    if original_bpm == warp.original_bpm {
        return Task::none();
    }
    r.update(Message::Clip(ClipMessage::Warp(ClipWarpMessage::SetWarp {
        clip_id,
        enabled: warp.enabled,
        original_bpm,
        transpose_semitones: warp.transpose_semitones,
        algorithm: warp.algorithm,
    })))
}

/// Ask the engine for a tempo estimate; the reply lands in
/// `engine_events::clips::tempo_detected`.
pub fn detect_tempo(r: &mut Resonance, clip_id: ClipId) {
    if !r.clips.iter().any(|c| c.id == clip_id) {
        return;
    }
    r.ui
        .interaction
        .tempo_detect
        .insert(clip_id, TempoDetectStatus::Running);
    if r.engine.send(AudioCommand::DetectClipTempo { clip_id }).is_err() {
        r.ui.interaction.tempo_detect.remove(&clip_id);
    }
}

fn start_marker_drag(r: &mut Resonance, clip_id: ClipId, index: usize) {
    let Some(clip) = r.clips.iter().find(|c| c.id == clip_id) else {
        return;
    };
    if index >= clip.warp.markers.len() {
        return;
    }
    let track_id = clip.track_id;
    r.ui.interaction.selected_clip = Some(clip_id);
    r.ui.select_track(Some(track_id));
    r.ui.interaction.warp_marker_drag = Some(WarpMarkerDragState { clip_id, index });
}

/// Slide the dragged marker to the beat under pointer x, snapped to the
/// grid and clamped between its neighbours (and into the clip), so the
/// set never reorders mid-gesture. The marker keeps its source frame —
/// that is what dragging a warp marker means: "this sound, at that beat".
fn update_marker_drag(r: &mut Resonance, x: f32) {
    let Some(drag) = r.ui.interaction.warp_marker_drag.clone() else {
        return;
    };
    let (zoom, sample_rate, bpm) = (r.viewport.zoom, r.sample_rate, r.transport.bpm);
    if zoom <= 0.0 || sample_rate == 0 || bpm <= 0.0 {
        return;
    }
    let raw = ((x / zoom).max(0.0) as f64 * sample_rate as f64) as u64;
    let snapped = crate::view::timeline::snap_sample_to_grid_tempo(
        raw,
        bpm,
        r.transport.time_sig_num,
        sample_rate,
        zoom,
        &r.tempo_map,
    );
    let Some(clip) = r.clips.iter_mut().find(|c| c.id == drag.clip_id) else {
        return;
    };
    let beats_per_frame = bpm as f64 / 60.0 / sample_rate as f64;
    let beat = (snapped as f64 - clip.start_sample as f64) * beats_per_frame;
    let clip_beats = clip.duration_samples as f64 * beats_per_frame;
    let markers = &mut clip.warp.markers;
    let Some(i) = (drag.index < markers.len()).then_some(drag.index) else {
        return;
    };
    let lo = if i > 0 {
        markers[i - 1].timeline_beat + MIN_WARP_MARKER_GAP_BEATS
    } else {
        0.0
    };
    let hi = markers
        .get(i + 1)
        .map_or(clip_beats, |m| m.timeline_beat - MIN_WARP_MARKER_GAP_BEATS);
    if hi < lo {
        return;
    }
    markers[i].timeline_beat = beat.clamp(lo, hi);
}

fn end_marker_drag(r: &mut Resonance) {
    let Some(drag) = r.ui.interaction.warp_marker_drag.take() else {
        return;
    };
    if let Some(clip) = r.clips.iter().find(|c| c.id == drag.clip_id) {
        let markers = clip.warp.markers.clone();
        let _ = r.engine.send(AudioCommand::SetClipWarpMarkers {
            clip_id: drag.clip_id,
            markers,
        });
    }
}
