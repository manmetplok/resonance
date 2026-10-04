//! Golden-image snapshots for clip warp: warped clips on the arrange
//! timeline (the `WARP` badge, the marker strip, marker hairlines and
//! handles) and the clip inspector's WARP section with a detection result
//! on offer.
//!
//! Window size matches the app's default 1440×900. Re-bless with
//! `RESONANCE_BLESS=1`.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::state::{ClipState, ClipWarpState, TrackState, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioEvent, FadeCurve, WarpAlgorithm, WarpMarker};

const WINDOW: (f32, f32) = (1440.0, 900.0);
const SR: u32 = 48_000;
const ZOOM: f32 = 120.0; // px per second

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

/// A pulsing peak set so the markers have a waveform to sit on.
fn peaks(n: usize) -> Vec<(f32, f32)> {
    (0..n)
        .map(|i| {
            let amp = if i % 12 < 3 { 0.85 } else { 0.25 };
            (-amp, amp)
        })
        .collect()
}

fn clip(id: u64, track_id: u64, start_s: f32, dur_s: f32, warp: ClipWarpState) -> ClipState {
    let dur = (dur_s * SR as f32) as u64;
    ClipState {
        id,
        track_id,
        start_sample: (start_s * SR as f32) as u64,
        duration_samples: dur,
        name: format!("loop {id}"),
        total_frames: dur,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::EqualPower,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::EqualPower,
        gain_db: 0.0,
        waveform_peaks: peaks(96),
        vocal_tuning: None,
        asset_ref: None,
        warp,
    }
}

fn marker(source_frame: u64, timeline_beat: f64) -> WarpMarker {
    WarpMarker {
        source_frame,
        timeline_beat,
    }
}

fn audio_track(id: u64, order: usize, name: &str) -> TrackState {
    let mut t = TrackState::new_audio(id, order);
    t.name = name.to_string();
    t
}

/// Three tracks: an unwarped clip (no badge), a warped clip with a known
/// source tempo and three markers, and a warped clip with no tempo yet and
/// no markers (badge + empty strip).
fn build_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);

    app.test_push_track(audio_track(1, 0, "Plain"));
    app.test_push_track(audio_track(2, 1, "Warped"));
    app.test_push_track(audio_track(3, 2, "No tempo"));

    app.test_push_clip(clip(10, 1, 0.3, 4.0, ClipWarpState::default()));
    app.test_push_clip(clip(
        20,
        2,
        0.3,
        4.0,
        ClipWarpState {
            enabled: true,
            original_bpm: Some(96.0),
            transpose_semitones: 0.0,
            algorithm: WarpAlgorithm::Transient,
            markers: vec![
                marker(0, 0.0),
                marker(60_000, 2.0),
                marker(126_000, 4.5),
            ],
        },
    ));
    app.test_push_clip(clip(
        30,
        3,
        0.3,
        3.0,
        ClipWarpState {
            enabled: true,
            ..ClipWarpState::default()
        },
    ));
    app
}

#[test]
fn clip_warp_timeline_render() {
    let app = build_app();
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/clip_warp_timeline_render.png");
}

#[test]
fn clip_warp_inspector_render() {
    let mut app = build_app();
    app.test_set_selected_clip(Some(20));
    // A finished detection, so the status line and its "Use" action show.
    app.test_apply_engine_event(AudioEvent::ClipTempoDetected {
        clip_id: 20,
        bpm: 97.5,
        confidence: 0.82,
    });
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/clip_warp_inspector_render.png");
}
