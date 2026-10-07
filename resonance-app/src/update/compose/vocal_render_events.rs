//! What arrives back from an off-thread vocal render — a take, a
//! failure, or nothing to install — and how each settles the lane's
//! control jobs and render state. Split out of the `ComposeMessage`
//! dispatcher (`update/compose/mod.rs`).

use iced::Task;

use crate::compose::VocalLaneKey;
use crate::message::Message;

use super::{section_meter, vocal_audio_install, vocal_audio_io, vocal_render, vocal_render_state};

/// `VocalAudioReady`: install the take, or discard it.
pub(super) fn ready(
    r: &mut crate::Resonance,
    data: Box<crate::compose::messages::VocalAudioReadyData>,
) -> Task<Message> {
    // Install FIRST: the epoch check inside decides whether this render
    // is current or was superseded (a later `vocal.render` or GUI
    // regeneration re-queued the lane while it was in flight). Only an
    // accepted install may tick the lane off the control-initiated render
    // jobs (doc #265, todo #1156) — ticking before the check let a
    // superseded render resolve a later job `done` while the audio that
    // job asked for was then discarded as stale, so a client that waited
    // on the job read old/no audio back. A track-level `vocal.render`
    // covers every lane on the track, so the job only resolves once the
    // last of them lands; no-op when no control job covers the lane (a
    // GUI-driven render).
    let (definition_id, track_id) = (data.definition_id, data.track_id);
    let render_epoch = data.render_epoch;
    // Sung at a tempo the section no longer has (its placement moved into
    // another tempo region, or the tempo was edited, while the render
    // ran): re-render instead of installing audio that drifts against the
    // grid (FU-V2d).
    if let Some(task) = rerender_on_tempo_change(r, &data) {
        vocal_audio_io::unlink_if_exists(&data.wav_path);
        vocal_audio_install::settle_render_event(r, definition_id, track_id, render_epoch, false);
        return task;
    }
    let accepted = vocal_audio_install::handle_vocal_audio_ready(r, *data);
    if accepted {
        r.mark_edited_without_history();
        r.control
            .jobs
            .complete_vocal_lane(definition_id, track_id, r.revision());
    }
    // A discarded render with no successor in flight fails the jobs
    // waiting on it instead of stranding them (UPD-08).
    vocal_audio_install::settle_render_event(r, definition_id, track_id, render_epoch, accepted);
    Task::none()
}

/// `VocalAudioFailed`: the lane's render failed.
pub(super) fn failed(
    r: &mut crate::Resonance,
    lane: VocalLaneKey,
    render_epoch: u64,
    error: String,
    kind: crate::compose::vocal_svs::VocalRenderErrorKind,
) {
    let (definition_id, track_id) = lane;
    // Same epoch gate as the success path: a failure from a superseded
    // render is moot — a newer render for the lane is already in flight,
    // and *its* outcome is what the lane (and any job waiting on it) will
    // get. Only a current-epoch failure means the lane's audio is
    // genuinely not coming, so only then does it fail the jobs still
    // waiting on this lane — and only those: before the message carried
    // the lane, a GUI regeneration of lane B erroring killed a control
    // job that covered only lane A. The lane keeps whatever take it had.
    let current =
        render_epoch == vocal_audio_install::current_render_epoch(r, definition_id, track_id);
    if current {
        r.control
            .jobs
            .fail_vocal_lane(definition_id, track_id, error.clone(), Some(kind));
        vocal_render_state::record_render_failure(r, lane, kind.into(), error.clone());
        r.compose.last_error = Some(error);
    }
    vocal_audio_install::settle_render_event(r, definition_id, track_id, render_epoch, current);
}

/// `VocalAudioUnavailable`: the render had no notes to sing.
pub(super) fn unavailable(r: &mut crate::Resonance, lane: VocalLaneKey, render_epoch: u64) {
    let (definition_id, track_id) = lane;
    // No notes, nothing sung: no banner. A job waiting on the lane asked
    // for audio that is not coming, so it fails with the reason; a stale
    // epoch settles like any stale event.
    let current =
        render_epoch == vocal_audio_install::current_render_epoch(r, definition_id, track_id);
    if current {
        let reason = "the vocal lane has no notes to sing";
        r.control.jobs.fail_vocal_lane(definition_id, track_id, reason, None);
        vocal_render_state::record_render_failure(
            r,
            lane,
            crate::compose::VocalLaneErrorKind::NoNotes,
            reason.to_owned(),
        );
    }
    vocal_audio_install::settle_render_event(r, definition_id, track_id, render_epoch, current);
}

/// FU-V2d: when a *current* render finished at a tempo the section no
/// longer has, queue a re-render at the section's tempo now and return
/// its task. `None` — install as usual — when the render is stale anyway
/// (the epoch check discards it), the tempo still matches, or no
/// re-render could be queued (then the old-tempo audio is still better
/// than none, and the error the re-render reported is left showing).
fn rerender_on_tempo_change(
    r: &mut crate::Resonance,
    data: &crate::compose::messages::VocalAudioReadyData,
) -> Option<Task<Message>> {
    let (definition_id, track_id) = (data.definition_id, data.track_id);
    if data.render_epoch != vocal_audio_install::current_render_epoch(r, definition_id, track_id)
        || !r.compose.placements.iter().any(|p| p.definition_id == definition_id)
    {
        return None;
    }
    let now = section_meter(r, definition_id).bpm;
    if (now - data.bpm).abs() <= 1e-3 {
        return None;
    }
    let task = vocal_render::rerender_vocal_audio(r, definition_id, track_id);
    let requeued = vocal_audio_install::current_render_epoch(r, definition_id, track_id)
        != data.render_epoch;
    requeued.then_some(task)
}
