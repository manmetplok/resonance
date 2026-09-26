//! Comp-delayed evaluation of post-PDC automation (doc #260 finding #9,
//! ba todo #1123).
//!
//! Fader/pan/mute automation is applied at sum time, *after* the
//! per-track delay lines — so the audio under the fader is
//! `track_stage()` samples older than the raw playhead. The render core
//! must therefore evaluate those lanes at the comp-delayed position;
//! otherwise a drawn move acts early by the comp delay. Driven through
//! the real `render_block` with a synthetic comp table and a gain lane,
//! plus unit coverage for the `track_stage()` accessor and the
//! MAX_COMP_LATENCY clamp predicate (finding #20).

use resonance_audio::test_support::{
    auto_gain_ramp, comp_latency_clamped, render_aux_with_comp_for_test, AutomationSnapshot,
    LatencyComp, MAX_COMP_LATENCY,
};
use resonance_audio::types::*;
use resonance_common::{real_to_lane_value, AutomationLane, AutomationTarget, Breakpoint, CurveKind};

const TRACK: u64 = 1;
const BLOCK: usize = 512;
const DELAY: u64 = 4;

/// A DC clip several blocks long, so the rendered window sits entirely
/// inside it — clear of the automatic edge declick, which would otherwise
/// shape the very frames these tests measure.
fn dc_clip(frames: usize) -> AudioClip {
    AudioClip {
        id: 10,
        track_id: TRACK,
        start_sample: 0,
        source: ClipSource::Memory(vec![1.0; frames * 2]),
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
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// A linear track-gain fade from 0 dB at frame 0 to -60 dB at `end`.
fn fade_snapshot(end: u64) -> AutomationSnapshot {
    let bp = |frame: u64, db: f32| {
        Breakpoint::new(
            frame,
            real_to_lane_value(&AutomationTarget::TrackGain(TRACK), db),
            CurveKind::Linear,
        )
    };
    let lane = AutomationLane::new(
        1,
        AutomationTarget::TrackGain(TRACK),
        vec![bp(0, 0.0), bp(end, -60.0)],
    );
    let mut snap = AutomationSnapshot::default();
    snap.mix_lanes.insert(lane.target.clone(), lane);
    snap
}

#[test]
fn track_gain_automation_is_evaluated_at_the_comp_delayed_position() {
    // Track 1 carries a 4-sample comp delay; a fade spans frames 0..64.
    // The block [0, 32) applies its gain ramp to audio that is 4 samples
    // old, so the ramp's end value must be the lane's value at frame 28
    // — not at the raw block end 32.
    let fade_end = 2 * BLOCK as u64;
    let comp = LatencyComp::new(DELAY, &[(TRACK, DELAY)], 0, &[]);
    let track = Track::new(TRACK, "t".into());
    track.set_output(TrackOutput::Master);

    let (data, _busses) = render_aux_with_comp_for_test(
        vec![track],
        vec![],
        vec![dc_clip(BLOCK * 4)],
        vec![],
        BLOCK,
        48_000,
        comp,
        fade_snapshot(fade_end),
    );

    // Expected ramp endpoints: the shifted evaluation window
    // [0-4 (sat 0), 32-4].
    let snap = fade_snapshot(fade_end);
    let ((_, shifted_end), _) = auto_gain_ramp(
        &snap,
        AutomationTarget::TrackGain(TRACK),
        AutomationTarget::TrackPan(TRACK),
        1.0,
        0.0,
        0,
        BLOCK as u64 - DELAY,
    )
    .expect("lane present");
    let ((_, raw_end), _) = auto_gain_ramp(
        &snap,
        AutomationTarget::TrackGain(TRACK),
        AutomationTarget::TrackPan(TRACK),
        1.0,
        0.0,
        0,
        BLOCK as u64,
    )
    .expect("lane present");
    assert!(
        (shifted_end - raw_end).abs() > 1e-4,
        "test setup must distinguish the two evaluation positions"
    );

    // The last output frame carries clip content (delayed 4) scaled by
    // the ramp's final per-sample gain, which equals the block-end
    // endpoint (`auto_gain_ramp` endpoints are final gains — the pan
    // law is already folded in).
    let last_l = data[(BLOCK - 1) * 2];
    assert!(
        (last_l - shifted_end).abs() < 1e-5,
        "gain must follow the comp-delayed lane position: got {last_l}, want {shifted_end} \
         (raw would be {raw_end})",
    );
}

#[test]
fn zero_latency_comp_keeps_raw_evaluation() {
    // With no comp delay the shift is 0 and evaluation stays at the raw
    // block positions — the common no-latency case is bit-identical.
    let fade_end = 2 * BLOCK as u64;
    let track = Track::new(TRACK, "t".into());
    track.set_output(TrackOutput::Master);
    let (data, _busses) = render_aux_with_comp_for_test(
        vec![track],
        vec![],
        vec![dc_clip(BLOCK * 4)],
        vec![],
        BLOCK,
        48_000,
        LatencyComp::empty(),
        fade_snapshot(fade_end),
    );
    let snap = fade_snapshot(fade_end);
    let ((_, raw_end), _) = auto_gain_ramp(
        &snap,
        AutomationTarget::TrackGain(TRACK),
        AutomationTarget::TrackPan(TRACK),
        1.0,
        0.0,
        0,
        BLOCK as u64,
    )
    .expect("lane present");
    let last_l = data[(BLOCK - 1) * 2];
    assert!((last_l - raw_end).abs() < 1e-5, "got {last_l}, want {raw_end}");
}

#[test]
fn track_stage_is_total_minus_bus_stage() {
    let comp = LatencyComp::new(7, &[(1, 7)], 3, &[(9, 1)]);
    assert_eq!(comp.max_latency(), 10);
    assert_eq!(comp.bus_stage(), 3);
    assert_eq!(comp.track_stage(), 7);
    assert_eq!(LatencyComp::empty().track_stage(), 0);
}

#[test]
fn clamp_predicate_fires_only_beyond_the_limit() {
    assert!(!comp_latency_clamped(&[]));
    assert!(!comp_latency_clamped(&[(1, 0), (2, MAX_COMP_LATENCY)]));
    assert!(comp_latency_clamped(&[(1, 0), (2, MAX_COMP_LATENCY + 1)]));
}
