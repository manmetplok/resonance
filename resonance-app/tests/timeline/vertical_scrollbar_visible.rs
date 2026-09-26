//! The arrange timeline's in-canvas vertical scrollbar sits at the right
//! edge of the *visible* window (review VIEW-33).
//!
//! The canvas is sized to the whole song (`Fixed(content_w)`) inside the
//! outer horizontal `Scrollable`, so `bounds.width` is the song width and
//! a bar placed at `bounds.width - THICKNESS` was off screen — unseeable
//! and unclickable — in any song wider than the window.

use crate::common;

use iced::{Point, Rectangle, Size};
use iced_test::simulator::Simulator;
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::view::timeline::scrollbar::THICKNESS;
use resonance_app::{theme, Resonance};
use resonance_audio::types::{FadeCurve, TrackType};

const SR: u32 = 48_000;
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Twenty audio tracks (taller than the window) and a five-minute clip
/// (far wider than it at the default 100 px/s zoom).
fn long_song() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    for id in 1..=20 {
        app.test_add_track(id, TrackType::Audio);
    }
    let len = 300 * SR as u64;
    app.test_push_clip(ClipState {
        id: 1,
        track_id: 1,
        start_sample: 0,
        duration_samples: len,
        name: "long".into(),
        total_frames: len,
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
    });
    app
}

#[test]
fn vertical_scrollbar_tracks_the_visible_right_edge() {
    let app = long_song();
    let canvas = Size::new(30_000.0, 700.0);
    // Scrolled 2000 px in, 1200 px of the canvas on screen.
    let visible = Rectangle::new(Point::new(2000.0, 0.0), Size::new(1200.0, 700.0));
    let x = app
        .test_timeline_vscrollbar_x(canvas, Some(visible))
        .expect("20 lanes overflow a 700 px canvas");
    assert_eq!(x, 3200.0 - THICKNESS);

    // No probe write yet (first frame): fall back to the canvas edge.
    let x = app.test_timeline_vscrollbar_x(canvas, None).unwrap();
    assert_eq!(x, 30_000.0 - THICKNESS);
}

#[test]
fn long_song_shows_vertical_scrollbar_at_window_edge() {
    let app = long_song();
    let mut settings = iced::Settings::default();
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![theme::ICON_FONT_BYTES.into()];
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    settings.fonts = fonts;
    settings.default_font = theme::UI_FONT;
    let mut ui = Simulator::with_size(settings, Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/timeline_long_song_vertical_scrollbar.png");
}
