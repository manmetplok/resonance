//! Solo on a multi-output sub-track (BUG-02).
//!
//! An expanded drum kit shows one strip per output port, each with its own
//! S button. Soloing a tap must make that tap — plus anything else soloed
//! — the only thing heard: the kit's own main output (port 0), its other
//! taps and every unsoloed track drop out. The parent still has to RUN,
//! because the tap's audio only exists as one of its instrument's ports;
//! it becomes a fan-out driver, the disposition stems already use.
//!
//! Every case renders twice — through the live audio callback and through
//! the offline master render that `render.mixdown` uses — and both must
//! agree, since they share one solo resolution.
//!
//! Levels are distinct powers of two, so a master sample names exactly
//! which sources reached it: port 0 = 1/2, tap A = 1/4, tap B = 1/8, the
//! other (audio) track = 1/16.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::multi_out_harness::{
    multi_out_instrument, EngineState, FRAMES, INSTRUMENT_ID, PARENT, PORTS, SR, TAP_A, TAP_B,
};
use resonance_audio::test_support::{MixAudioHarness, StemSource};
use resonance_audio::types::*;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const LEVELS: [f32; PORTS] = [0.5, 0.25, 0.125];
const MAIN: f32 = LEVELS[0];
const A: f32 = LEVELS[1];
const B: f32 = LEVELS[2];
const OTHER_LEVEL: f32 = 0.0625;
const OTHER: TrackId = 3;
const KIT: f32 = MAIN + A + B;
const EVERYTHING: f32 = KIT + OTHER_LEVEL;

const BLOCK: usize = 128;
/// Live blocks rendered before reading: ramps and the clip's head declick
/// are long over.
const LIVE_BLOCKS: usize = 6;

fn dc_clip(track_id: TrackId, level: f32) -> AudioClip {
    AudioClip {
        id: 1,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![level; 8192 * 2]),
        name: "dc".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// What a frozen kit's cache holds: the whole fan-out, baked.
const FROZEN_KIT: f32 = 0.75;

/// Which flags a case sets before rendering.
#[derive(Default, Clone, Copy)]
struct Flags {
    solo: &'static [TrackId],
    mute: &'static [TrackId],
    /// Freeze the kit to a [`FROZEN_KIT`] cache.
    frozen: bool,
}

fn apply(tracks: &TrackMap, flags: Flags) {
    if flags.frozen {
        let frames = 8192;
        let cache_ref =
            FreezeCacheRef::new("kit.wav".into(), SR, 32, 1, FreezeCacheStatus::Frozen);
        let pcm = Arc::new(vec![FROZEN_KIT; frames * 2]);
        let source = FrozenSource::new(cache_ref, pcm, SR, frames as u64);
        tracks.get(&PARENT).unwrap().frozen_source.store(Some(Arc::new(source)));
    }
    for id in flags.solo {
        tracks.get(id).unwrap().set_soloed(true);
    }
    for id in flags.mute {
        tracks.get(id).unwrap().set_muted(true);
    }
}

/// The master's last left sample through the offline master render.
fn offline(flags: Flags) -> f32 {
    let state = EngineState::with_port_levels(LEVELS);
    let other = Track::new(OTHER, "Other".into());
    state.shared.edit_tracks(|m| {
        m.insert(OTHER, Arc::new(other));
    });
    state.shared.edit_clips(|v| v.push(Arc::new(dc_clip(OTHER, OTHER_LEVEL))));
    apply(&state.shared.tracks(), flags);
    let out = state.render(StemSource::Master);
    out[(FRAMES as usize - 1) * 2]
}

/// The master's last left sample after a few live callbacks.
fn live(flags: Flags) -> f32 {
    let parent = Track::with_type(PARENT, "Kit".into(), TrackType::Instrument);
    parent.push_plugin(INSTRUMENT_ID);
    let tracks = vec![
        parent,
        Track::new_sub_track(TAP_A, "Tap 1".into(), PARENT, 1),
        Track::new_sub_track(TAP_B, "Tap 2".into(), PARENT, 2),
        Track::new(OTHER, "Other".into()),
    ];
    let mut h = MixAudioHarness::new(
        tracks,
        Vec::new(),
        vec![dc_clip(OTHER, OTHER_LEVEL)],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.edit_plugins(|p| p.insert(INSTRUMENT_ID, Arc::new(multi_out_instrument(LEVELS))));
    h.shared().playing.store(true, Ordering::Relaxed);
    apply(&h.tracks(), flags);
    let mut last = 0.0;
    for _ in 0..LIVE_BLOCKS {
        last = h.render()[(BLOCK - 1) * 2];
    }
    last
}

fn check(flags: Flags, want: f32, what: &str) {
    let off = offline(flags);
    assert!((off - want).abs() < 1e-6, "{what} (offline): want {want}, got {off}");
    let liv = live(flags);
    assert!((liv - want).abs() < 1e-6, "{what} (live): want {want}, got {liv}");
}

#[test]
fn nothing_soloed_plays_everything() {
    check(Flags::default(), EVERYTHING, "no solo");
}

#[test]
fn a_soloed_tap_is_the_only_thing_heard() {
    let flags = Flags {
        solo: &[TAP_A],
        ..Flags::default()
    };
    check(flags, A, "tap A soloed");
}

#[test]
fn two_soloed_taps_are_heard_together() {
    let flags = Flags {
        solo: &[TAP_A, TAP_B],
        ..Flags::default()
    };
    check(flags, A + B, "taps A and B soloed");
}

#[test]
fn a_soloed_tap_and_a_soloed_track_add_up() {
    let flags = Flags {
        solo: &[TAP_B, OTHER],
        ..Flags::default()
    };
    check(flags, B + OTHER_LEVEL, "tap B and the other track soloed");
}

#[test]
fn a_soloed_parent_still_plays_the_whole_kit() {
    let flags = Flags {
        solo: &[PARENT],
        ..Flags::default()
    };
    check(flags, KIT, "parent soloed");
    let flags = Flags {
        solo: &[PARENT, TAP_A],
        ..Flags::default()
    };
    check(flags, KIT, "parent and tap A soloed");
}

/// A soloed track that is muted stays silent, top-level or tap; a muted
/// parent silences its taps whatever their solo.
#[test]
fn mute_wins_over_a_tap_solo() {
    let flags = Flags {
        solo: &[TAP_A],
        mute: &[TAP_A],
        ..Flags::default()
    };
    check(flags, 0.0, "tap A soloed and muted");
    let flags = Flags {
        solo: &[TAP_A],
        mute: &[PARENT],
        ..Flags::default()
    };
    check(flags, 0.0, "tap A soloed under a muted parent");
}

/// A frozen kit's taps are baked into one cache and cannot be separated,
/// so soloing one of its taps plays the whole frozen kit — still a solo
/// (the other track drops out), never silence.
#[test]
fn a_soloed_tap_of_a_frozen_kit_plays_the_frozen_kit() {
    let flags = Flags {
        frozen: true,
        ..Flags::default()
    };
    check(flags, FROZEN_KIT + OTHER_LEVEL, "frozen kit, no solo");
    let flags = Flags {
        solo: &[TAP_A],
        frozen: true,
        ..Flags::default()
    };
    check(flags, FROZEN_KIT, "frozen kit, tap A soloed");
}
