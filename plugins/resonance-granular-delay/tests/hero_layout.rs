//! Hero-band geometry (ba todo #1265).
//!
//! `HeroLayout` is the one mapping the frame, the grain cloud and the
//! gestures all read, so it is worth pinning on its own: it is pure
//! (band rect + effective delay in, coordinates out) and needs no egui
//! context to exercise.

#![cfg(feature = "editor")]

use resonance_granular_delay::editor::hero::{HeroLayout, PITCH_RANGE_ST, TAP_HIT_HALF_W};
use wayland_plugin_gui::egui;

const MAX_DELAY_MS: f32 = 4000.0;

fn band() -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 400.0))
}

#[test]
fn plot_leaves_the_ruler_gutter_on_the_right() {
    let layout = HeroLayout::new(band(), 500.0);
    assert_eq!(layout.plot.left(), layout.canvas.left());
    assert_eq!(layout.plot.top(), layout.canvas.top());
    assert_eq!(layout.plot.bottom(), layout.canvas.bottom());
    assert!(
        layout.plot.right() < layout.canvas.right(),
        "the pitch ruler needs a gutter"
    );
}

#[test]
fn the_write_head_is_now_and_time_runs_left() {
    let layout = HeroLayout::new(band(), 500.0);
    // 0 ms behind the head sits at the right edge of the plot.
    assert!((layout.x_of_ms(0.0) - layout.plot.right()).abs() < 1e-3);
    // Older content is further left.
    assert!(layout.x_of_ms(1000.0) < layout.x_of_ms(200.0));
}

#[test]
fn time_axis_round_trips() {
    let layout = HeroLayout::new(band(), 750.0);
    for ms in [0.0f32, 50.0, 375.0, 750.0, 1400.0] {
        let back = layout.ms_of_x(layout.x_of_ms(ms));
        assert!((back - ms).abs() < 1e-2, "{ms} ms -> {back} ms");
    }
}

#[test]
fn pitch_axis_round_trips_and_points_up() {
    let layout = HeroLayout::new(band(), 500.0);
    for st in [-PITCH_RANGE_ST, -12.0, 0.0, 7.0, PITCH_RANGE_ST] {
        let back = layout.st_of_y(layout.y_of_st(st));
        assert!((back - st).abs() < 1e-3, "{st} st -> {back} st");
    }
    // Higher pitch is higher on screen (smaller y).
    assert!(layout.y_of_st(12.0) < layout.y_of_st(0.0));
    assert!((layout.y_of_st(0.0) - layout.mid_y()).abs() < 1e-6);
}

#[test]
fn the_tap_sits_at_the_delay_and_carries_its_grab_zone() {
    let delay_ms = 600.0;
    let layout = HeroLayout::new(band(), delay_ms);
    assert!((layout.tap_x - layout.x_of_ms(delay_ms)).abs() < 1e-3);
    assert!((layout.tap_hit.width() - TAP_HIT_HALF_W * 2.0).abs() < 1e-3);
    assert_eq!(layout.tap_hit.top(), layout.plot.top());
    assert_eq!(layout.tap_hit.bottom(), layout.plot.bottom());
    assert!(layout.tap_hit.contains(egui::pos2(layout.tap_x, layout.plot.center().y)));
}

/// The window follows the delay so the tap stays mid-view, but never
/// past what the buffer holds and never so tight that a short delay
/// fills the band.
#[test]
fn the_window_tracks_the_delay_within_the_buffer() {
    let short = HeroLayout::new(band(), 20.0);
    assert!((short.window_seconds - 1.5).abs() < 1e-6);

    let mid = HeroLayout::new(band(), 1000.0);
    assert!((mid.window_seconds - 2.0).abs() < 1e-6);

    let long = HeroLayout::new(band(), MAX_DELAY_MS);
    assert!((long.window_seconds - 4.0).abs() < 1e-6);
    // At the widest window the tap is still on the plot.
    assert!(long.tap_x >= long.plot.left() && long.tap_x <= long.plot.right());
}
