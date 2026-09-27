//! The clip-id allocator must never re-issue a loaded clip's id (ba doc
//! #271 V5).
//!
//! Derived MIDI clips and SVS-rendered vocal *audio* clips draw from the
//! same counter — since D-7b the app's one clip allocator,
//! `EntityIds::clips`. A project load once reset that counter and the
//! rebuild passes only bumped it past the loaded MIDI clips, so an audio
//! clip whose id sat above every MIDI id was invisible to the allocator and
//! its id was handed out again — one id was simultaneously a MIDI clip on
//! one track and an audio clip on another, and a client holding a clip id
//! across a render silently addressed the wrong object, or one of a
//! different type. The counter is never reset now, but a loaded project can
//! still hold ids above it (saved by a session whose counter ran further),
//! so the load must raise it past every audio clip.

use resonance_app::project::{LoadedProject, ProjectClip};
use resonance_app::state::ids::CLIP_ID_BASE;
use resonance_app::Resonance;
use resonance_audio::types::{ClipId, TrackId, TrackType};
use std::collections::HashMap;

const VOCAL_TRACK: TrackId = 7;
const AUDIO_TRACK: TrackId = 8;

fn audio_clip(id: ClipId, track_id: TrackId) -> ProjectClip {
    ProjectClip {
        id,
        track_id,
        start_sample: 0,
        name: format!("vocal {id}"),
        total_frames: 48_000,
        trim_start_frames: 0,
        trim_end_frames: 0,
        audio_file: format!("audio/clip_{id}.wav"),
        asset_ref: None,
        fade_in_frames: 0,
        fade_in_curve: "linear".to_owned(),
        fade_out_frames: 0,
        fade_out_curve: "linear".to_owned(),
        gain_db: 0.0,
    }
}

/// Load a project holding `audio_clips` (on a vocal and a plain audio
/// track) into a fresh app, then take the next id the allocator would hand
/// out.
fn next_id_after_load(audio_clips: Vec<ProjectClip>) -> ClipId {
    let (mut source, _task) = Resonance::new_for_test();
    source.test_add_track(VOCAL_TRACK, TrackType::Vocal);
    source.test_add_track(AUDIO_TRACK, TrackType::Audio);
    let mut file = source.test_build_project_file();
    file.clips = audio_clips;

    let (mut app, _task) = Resonance::new_for_test();
    // A fresh app's counter starts at the base — the state that makes the
    // raise on load load-bearing.
    assert_eq!(app.test_next_clip_id(), CLIP_ID_BASE);
    app.test_replay_loaded_project_from(LoadedProject {
        file,
        project_dir: std::env::temp_dir().join("resonance-no-such-bundle"),
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
    });
    app.test_next_clip_id()
}

#[test]
fn a_loaded_vocal_audio_clip_id_is_never_reissued() {
    let live = CLIP_ID_BASE + 24;
    let next = next_id_after_load(vec![audio_clip(live, VOCAL_TRACK)]);
    assert!(
        next > live,
        "allocator would hand out {next}, colliding with live audio clip {live}"
    );
}

/// The raise must not depend on the vocal rebuild claiming the clip: an
/// audio clip is off-limits whether or not it maps to a vocal lane on this
/// load (a track whose type changed, a clip that no longer lines up with a
/// placement).
#[test]
fn an_unclaimed_audio_clip_id_is_still_reserved() {
    let live = CLIP_ID_BASE + 99;
    // Not a vocal track, so `rebuild_vocal_audio_clips` skips it.
    let next = next_id_after_load(vec![audio_clip(live, AUDIO_TRACK)]);
    assert!(
        next > live,
        "allocator would hand out {next}, colliding with live audio clip {live}"
    );
}

/// Legacy engine-allocated clip ids sit below the base and cannot push the
/// counter around: it starts at the base and only goes up. The only thing
/// the load takes from it is the block its replay grants the engine
/// (ARCH-04 D-7d), which starts at the base itself.
#[test]
fn ids_below_the_base_leave_the_counter_alone() {
    let next = next_id_after_load(vec![audio_clip(42, VOCAL_TRACK)]);
    assert_eq!(next, CLIP_ID_BASE + resonance_audio::types::CLIP_GRANT_SIZE);
}
