//! The derived clip-id allocator must never re-issue a live id (ba doc
//! #271 V5).
//!
//! Derived MIDI clips and SVS-rendered vocal *audio* clips draw from the
//! same `ComposeState` counter. A project load resets that counter and
//! the rebuild passes only bumped it past the loaded MIDI clips, so an
//! audio clip whose id sat above every MIDI id was invisible to the
//! allocator and its id was handed out again — one id was simultaneously
//! a MIDI clip on one track and an audio clip on another, and a client
//! holding a clip id across a render silently addressed the wrong
//! object, or one of a different type.

use resonance_app::compose::{ComposeState, DERIVED_CLIP_ID_BASE};
use resonance_app::state::ClipState;
use resonance_audio::types::{ClipId, FadeCurve, TempoMap, TrackId};
use std::collections::{HashMap, HashSet};

const VOCAL_TRACK: TrackId = 7;

fn audio_clip(id: ClipId, track_id: TrackId) -> ClipState {
    ClipState {
        id,
        track_id,
        start_sample: 0,
        duration_samples: 48_000,
        name: format!("vocal {id}"),
        total_frames: 48_000,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    }
}

/// Rebuild the vocal-audio map the way a project load does, then take
/// the next id the allocator would hand out.
fn next_id_after_load(audio_clips: &[ClipState]) -> ClipId {
    let mut compose = ComposeState::default();
    let mut tempo_map = TempoMap::default();
    tempo_map.rebuild_bar_table(48_000);

    // A fresh `ComposeState` starts its counter at the base, which is
    // also where a project load resets it — the state that makes the
    // reservation pass load-bearing.
    assert_eq!(compose.next_derived_clip_id, DERIVED_CLIP_ID_BASE);

    let paths: HashMap<ClipId, std::path::PathBuf> = HashMap::new();
    let vocal_track_ids: HashSet<TrackId> = [VOCAL_TRACK].into_iter().collect();
    compose.rebuild_vocal_audio_clips(audio_clips, &paths, &vocal_track_ids, &tempo_map);

    compose.fresh_derived_clip_id()
}

#[test]
fn a_loaded_vocal_audio_clip_id_is_never_reissued() {
    let live = DERIVED_CLIP_ID_BASE + 24;
    let next = next_id_after_load(&[audio_clip(live, VOCAL_TRACK)]);
    assert!(
        next > live,
        "allocator handed out {next}, colliding with live audio clip {live}"
    );
}

/// The reservation must not depend on the rebuild claiming the clip: an
/// audio clip in the derived range is off-limits whether or not it maps
/// to a vocal lane on this load (a track whose type changed, a clip that
/// no longer lines up with a placement).
#[test]
fn an_unclaimed_audio_clip_id_is_still_reserved() {
    let live = DERIVED_CLIP_ID_BASE + 99;
    // Not a vocal track, so `rebuild_vocal_audio_clips` skips it.
    let next = next_id_after_load(&[audio_clip(live, VOCAL_TRACK + 1)]);
    assert!(
        next > live,
        "allocator handed out {next}, colliding with live audio clip {live}"
    );
}

/// Ordinary (non-derived) audio clip ids sit below the derived base and
/// must not push the counter around.
#[test]
fn ids_outside_the_derived_range_are_ignored() {
    let next = next_id_after_load(&[audio_clip(42, VOCAL_TRACK)]);
    assert_eq!(next, DERIVED_CLIP_ID_BASE);
}
