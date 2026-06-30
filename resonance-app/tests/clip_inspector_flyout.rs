//! Clip fade/gain **inspector flyout** (todo #319, design doc #153, arch
//! doc #156).
//!
//! The flyout is a warm card shown over the arrange area for the selected
//! editable audio clip. These tests lock its view-layer behaviour:
//!
//! - it appears (with its fade/gain controls) only for an editable audio
//!   clip, and is absent when nothing is selected;
//! - a frozen track or a source-less clip degrades to the `BAD`-toned
//!   banner with the fade controls hidden (gain still applies);
//! - its controls emit the shared `ClipMessage` edits (handled by todo
//!   #317), and applying those edits updates the same `ClipState` mirror
//!   the on-canvas drags use — so the flyout and the canvas agree.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ClipMessage, Message};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::FadeCurve;
use resonance_common::freeze::{FreezeCacheRef, FreezeCacheStatus};

const WINDOW: (f32, f32) = (1440.0, 900.0);
const SAMPLE_RATE: u32 = 48_000;

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

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// An audio clip with the given fade/gain state. `total_frames > 0` marks
/// it as having an editable sample source.
fn clip(id: u64, track_id: u64) -> ClipState {
    ClipState {
        id,
        track_id,
        start_sample: 0,
        duration_samples: 96_000,
        name: format!("take {id}"),
        total_frames: 96_000,
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
    }
}

/// A live arrange-view app with a single audio clip, optionally selected.
fn app_with_clip(clip: ClipState, select: bool) -> Resonance {
    let mut app = Resonance::new().0;
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_sample_rate(SAMPLE_RATE);
    let id = clip.id;
    app.test_push_clip(clip);
    if select {
        // Selection is a side effect of starting a clip interaction; a
        // gain grab + release selects the clip without changing any value.
        app.test_dispatch(Message::Clip(ClipMessage::StartClipGainDrag {
            clip_id: id,
            anchor_y: 0.0,
        }));
        app.test_dispatch(Message::Clip(ClipMessage::EndClipGainDrag));
    }
    app
}

#[test]
fn flyout_present_for_selected_editable_clip() {
    let app = app_with_clip(clip(7, 1), true);
    let mut ui = simulator(&app);

    ui.find("CLIP").expect("warm CLIP pill is shown");
    ui.find("FADE IN").expect("fade-in control is shown");
    ui.find("FADE OUT").expect("fade-out control is shown");
    ui.find("GAIN").expect("gain control is shown");
    ui.find("Reset to default")
        .expect("reset action is shown");
    // The curve picker exposes all three curves per fade.
    ui.find("Eq-pow").expect("equal-power curve segment is shown");
}

#[test]
fn flyout_absent_without_selection() {
    let app = app_with_clip(clip(7, 1), false);
    let mut ui = simulator(&app);

    assert!(
        ui.find("FADE IN").is_err(),
        "the inspector flyout must not render when no clip is selected"
    );
}

#[test]
fn source_less_clip_degrades_to_banner() {
    let mut c = clip(7, 1);
    c.total_frames = 0; // no editable sample source
    let app = app_with_clip(c, true);
    let mut ui = simulator(&app);

    ui.find("Fades unavailable")
        .expect("source-less clip shows the BAD-toned banner");
    ui.find("Clip gain still applies.")
        .expect("banner notes that gain still applies");
    // Gain stays available, fade controls are hidden.
    ui.find("GAIN").expect("gain control remains for a degraded clip");
    assert!(
        ui.find("FADE IN").is_err(),
        "fade controls are hidden for a source-less clip"
    );
}

#[test]
fn frozen_clip_degrades_to_banner() {
    let app = {
        let mut app = Resonance::new().0;
        app.test_set_active_project(true);
        app.test_set_view_mode(ViewMode::Arrange);
        app.test_set_sample_rate(SAMPLE_RATE);
        app.test_push_clip(clip(7, 1));
        app.test_set_freeze_status(
            1,
            resonance_app::state::FreezeStatus::Frozen {
                cache_ref: FreezeCacheRef::new(
                    "f.wav".into(),
                    SAMPLE_RATE,
                    24,
                    0,
                    FreezeCacheStatus::Frozen,
                ),
            },
        );
        app.test_dispatch(Message::Clip(ClipMessage::StartClipGainDrag {
            clip_id: 7,
            anchor_y: 0.0,
        }));
        app.test_dispatch(Message::Clip(ClipMessage::EndClipGainDrag));
        app
    };
    let mut ui = simulator(&app);

    ui.find("Fades unavailable")
        .expect("frozen clip shows the BAD-toned banner");
    assert!(
        ui.find("FADE IN").is_err(),
        "fade controls are hidden while the track is frozen"
    );
}

/// Collect the messages a single click on `label` produces.
fn click_messages(app: &Resonance, label: &str) -> Vec<Message> {
    let mut ui = simulator(app);
    ui.click(label).unwrap_or_else(|_| panic!("clicking {label:?} should hit a control"));
    ui.into_messages().collect()
}

#[test]
fn reset_button_emits_reset_edit_and_clears_clip() {
    let mut c = clip(7, 1);
    c.fade_in_frames = 4_800;
    c.fade_out_frames = 9_600;
    c.gain_db = -6.0;
    let mut app = app_with_clip(c, true);

    let msgs = click_messages(&app, "Reset to default");
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Message::Clip(ClipMessage::ResetClipFadeGain { clip_id: 7 })
        )),
        "reset button emits ResetClipFadeGain, got {msgs:?}"
    );

    // Applying the emitted edit clears the clip — the same mirror the
    // on-canvas handles read.
    for m in msgs {
        app.test_dispatch(m);
    }
    let c = &app.test_clips()[0];
    assert_eq!(c.fade_in_frames, 0);
    assert_eq!(c.fade_out_frames, 0);
    assert_eq!(c.gain_db, 0.0);
    assert_eq!(c.fade_in_curve, FadeCurve::default());
}

#[test]
fn gain_stepper_emits_gain_edit_and_updates_clip() {
    let mut app = app_with_clip(clip(7, 1), true);

    let msgs = click_messages(&app, "+0.5");
    let gain = msgs.iter().find_map(|m| match m {
        Message::Clip(ClipMessage::SetClipGainDb { clip_id: 7, gain_db }) => Some(*gain_db),
        _ => None,
    });
    let gain = gain.expect("+ stepper emits SetClipGainDb");
    assert!((gain - 0.5).abs() < 1e-4, "one + step is +0.5 dB, got {gain}");

    for m in msgs {
        app.test_dispatch(m);
    }
    assert!((app.test_clips()[0].gain_db - 0.5).abs() < 1e-4);
}

#[test]
fn curve_segment_emits_curve_edit_and_updates_clip() {
    let mut app = app_with_clip(clip(7, 1), true);

    // The first "Exp" segment in the layout is the fade-in picker.
    let msgs = click_messages(&app, "Exp");
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Message::Clip(ClipMessage::SetClipFadeInCurve {
                clip_id: 7,
                curve: FadeCurve::Exp
            })
        )),
        "fade-in curve segment emits SetClipFadeInCurve, got {msgs:?}"
    );

    for m in msgs {
        app.test_dispatch(m);
    }
    let c = &app.test_clips()[0];
    assert_eq!(c.fade_in_curve, FadeCurve::Exp);
    // The fade-out curve is untouched.
    assert_eq!(c.fade_out_curve, FadeCurve::default());
}
