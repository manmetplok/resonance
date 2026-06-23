//! Pure-geometry coverage for the timeline automation-lane renderer
//! (todo #381 / A3, arch doc #162 §3). The canvas drawing itself is
//! verified visually by the `iced_test` golden suite (owned by the
//! e2e-tester); these tests pin the *math* that drives it — the value→y
//! mapping, the Linear/Stepped envelope polyline, the per-track lane
//! pick order, and the live read-out formatting — so a regression in the
//! shape logic fails fast without a screenshot.

use resonance_app::message::{AutomationMessage, Message};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::view::timeline::automation::{
    automation_band, format_real_value, lane_segments_polyline, target_priority, value_to_y,
};
use resonance_app::{theme, Resonance};
use resonance_common::{AutomationTarget, Breakpoint, CurveKind};

/// Identity-ish x mapping for the pure polyline tests: 1 px per frame so a
/// breakpoint at frame `n` lands at x = `n`.
fn x_one_to_one(frames: u64) -> f32 {
    frames as f32
}

fn bp(time_frames: u64, value: f32, curve: CurveKind) -> Breakpoint {
    Breakpoint::new(time_frames, value, curve)
}

#[test]
fn value_axis_maps_top_and_bottom() {
    let (band_top, band_height) = automation_band(100.0);
    // 1.0 sits at the band top, 0.0 at the bottom (axis grows downward).
    assert_eq!(value_to_y(1.0, band_top, band_height), band_top);
    assert_eq!(
        value_to_y(0.0, band_top, band_height),
        band_top + band_height
    );
    // 0.5 lands exactly mid-band.
    assert_eq!(
        value_to_y(0.5, band_top, band_height),
        band_top + band_height / 2.0
    );
}

#[test]
fn value_to_y_clamps_out_of_range() {
    let (band_top, band_height) = automation_band(0.0);
    assert_eq!(value_to_y(2.0, band_top, band_height), band_top);
    assert_eq!(
        value_to_y(-1.0, band_top, band_height),
        band_top + band_height
    );
}

#[test]
fn empty_lane_has_no_polyline() {
    let poly = lane_segments_polyline(&[], 0.0, 500.0, 10.0, 80.0, x_one_to_one);
    assert!(poly.is_empty());
}

#[test]
fn single_point_draws_a_flat_line_across_the_band() {
    let pts = [bp(100, 0.5, CurveKind::Linear)];
    let poly = lane_segments_polyline(&pts, 0.0, 500.0, 10.0, 80.0, x_one_to_one);
    // left lead-in, the point, right lead-out.
    assert_eq!(poly.len(), 3);
    let mid_y = value_to_y(0.5, 10.0, 80.0);
    assert_eq!(poly[0], iced::Point::new(0.0, mid_y));
    assert_eq!(poly[1], iced::Point::new(100.0, mid_y));
    assert_eq!(poly[2], iced::Point::new(500.0, mid_y));
    // Perfectly flat: every y equal.
    assert!(poly.iter().all(|p| (p.y - mid_y).abs() < f32::EPSILON));
}

#[test]
fn linear_segment_is_a_diagonal_between_points() {
    let pts = [
        bp(0, 0.0, CurveKind::Linear),
        bp(100, 1.0, CurveKind::Linear),
    ];
    let poly = lane_segments_polyline(&pts, 0.0, 200.0, 0.0, 100.0, x_one_to_one);
    // lead-in, p0, p1, lead-out — no extra corner point for Linear.
    assert_eq!(poly.len(), 4);
    let y0 = value_to_y(0.0, 0.0, 100.0); // 100.0 (bottom)
    let y1 = value_to_y(1.0, 0.0, 100.0); // 0.0   (top)
    assert_eq!(poly[1], iced::Point::new(0.0, y0));
    assert_eq!(poly[2], iced::Point::new(100.0, y1));
    // The diagonal moves in both x and y between p0 and p1.
    assert_ne!(poly[1].x, poly[2].x);
    assert_ne!(poly[1].y, poly[2].y);
}

#[test]
fn stepped_segment_holds_flat_then_steps_vertically() {
    let pts = [
        bp(0, 0.2, CurveKind::Stepped),
        bp(100, 0.8, CurveKind::Linear),
    ];
    let poly = lane_segments_polyline(&pts, 0.0, 200.0, 0.0, 100.0, x_one_to_one);
    // lead-in, p0, the flat-hold corner at p1.x/p0.value, p1, lead-out.
    assert_eq!(poly.len(), 5);
    let y0 = value_to_y(0.2, 0.0, 100.0);
    let y1 = value_to_y(0.8, 0.0, 100.0);
    // p0 at its own x and value.
    assert_eq!(poly[1], iced::Point::new(0.0, y0));
    // Corner: advanced in x to p1.x but still at p0's value (flat hold).
    assert_eq!(poly[2], iced::Point::new(100.0, y0));
    // Then a pure vertical step at p1.x up to p1's value.
    assert_eq!(poly[3], iced::Point::new(100.0, y1));
    assert_eq!(poly[2].x, poly[3].x);
    assert_ne!(poly[2].y, poly[3].y);
}

#[test]
fn lead_in_and_out_clamp_to_the_end_values() {
    let pts = [
        bp(50, 0.3, CurveKind::Linear),
        bp(150, 0.7, CurveKind::Linear),
    ];
    let poly = lane_segments_polyline(&pts, -20.0, 400.0, 0.0, 100.0, x_one_to_one);
    let first = poly.first().unwrap();
    let last = poly.last().unwrap();
    // Lead-in starts at the left edge holding the first point's value.
    assert_eq!(first.x, -20.0);
    assert_eq!(first.y, value_to_y(0.3, 0.0, 100.0));
    // Lead-out ends at the right edge holding the last point's value.
    assert_eq!(last.x, 400.0);
    assert_eq!(last.y, value_to_y(0.7, 0.0, 100.0));
}

#[test]
fn lane_pick_order_prefers_gain_then_pan_then_mute_then_params() {
    let g = target_priority(AutomationTarget::TrackGain(1));
    let p = target_priority(AutomationTarget::TrackPan(1));
    let m = target_priority(AutomationTarget::TrackMute(1));
    let param = target_priority(AutomationTarget::PluginParam {
        instance: 9,
        param_id: 0,
    });
    assert!(g < p && p < m && m < param);
    // Lower CLAP param ids sort before higher ones so the choice is stable.
    let param_hi = target_priority(AutomationTarget::PluginParam {
        instance: 9,
        param_id: 5,
    });
    assert!(param < param_hi);
}

#[test]
fn live_readout_formats_per_target_kind() {
    // Gain → signed dB.
    assert_eq!(
        format_real_value(AutomationTarget::TrackGain(1), -6.0),
        "-6.0 dB"
    );
    // Pan → centre / left / right.
    assert_eq!(format_real_value(AutomationTarget::TrackPan(1), 0.0), "C");
    assert_eq!(format_real_value(AutomationTarget::TrackPan(1), -0.5), "L50");
    assert_eq!(format_real_value(AutomationTarget::TrackPan(1), 1.0), "R100");
    // Mute → On/Off threshold at 0.5.
    assert_eq!(format_real_value(AutomationTarget::TrackMute(1), 1.0), "On");
    assert_eq!(format_real_value(AutomationTarget::TrackMute(1), 0.0), "Off");
    // Plugin param → two decimals.
    assert_eq!(
        format_real_value(
            AutomationTarget::PluginParam {
                instance: 1,
                param_id: 2
            },
            0.736
        ),
        "0.74"
    );
}

// ---- End-to-end render smoke test ----
//
// Drives the real `Resonance::view()` through the iced simulator with a
// seeded automation lane on the Arrange tab, exercising the actual canvas
// draw path (`draw_automation_lanes` + the uncached live-value overlay). It
// does not compare against a golden image — the env-divergent PNG goldens are
// blessed by the e2e-tester — so this just proves the draw code renders
// without panicking when lanes are present.

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

fn send(app: &mut Resonance, m: AutomationMessage) {
    let _ = app.update(Message::Automation(m));
}

#[test]
fn arrange_view_renders_with_a_seeded_automation_lane() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Arrange);

    let mut track = TrackState::new_audio(1, 0);
    track.volume = -6.0;
    app.test_push_track(track);

    // A gain lane with a Linear then a Stepped segment so both branches of
    // the envelope renderer execute.
    let target = AutomationTarget::TrackGain(1);
    let sr = 48_000u64;
    send(
        &mut app,
        AutomationMessage::AddBreakpoint {
            target,
            time_frames: 0,
            value: 0.2,
            curve: CurveKind::Linear,
        },
    );
    send(
        &mut app,
        AutomationMessage::AddBreakpoint {
            target,
            time_frames: sr, // ~1s in
            value: 0.9,
            curve: CurveKind::Stepped,
        },
    );
    send(
        &mut app,
        AutomationMessage::AddBreakpoint {
            target,
            time_frames: sr * 2,
            value: 0.5,
            curve: CurveKind::Linear,
        },
    );

    let lane = app
        .test_automation()
        .lanes
        .get(&target)
        .expect("seeded lane present");
    assert_eq!(lane.points.len(), 3, "three breakpoints seeded");

    let mut ui = iced_test::simulator::Simulator::with_size(
        sim_settings(),
        iced::Size::new(1440.0, 900.0),
        app.view(),
    );
    // Rendering exercises the cached lane layer; a clean snapshot means the
    // automation draw path produced valid geometry.
    ui.snapshot(&theme::resonance_theme())
        .expect("arrange view with an automation lane should render");
}
