//! Per-block application of automation lanes in the shared render core
//! (ba todo #376, doc #162 §2).
//!
//! The render core resolves each enabled lane into the concrete values
//! it applies: gain/pan lanes become per-block stereo-gain ramp
//! endpoints, mute lanes a per-block boolean, the master lane a per-block
//! volume, and plugin-param lanes a normalized value mapped into the
//! plugin's range. These tests drive the resolution helpers directly
//! (they are pure and shared verbatim by the live and bounce paths) and
//! feed their output through the real gain-application helper
//! (`sum_to_output`) to show an automated fade actually reaches the mix.

use resonance_audio::__test_support::{
    auto_gain_ramp, auto_master_volume, auto_muted, sum_to_output, AutomationSnapshot,
    ResolvedParamLane,
};
use resonance_common::{
    lane_value_to_plugin_param, real_to_lane_value, AutomationLane, AutomationTarget, Breakpoint,
    CurveKind,
};

const TRACK: u64 = 1;
const BLOCK: usize = 512;

/// A normalized breakpoint at `frame` for the dB value `db` on a gain
/// target (so the test reads in real units).
fn gain_bp(frame: u64, db: f32) -> Breakpoint {
    Breakpoint::new(
        frame,
        real_to_lane_value(&AutomationTarget::TrackGain(TRACK), db),
        CurveKind::Linear,
    )
}

/// Snapshot holding a single track-gain fade from `0 dB` at frame 0 down
/// to the lane floor (`-60 dB`, i.e. silence) at `end`.
fn fade_out_snapshot(end: u64) -> AutomationSnapshot {
    let lane = AutomationLane::new(
        1,
        AutomationTarget::TrackGain(TRACK),
        vec![gain_bp(0, 0.0), gain_bp(end, -60.0)],
    );
    let mut snap = AutomationSnapshot::default();
    snap.mix_lanes.insert(lane.target.clone(), lane);
    snap
}

#[test]
fn track_gain_fade_ramps_down_and_is_continuous_across_blocks() {
    let fade_frames = (BLOCK * 8) as u64;
    let snap = fade_out_snapshot(fade_frames);
    let blocks = fade_frames as usize / BLOCK;

    let mut prev_end_l = f32::INFINITY;
    for b in 0..blocks {
        let start = (b * BLOCK) as u64;
        let end = start + BLOCK as u64;
        let ((gl_start, gl_end), (_gr_start, _gr_end)) = auto_gain_ramp(
            &snap,
            AutomationTarget::TrackGain(TRACK),
            AutomationTarget::TrackPan(TRACK),
            1.0, // static volume (unused — gain lane present)
            0.0, // static pan (centre)
            start,
            end,
        )
        .expect("gain lane present ⇒ Some");

        // Strictly descending fade.
        assert!(
            gl_end < gl_start,
            "block {b}: end {gl_end} should be below start {gl_start}",
        );
        // Continuous: this block's start equals the previous block's end
        // (both evaluate the lane at the same boundary frame), so the
        // per-sample ramp chains into one smooth sweep with no seam jump.
        if b > 0 {
            assert!(
                (gl_start - prev_end_l).abs() < 1e-6,
                "block {b}: start {gl_start} != previous end {prev_end_l}",
            );
        }
        prev_end_l = gl_end;
    }

    // First block starts near unity (0 dB), final block ends at silence.
    let first = auto_gain_ramp(
        &snap,
        AutomationTarget::TrackGain(TRACK),
        AutomationTarget::TrackPan(TRACK),
        1.0,
        0.0,
        0,
        BLOCK as u64,
    )
    .unwrap();
    assert!((first.0 .0 - 0.707).abs() < 0.01, "centre-panned unity ≈ 0.707");
    assert_eq!(prev_end_l, 0.0, "fade lands exactly on silence");
}

#[test]
fn automated_fade_drives_the_mix_to_silence() {
    // Feed a constant source through the automated gain ramp via the same
    // `sum_to_output` helper the render core uses: the output envelope
    // must fall monotonically to (near) zero — an audible fade.
    let fade_frames = (BLOCK * 6) as u64;
    let snap = fade_out_snapshot(fade_frames);
    let src = vec![1.0f32; BLOCK];

    let mut last_peak = f32::INFINITY;
    let mut final_sample = f32::NAN;
    for b in 0..(fade_frames as usize / BLOCK) {
        let start = (b * BLOCK) as u64;
        let (gain_l, gain_r) = auto_gain_ramp(
            &snap,
            AutomationTarget::TrackGain(TRACK),
            AutomationTarget::TrackPan(TRACK),
            1.0,
            0.0,
            start,
            start + BLOCK as u64,
        )
        .unwrap();

        let mut data = vec![0.0f32; BLOCK * 2];
        sum_to_output(&mut data, 2, BLOCK, &src, &src, gain_l, gain_r);

        // The block's loudest sample never exceeds the previous block's —
        // the fade only ever quietens.
        let peak = data.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak <= last_peak + 1e-6, "block {b} peak {peak} rose above {last_peak}");
        last_peak = peak;
        final_sample = data[BLOCK * 2 - 1];
    }
    // The ramp lands exactly on the lane floor (silence) at its last frame.
    assert_eq!(final_sample, 0.0, "fade reaches exact silence at its end");
}

#[test]
fn no_lane_yields_no_override() {
    let snap = AutomationSnapshot::default();
    assert!(
        auto_gain_ramp(
            &snap,
            AutomationTarget::TrackGain(TRACK),
            AutomationTarget::TrackPan(TRACK),
            0.5,
            0.0,
            0,
            BLOCK as u64,
        )
        .is_none(),
        "absent lane ⇒ None so the static fader applies"
    );
    assert!(auto_muted(&snap, AutomationTarget::TrackMute(TRACK), 0).is_none());
    assert!(auto_master_volume(&snap, 0).is_none());
}

#[test]
fn mute_lane_thresholds_at_half() {
    let lane = AutomationLane::new(
        1,
        AutomationTarget::TrackMute(TRACK),
        vec![
            Breakpoint::new(0, 0.0, CurveKind::Stepped),
            Breakpoint::new(1000, 1.0, CurveKind::Stepped),
        ],
    );
    let mut snap = AutomationSnapshot::default();
    snap.mix_lanes.insert(lane.target.clone(), lane);

    assert_eq!(
        auto_muted(&snap, AutomationTarget::TrackMute(TRACK), 0),
        Some(false),
        "before the mute breakpoint ⇒ unmuted"
    );
    assert_eq!(
        auto_muted(&snap, AutomationTarget::TrackMute(TRACK), 2000),
        Some(true),
        "after the mute breakpoint ⇒ muted"
    );
}

#[test]
fn pan_lane_shifts_stereo_balance() {
    // Hard-left pan: left channel keeps the gain, right collapses to zero.
    let lane = AutomationLane::new(
        1,
        AutomationTarget::TrackPan(TRACK),
        vec![Breakpoint::new(
            0,
            real_to_lane_value(&AutomationTarget::TrackPan(TRACK), -1.0),
            CurveKind::Linear,
        )],
    );
    let mut snap = AutomationSnapshot::default();
    snap.mix_lanes.insert(lane.target.clone(), lane);

    let ((gl, _), (gr, _)) = auto_gain_ramp(
        &snap,
        AutomationTarget::TrackGain(TRACK),
        AutomationTarget::TrackPan(TRACK),
        1.0,
        0.0,
        0,
        BLOCK as u64,
    )
    .expect("pan lane present ⇒ Some");
    assert!(gl > 0.99, "hard-left keeps the left channel near unity: {gl}");
    assert!(gr < 0.01, "hard-left silences the right channel: {gr}");
}

#[test]
fn master_lane_sweeps_volume() {
    let lane = AutomationLane::new(
        1,
        AutomationTarget::MasterGain,
        vec![
            Breakpoint::new(0, real_to_lane_value(&AutomationTarget::MasterGain, 0.0), CurveKind::Linear),
            Breakpoint::new(
                1000,
                real_to_lane_value(&AutomationTarget::MasterGain, -60.0),
                CurveKind::Linear,
            ),
        ],
    );
    let mut snap = AutomationSnapshot::default();
    snap.mix_lanes.insert(lane.target.clone(), lane);

    let start = auto_master_volume(&snap, 0).expect("master lane ⇒ Some");
    let end = auto_master_volume(&snap, 1000).expect("master lane ⇒ Some");
    assert!((start - 1.0).abs() < 0.01, "0 dB ≈ unity, got {start}");
    assert_eq!(end, 0.0, "lane floor ⇒ silence");
}

#[test]
fn plugin_param_lane_maps_into_plugin_range() {
    // A resolved plugin-param lane sweeps normalized 0→1 across its frames;
    // mapped into the plugin's 100..=200 range it sweeps 100→200, the value
    // the render core queues via `set_param`.
    let lane = AutomationLane::new(
        1,
        AutomationTarget::PluginParam { instance: 9, param_id: 7 },
        vec![
            Breakpoint::new(0, 0.0, CurveKind::Linear),
            Breakpoint::new(1000, 1.0, CurveKind::Linear),
        ],
    );
    let resolved = ResolvedParamLane { param_id: 7, lane, min: 100.0, max: 200.0 };

    let at = |frame: u64| lane_value_to_plugin_param(resolved.lane.sample(frame), resolved.min, resolved.max);
    assert!((at(0) - 100.0).abs() < 1e-6, "start of sweep ⇒ min");
    assert!((at(500) - 150.0).abs() < 1e-6, "mid sweep ⇒ midpoint");
    assert!((at(1000) - 200.0).abs() < 1e-6, "end of sweep ⇒ max");
}
