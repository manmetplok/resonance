//! Vocal pitch-editor (graphical tuning) handlers, doc #160.
//!
//! Todo #359 wires the editor's open/close lifecycle and the analysis
//! request. Opening the editor on a vocal clip records the open clip and
//! sends `AudioCommand::AnalyzeClipPitch`; the engine runs f0 detection +
//! note segmentation off the realtime thread (todo #357) and returns the
//! result as `AudioEvent::ClipPitchDetected`, which the engine-event
//! handler mirrors into that clip's
//! [`ClipState::vocal_tuning`](crate::state::ClipState). The per-note /
//! global edit handlers and the editor view land in later todos.

use iced::Task;
use resonance_audio::types::{AudioCommand, ClipId, TrackType};

use crate::message::{Message, VocalTuningMessage};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: VocalTuningMessage) -> Task<Message> {
    match m {
        VocalTuningMessage::OpenPitchEditor(clip_id) => open_pitch_editor(r, clip_id),
        VocalTuningMessage::ClosePitchEditor => {
            r.interaction.editing_pitch_clip = None;
        }
    }
    Task::none()
}

/// Open the pitch editor on `clip_id` and request analysis. Graphical
/// pitch correction only applies to vocal-track audio clips, so this is a
/// no-op for a non-vocal clip or an unknown id — the editor never opens
/// and no analysis is requested.
fn open_pitch_editor(r: &mut Resonance, clip_id: ClipId) {
    let Some(clip) = r.clips.iter().find(|c| c.id == clip_id) else {
        return;
    };
    let is_vocal = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == clip.track_id)
        .is_some_and(|t| t.track_type == TrackType::Vocal);
    if !is_vocal {
        return;
    }
    r.interaction.editing_pitch_clip = Some(clip_id);
    // Run f0 detection + note segmentation off the realtime thread. The
    // detected contour/notes return via `AudioEvent::ClipPitchDetected`
    // and are folded into the clip's `vocal_tuning` mirror there. A failed
    // send (engine shutting down) just leaves the editor open with no
    // analysis — reopening retries.
    let _ = r.engine.send(AudioCommand::AnalyzeClipPitch { clip_id });
}
