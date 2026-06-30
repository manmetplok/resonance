//! Golden-image snapshot for clip fade / gain / crossfade rendering on the
//! arrange-view timeline (todo #320, design doc #153, arch doc #156).
//!
//! Locks in the Canvas additions: fade-in/out ramp lines + darkened
//! wedges, the circular fade-handle and lavender gain beads, the gain
//! body tint + mono dB header tag, the automatic lavender crossfade where
//! two same-track audio clips overlap, and the diagonal "unsupported"
//! hatch on a frozen track's clip.
//!
//! Window size matches the app's default 1440×900. On first run
//! `matches_image()` writes the golden under `tests/snapshots/`;
//! subsequent runs diff against the committed PNG.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::state::{ClipState, FreezeStatus, TrackState, ViewMode};
use resonance_app::{theme, Resonance, STARTUP_TAB};
use resonance_audio::types::FadeCurve;
use resonance_common::FreezeCacheRef;

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

/// A simple ascending/descending peak set so the body shows a waveform
/// under the fade wedges.
fn peaks(n: usize) -> Vec<(f32, f32)> {
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let amp = 0.25 + 0.6 * (t * std::f32::consts::PI).sin();
            (-amp, amp)
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn clip(
    id: u64,
    track_id: u64,
    start_s: f32,
    dur_s: f32,
    fade_in_s: f32,
    fade_out_s: f32,
    gain_db: f32,
) -> ClipState {
    let dur = (dur_s * SR as f32) as u64;
    ClipState {
        id,
        track_id,
        start_sample: (start_s * SR as f32) as u64,
        duration_samples: dur,
        name: format!("take {id}"),
        total_frames: dur,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: (fade_in_s * SR as f32) as u64,
        fade_in_curve: FadeCurve::EqualPower,
        fade_out_frames: (fade_out_s * SR as f32) as u64,
        fade_out_curve: FadeCurve::EqualPower,
        gain_db,
        waveform_peaks: peaks(96),
        vocal_tuning: None,
        asset_ref: None,
    }
}

fn audio_track(id: u64, order: usize, name: &str) -> TrackState {
    let mut t = TrackState::new_audio(id, order);
    t.name = name.to_string();
    t
}

/// Arrange-pinned app seeded with four audio tracks demonstrating every
/// fade/gain/crossfade/frozen surface.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    // Mark a project active so the timeline shows instead of the welcome
    // overlay (demo seeding does this in the real app).
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);

    app.test_push_track(audio_track(1, 0, "Fades"));
    app.test_push_track(audio_track(2, 1, "Gain"));
    app.test_push_track(audio_track(3, 2, "Crossfade"));
    app.test_push_track(audio_track(4, 3, "Frozen"));

    // Track 1 — a clip with both fade-in and fade-out (ramps + beads).
    app.test_push_clip(clip(10, 1, 0.3, 3.5, 0.8, 1.0, 0.0));
    // Track 2 — a loud clip (bright tint + +6.0 dB tag + gain bead) and a
    // quiet clip (darker tint + -8.0 dB tag).
    app.test_push_clip(clip(20, 2, 0.3, 2.5, 0.0, 0.0, 6.0));
    app.test_push_clip(clip(21, 2, 3.2, 2.5, 0.0, 0.0, -8.0));
    // Track 3 — two overlapping clips => automatic crossfade seam.
    app.test_push_clip(clip(30, 3, 0.3, 3.0, 0.0, 0.0, 0.0));
    app.test_push_clip(clip(31, 3, 2.6, 3.0, 0.0, 0.0, 0.0));
    // Track 4 — frozen track: its clip renders the unsupported hatch.
    app.test_push_clip(clip(40, 4, 0.3, 4.0, 0.0, 0.0, 0.0));
    app.test_set_freeze_status(
        4,
        FreezeStatus::Frozen {
            cache_ref: FreezeCacheRef::new(
                "frozen-4.wav".to_string(),
                SR,
                24,
                0,
                resonance_common::FreezeCacheStatus::Frozen,
            ),
        },
    );

    app
}

#[test]
fn clip_fades_gain_crossfade_render() {
    let app = build_app();
    let mut ui =
        Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image("tests/snapshots/clip_fades_gain_crossfade_render.png")
            .expect("matches_image i/o"),
        "snapshot diverged from golden"
    );
}
