use resonance_common::automation::{
    lane_value_to_plugin_param, lane_value_to_real, plugin_param_to_lane_value,
    real_to_lane_value, sample_lane, AutomationLane, AutomationTarget, Breakpoint, CurveKind,
    GAIN_MAX_DB, GAIN_MIN_DB,
};

fn approx(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
}

fn lin(time: u64, value: f32) -> Breakpoint {
    Breakpoint::new(time, value, CurveKind::Linear)
}

fn step(time: u64, value: f32) -> Breakpoint {
    Breakpoint::new(time, value, CurveKind::Stepped)
}

// --- sample_lane ---------------------------------------------------------

#[test]
fn empty_lane_samples_zero() {
    approx(sample_lane(&[], 0), 0.0);
    approx(sample_lane(&[], 9999), 0.0);
}

#[test]
fn single_point_is_constant_everywhere() {
    let pts = [lin(100, 0.7)];
    approx(sample_lane(&pts, 0), 0.7);
    approx(sample_lane(&pts, 100), 0.7);
    approx(sample_lane(&pts, 1_000_000), 0.7);
}

#[test]
fn sampling_at_breakpoints_returns_their_values() {
    let pts = [lin(0, 0.0), lin(100, 0.5), lin(200, 1.0)];
    approx(sample_lane(&pts, 0), 0.0);
    approx(sample_lane(&pts, 100), 0.5);
    approx(sample_lane(&pts, 200), 1.0);
}

#[test]
fn linear_interpolates_between_points() {
    let pts = [lin(0, 0.0), lin(100, 1.0)];
    approx(sample_lane(&pts, 25), 0.25);
    approx(sample_lane(&pts, 50), 0.5);
    approx(sample_lane(&pts, 75), 0.75);
}

#[test]
fn stepped_holds_left_value_until_next_point() {
    let pts = [step(0, 0.2), step(100, 0.8)];
    approx(sample_lane(&pts, 0), 0.2);
    approx(sample_lane(&pts, 50), 0.2);
    approx(sample_lane(&pts, 99), 0.2);
    approx(sample_lane(&pts, 100), 0.8);
    approx(sample_lane(&pts, 150), 0.8);
}

#[test]
fn curve_kind_of_left_point_decides_the_segment() {
    // Left point stepped, right point linear: the stepped segment holds.
    let pts = [step(0, 0.1), lin(100, 0.9)];
    approx(sample_lane(&pts, 50), 0.1);
    // Left point linear: interpolate regardless of the right point's curve.
    let pts = [lin(0, 0.1), step(100, 0.9)];
    approx(sample_lane(&pts, 50), 0.5);
}

#[test]
fn outside_range_clamps_to_end_values() {
    let pts = [lin(100, 0.3), lin(200, 0.6)];
    approx(sample_lane(&pts, 0), 0.3);
    approx(sample_lane(&pts, 50), 0.3);
    approx(sample_lane(&pts, 99), 0.3);
    approx(sample_lane(&pts, 250), 0.6);
    approx(sample_lane(&pts, u64::MAX), 0.6);
}

// --- AutomationLane ------------------------------------------------------

#[test]
fn new_sorts_out_of_order_points() {
    let lane = AutomationLane::new(
        1,
        AutomationTarget::MasterGain,
        vec![lin(200, 1.0), lin(0, 0.0), lin(100, 0.5)],
    );
    let times: Vec<u64> = lane.points.iter().map(|p| p.time_frames).collect();
    assert_eq!(times, vec![0, 100, 200]);
    assert!(lane.enabled);
    approx(lane.sample(50), 0.25);
}

#[test]
fn insert_point_keeps_sorted_order() {
    let mut lane = AutomationLane::new(
        7,
        AutomationTarget::TrackGain(3),
        vec![lin(0, 0.0), lin(200, 1.0)],
    );
    lane.insert_point(lin(100, 0.5));
    let times: Vec<u64> = lane.points.iter().map(|p| p.time_frames).collect();
    assert_eq!(times, vec![0, 100, 200]);
}

#[test]
fn breakpoint_value_is_clamped() {
    let over = Breakpoint::new(0, 1.5, CurveKind::Linear);
    let under = Breakpoint::new(0, -0.5, CurveKind::Linear);
    approx(over.value, 1.0);
    approx(under.value, 0.0);
}

// --- value mapping -------------------------------------------------------

#[test]
fn gain_maps_normalized_to_db_range() {
    let t = AutomationTarget::TrackGain(0);
    approx(lane_value_to_real(t, 0.0), GAIN_MIN_DB);
    approx(lane_value_to_real(t, 1.0), GAIN_MAX_DB);
    approx(lane_value_to_real(t, 0.5), (GAIN_MIN_DB + GAIN_MAX_DB) / 2.0);
    // Master and bus gain share the same range.
    approx(
        lane_value_to_real(AutomationTarget::MasterGain, 1.0),
        GAIN_MAX_DB,
    );
    approx(
        lane_value_to_real(AutomationTarget::BusGain(2), 0.0),
        GAIN_MIN_DB,
    );
}

#[test]
fn pan_maps_normalized_to_minus_one_to_one() {
    let t = AutomationTarget::TrackPan(0);
    approx(lane_value_to_real(t, 0.0), -1.0);
    approx(lane_value_to_real(t, 0.5), 0.0);
    approx(lane_value_to_real(t, 1.0), 1.0);
}

#[test]
fn mute_thresholds_at_half() {
    let t = AutomationTarget::TrackMute(0);
    approx(lane_value_to_real(t, 0.0), 0.0);
    approx(lane_value_to_real(t, 0.49), 0.0);
    approx(lane_value_to_real(t, 0.5), 1.0);
    approx(lane_value_to_real(t, 1.0), 1.0);
}

#[test]
fn plugin_param_target_mapping_is_identity() {
    let t = AutomationTarget::PluginParam {
        instance: 4,
        param_id: 12,
    };
    approx(lane_value_to_real(t, 0.3), 0.3);
    approx(real_to_lane_value(t, 0.3), 0.3);
}

#[test]
fn gain_round_trips() {
    let t = AutomationTarget::TrackGain(0);
    for &v in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
        approx(real_to_lane_value(t, lane_value_to_real(t, v)), v);
    }
}

#[test]
fn pan_round_trips() {
    let t = AutomationTarget::BusPan(1);
    for &v in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
        approx(real_to_lane_value(t, lane_value_to_real(t, v)), v);
    }
}

#[test]
fn real_to_lane_clamps_out_of_range_input() {
    let t = AutomationTarget::TrackGain(0);
    approx(real_to_lane_value(t, GAIN_MAX_DB + 50.0), 1.0);
    approx(real_to_lane_value(t, GAIN_MIN_DB - 50.0), 0.0);
}

#[test]
fn plugin_param_scales_to_explicit_range() {
    // 0..127 MIDI-style range.
    assert!((lane_value_to_plugin_param(0.0, 0.0, 127.0) - 0.0).abs() < 1e-9);
    assert!((lane_value_to_plugin_param(1.0, 0.0, 127.0) - 127.0).abs() < 1e-9);
    assert!((lane_value_to_plugin_param(0.5, 0.0, 127.0) - 63.5).abs() < 1e-9);
    // Inverse round-trips.
    let v = plugin_param_to_lane_value(63.5, 0.0, 127.0);
    approx(v, 0.5);
}

#[test]
fn plugin_param_degenerate_range_maps_to_zero() {
    approx(plugin_param_to_lane_value(5.0, 5.0, 5.0), 0.0);
}

// --- serde ---------------------------------------------------------------

#[test]
fn lane_round_trips_through_json() {
    let lane = AutomationLane::new(
        42,
        AutomationTarget::PluginParam {
            instance: 9,
            param_id: 3,
        },
        vec![lin(0, 0.0), step(480, 1.0), lin(960, 0.25)],
    );
    let json = serde_json::to_string(&lane).expect("serialize");
    let back: AutomationLane = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(lane, back);
}
