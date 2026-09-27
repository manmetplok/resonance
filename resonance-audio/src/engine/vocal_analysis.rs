//! Off-realtime vocal pitch analysis: monophonic f0 detection (#352) plus
//! note-blob segmentation (#354) over a clip's mono mix. The result is
//! cached on the clip's [`VocalTuning`] and mirrored back to the GUI via
//! `AudioEvent::ClipPitchDetected`.
//!
//! The detector and segmenter live in `resonance-dsp`; this module is the
//! engine glue that pulls the clip's PCM, maps the DSP outputs into the
//! `VocalTuning` cache representation, and runs the whole thing on a
//! short-lived worker thread so the realtime mixer never sees the cost.
//! The worker only *computes*: it posts the result to the engine thread
//! ([`EngineInternal::PitchAnalysed`]), which stores it on the clip and
//! emits the event (code review ARCH-02 B-5 — only the engine thread edits
//! the render graph).

use std::sync::Arc;

use parking_lot::RwLock;
use resonance_dsp::{detect_f0, segment_notes, F0Config, SegmentConfig};

use crate::types::*;

use super::internal::EngineInternal;
use super::thread::{HandlerCtx, HandlerState};

/// Engine handler for [`AudioCommand::AnalyzeClipPitch`]. Spawns a worker
/// thread that snapshots the clip's mono mix and runs f0 detection + note
/// segmentation off the realtime thread ([`analyse_clip_pitch`]); the
/// result comes back through the engine inbox and is applied by
/// [`apply_pitch_analysis`].
///
/// `state` is unused today (analysis allocates no engine-thread ids) but
/// kept in the signature so the handler matches the dispatch convention
/// and can grow stateful bookkeeping later without a call-site churn.
pub(crate) fn handle_analyze_clip_pitch(
    ctx: &HandlerCtx,
    _state: &mut HandlerState,
    clip_id: ClipId,
) {
    let clips_arc = Arc::clone(ctx.clips);
    let shared = Arc::clone(ctx.shared);
    let sample_rate = ctx.sample_rate;

    let spawn_result = std::thread::Builder::new()
        .name("resonance-pitch".into())
        .spawn(move || {
            if let Some(result) = analyse_clip_pitch(&clips_arc, clip_id, sample_rate) {
                shared.inbox.post(result);
            }
        });
    if let Err(e) = spawn_result {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::io(format!(
            "Failed to spawn pitch-analysis thread: {e}"
        ))));
    }
}

/// The worker half: read the clip's mono mix, run the DSP, and return the
/// [`EngineInternal::PitchAnalysed`] to post. `None` — nothing to post —
/// when the clip does not exist.
pub(crate) fn analyse_clip_pitch(
    clips: &RwLock<Vec<AudioClip>>,
    clip_id: ClipId,
    sample_rate: u32,
) -> Option<EngineInternal> {
    let mono = {
        let guard = clips.read();
        let clip = guard.iter().find(|c| c.id == clip_id)?;
        mono_mix(clip.source.as_frames())
    };
    let (contour, notes) = analyze_pitch(&mono, sample_rate);
    Some(EngineInternal::PitchAnalysed {
        clip_id,
        contour,
        notes,
    })
}

/// The engine half ([`EngineInternal::PitchAnalysed`]): store the freshly
/// detected geometry on the clip's [`VocalTuning`] cache, then emit
/// `ClipPitchDetected`. Engine thread.
///
/// A clip deleted while the analysis was in flight makes this a silent
/// no-op — no cache write and no event, mirroring the missing-clip branch
/// of the other clip handlers. Re-analysis replaces the cached
/// contour/notes; any prior per-note edits are dropped along with the old
/// blobs (the geometry they referenced no longer exists).
pub(crate) fn apply_pitch_analysis(
    ctx: &HandlerCtx,
    clip_id: ClipId,
    contour: Vec<F0Frame>,
    notes: Vec<NoteBlob>,
) {
    {
        let mut guard = ctx.clips.write();
        let Some(clip) = guard.iter_mut().find(|c| c.id == clip_id) else {
            return;
        };
        let tuning = clip.vocal_tuning_mut();
        tuning.contour = contour.clone();
        tuning.notes = notes.clone();
        // The cache was resynthesised against the PREVIOUS note geometry,
        // which we just replaced, so it is now stale. Nothing else clears
        // it -- `ensure_tuning_caches` only ever REBUILDS, and only from
        // the four offline paths (bounce/wav/stem/freeze) -- so without
        // this the mixer would keep serving audio retuned to the old
        // notes until the next export.
        clip.tuning_render_cache = None;
    }

    let _ = ctx.event_tx.send(AudioEvent::ClipPitchDetected {
        clip_id,
        notes,
        contour,
    });
}

/// Collapse a stereo-interleaved `[l, r, l, r, …]` buffer to a mono mix
/// (`(l + r) / 2` per frame), matching the averaging the waveform-peak
/// helper uses. The pitch detector is monophonic, so it operates on this
/// single channel.
fn mono_mix(frames: &[f32]) -> Vec<f32> {
    frames
        .chunks_exact(2)
        .map(|lr| (lr[0] + lr[1]) * 0.5)
        .collect()
}

/// Run f0 detection + note segmentation on a `mono` buffer at
/// `sample_rate`, returning the contour and notes in the
/// [`VocalTuning`] cache representation.
///
/// The DSP layer reports frame positions in seconds; this maps them to
/// stereo sample-frame offsets from the clip's audio start (one mono
/// sample equals one stereo frame), the anchor `VocalTuning` uses so the
/// cache survives trims and timeline moves. Note `cents_contour` is
/// re-expressed as deviation from the blob's *mean* pitch (what
/// [`NoteBlob`] documents), whereas the segmenter reports deviation from
/// the nearest semitone.
pub fn analyze_pitch(mono: &[f32], sample_rate: u32) -> (Vec<F0Frame>, Vec<NoteBlob>) {
    let config = F0Config::new(sample_rate as f32);
    let dsp_contour = detect_f0(mono, config);
    let dsp_notes = segment_notes(&dsp_contour, SegmentConfig::default());

    let sr = sample_rate as f32;
    let secs_to_frame = |secs: f32| -> u64 { (secs * sr).round().max(0.0) as u64 };

    let contour = dsp_contour
        .iter()
        .map(|f| F0Frame {
            frame: secs_to_frame(f.time_secs),
            f0_hz: f.f0_hz,
            confidence: f.confidence,
            voiced: f.voiced,
        })
        .collect();

    // The detector's centre times mark frame midpoints a hop apart; extend
    // each note's offset by one hop so `end_frame` is the exclusive bound
    // [`NoteBlob`] expects rather than the last frame's midpoint. Clamp to
    // the clip length and keep the `end >= start` invariant.
    let hop = config.hop_size as u64;
    let total_frames = mono.len() as u64;
    let notes = dsp_notes
        .iter()
        .map(|b| {
            let start_frame = secs_to_frame(b.onset_secs);
            let end_frame = (secs_to_frame(b.offset_secs) + hop)
                .min(total_frames)
                .max(start_frame);
            // Re-base the per-frame cents from "deviation from nearest
            // semitone" (segmenter) to "deviation from mean pitch"
            // (NoteBlob): subtract the blob's mean offset.
            let cents_contour = b
                .cents_contour
                .iter()
                .map(|c| c - b.cents_offset)
                .collect();
            NoteBlob {
                start_frame,
                end_frame,
                mean_pitch_midi: b.midi,
                cents_contour,
                edit: NoteEdit::default(),
            }
        })
        .collect();

    (contour, notes)
}
