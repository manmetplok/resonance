//! Aux sends from a multi-output instrument's sub-tracks (field report
//! 2026-10-06 §3).
//!
//! A kit tap ("SM Drums → Snare") can carry a send to a reverb return.
//! The mixer used to apply sends for top-level tracks only, so the snare
//! never reached the room; and the depth measurement still counted the
//! tap as a feeder while letting its DRY route into the return's output,
//! which inflated the return's gain for every track sending there.

use std::sync::Arc;

use crate::multi_out_harness;

use multi_out_harness::{peak, EngineState, PARENT, PORT_LEVELS, SR, TAP_A};
use resonance_audio::test_support::{
    measure_mix_detailed, AutomationSnapshot, MeasureSource, MixMeasurement, StemSource,
};
use resonance_audio::types::*;

/// An empty return bus: no effect, unity fader, so its true gain is 0 dB.
const RETURN: BusId = 60;
/// A top-level audio track next to the kit.
const GUITAR: TrackId = 3;
const CLIP_FRAMES: usize = SR as usize;

fn add_return(state: &EngineState) {
    let bus = Bus::new(RETURN, "Room".into());
    bus.set_is_return(true);
    state.shared.edit_busses(|b| {
        b.insert(RETURN, Arc::new(bus));
    });
}

/// A top-level track playing `pcm` (interleaved stereo) from sample 0.
fn add_audio_track(state: &EngineState, id: TrackId, pcm: Vec<f32>) {
    let track = Track::new(id, format!("track {id}"));
    state.shared.edit_tracks(|m| {
        m.insert(id, Arc::new(track));
    });
    state.shared.edit_clips(|c| {
        c.push(Arc::new(AudioClip {
            id,
            track_id: id,
            start_sample: 0,
            source: ClipSource::memory(pcm),
            name: "clip".into(),
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
            warp_algorithm: WarpAlgorithm::default(),
            warp_markers: Vec::new(),
            tuning_render_cache: None,
        }))
    });
}

/// Zero-mean noise, so it is uncorrelated with the kit's DC taps.
fn noise(seed: u32) -> Vec<f32> {
    let mut state = seed | 1;
    (0..CLIP_FRAMES * 2)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 0.2
        })
        .collect()
}

fn set_sends(state: &EngineState, sends: &[(TrackId, f32, bool, bool)]) {
    let sends: Vec<AuxSend> = sends
        .iter()
        .enumerate()
        .map(|(i, &(from, level_db, pre_fader, enabled))| AuxSend {
            id: 1_000 + i as SendId,
            source: SendSource::Track(from),
            dest: RETURN,
            level_db,
            pre_fader,
            enabled,
        })
        .collect();
    state.shared.aux_sends.store(Arc::new(sends));
}

fn measure_depth(state: &EngineState, targets: Vec<StemSource>) -> Vec<MixMeasurement> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let detail = DetailSet {
        depth: true,
        ..DetailSet::default()
    };
    measure_mix_detailed(
        7,
        targets,
        None,
        MeasureSource::Render,
        detail,
        &state.shared,
        &state.tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &tx,
    );
    let events: Vec<AudioEvent> = rx.try_iter().collect();
    match events.as_slice() {
        [AudioEvent::MixMeasured { results, .. }] => results.clone(),
        other => panic!("expected one MixMeasured, got {other:?}"),
    }
}

fn return_gain(m: &MixMeasurement) -> f32 {
    let depth = m.detail.depth.as_ref().expect("depth was asked for");
    depth
        .sends
        .iter()
        .find(|s| s.bus_id == RETURN)
        .and_then(|s| s.return_gain_db)
        .expect("a measured return gain")
}

fn master_level(state: &EngineState) -> f32 {
    peak(&state.render(StemSource::Master))
}

/// The field report's routing, scaled down: a guitar at -10 dB and a kit
/// tap at -12 dB into an empty return. Its gain is 0 dB; the tap's dry
/// route leaking into the measurement read +10.8 dB.
#[test]
fn an_empty_return_fed_by_a_track_and_a_kit_tap_measures_unity() {
    let state = EngineState::new();
    add_return(&state);
    add_audio_track(&state, GUITAR, noise(11));
    set_sends(&state, &[(GUITAR, -10.0, false, true), (TAP_A, -12.0, false, true)]);

    let r = measure_depth(&state, vec![StemSource::Track(GUITAR), StemSource::Track(TAP_A)]);
    for m in &r {
        let gain = return_gain(m);
        assert!(gain.abs() < 0.5, "{:?}: return gain {gain} dB, expected 0", m.target);
    }
}

/// A pre-fader tap send skips the tap's fader and the kit's group trim,
/// and the measurement divides both back out of the tap's dry energy.
#[test]
fn a_pre_fader_kit_tap_send_measures_unity_through_both_faders() {
    let state = EngineState::new();
    add_return(&state);
    add_audio_track(&state, GUITAR, noise(11));
    state.shared.tracks().get(&TAP_A).unwrap().set_volume(0.5);
    state.shared.tracks().get(&PARENT).unwrap().set_volume(0.5);
    set_sends(&state, &[(GUITAR, -10.0, false, true), (TAP_A, -12.0, true, true)]);

    let r = measure_depth(&state, vec![StemSource::Track(GUITAR)]);
    let gain = return_gain(&r[0]);
    assert!(gain.abs() < 0.5, "return gain {gain} dB, expected 0");
}

/// The send is audible: switching it on adds the tap's level once more
/// through the return.
#[test]
fn a_kit_tap_send_reaches_the_return_and_master() {
    let state = EngineState::new();
    add_return(&state);
    let dry = PORT_LEVELS[1] + PORT_LEVELS[2];

    set_sends(&state, &[(TAP_A, 0.0, false, false)]);
    let off = master_level(&state);
    assert!((off - dry).abs() < 1e-5, "send off: {off}, expected {dry}");

    set_sends(&state, &[(TAP_A, 0.0, false, true)]);
    let on = master_level(&state);
    let expected = dry + PORT_LEVELS[1];
    assert!(
        (on - expected).abs() < 1e-5,
        "send on: {on}, expected {expected} (the tap once more through the return)"
    );

    // Bit-identical with the send disabled again: nothing lingers.
    set_sends(&state, &[(TAP_A, 0.0, false, false)]);
    assert_eq!(master_level(&state), off);
}

/// What one send adds on master, as (post-fader, pre-fader).
fn send_contributions(state: &EngineState, from: TrackId) -> (f32, f32) {
    set_sends(state, &[(from, 0.0, false, false)]);
    let off = master_level(state);
    set_sends(state, &[(from, 0.0, false, true)]);
    let post = master_level(state) - off;
    set_sends(state, &[(from, 0.0, true, true)]);
    let pre = master_level(state) - off;
    (post, pre)
}

/// A tap's send follows its own fader post-fader and ignores it
/// pre-fader, exactly as a top-level track's does.
#[test]
fn pre_and_post_fader_tap_sends_behave_like_top_level_ones() {
    let level = PORT_LEVELS[1];

    let kit = EngineState::new();
    add_return(&kit);
    kit.shared.tracks().get(&TAP_A).unwrap().set_volume(0.5);
    let (tap_post, tap_pre) = send_contributions(&kit, TAP_A);

    let top = EngineState::new();
    add_return(&top);
    add_audio_track(&top, GUITAR, vec![level; CLIP_FRAMES * 2]);
    top.shared.tracks().get(&GUITAR).unwrap().set_volume(0.5);
    let (top_post, top_pre) = send_contributions(&top, GUITAR);

    assert!((tap_post - level * 0.5).abs() < 1e-5, "tap post-fader adds {tap_post}");
    assert!((tap_pre - level).abs() < 1e-5, "tap pre-fader adds {tap_pre}");
    assert!((tap_post - top_post).abs() < 1e-6, "post: tap {tap_post} vs top {top_post}");
    assert!((tap_pre - top_pre).abs() < 1e-6, "pre: tap {tap_pre} vs top {top_pre}");
}

/// Post-fader, the kit's group trim (the parent fader) rides on the tap
/// send as it does on the tap's own route; muting the tap silences both.
#[test]
fn a_post_fader_tap_send_follows_the_group_trim_and_the_taps_mute() {
    let state = EngineState::new();
    add_return(&state);
    state.shared.tracks().get(&PARENT).unwrap().set_volume(0.5);
    let (post, _) = send_contributions(&state, TAP_A);
    assert!((post - PORT_LEVELS[1] * 0.5).abs() < 1e-5, "post-fader adds {post}");

    // The master stem honours mute: a muted tap neither plays nor sends.
    state.shared.tracks().get(&TAP_A).unwrap().set_muted(true);
    set_sends(&state, &[(TAP_A, 0.0, true, true)]);
    let muted = master_level(&state);
    assert!(
        (muted - PORT_LEVELS[2] * 0.5).abs() < 1e-5,
        "muted tap: {muted}, expected only the other tap"
    );
}
