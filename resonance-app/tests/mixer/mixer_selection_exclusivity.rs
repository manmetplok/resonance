//! Track, bus and master selection are mutually exclusive whichever path
//! selects the track — not only `UiMessage::SelectTrack`.
//!
//! The inspector shows a selected bus first, then the master, then a
//! track. Clip drags, trims, fade / gain handles and the Arrange context
//! menu used to set the track selection directly, so selecting Bass in
//! Arrange left the Mixer inspector on the master or a bus. Every path
//! now goes through `UiTransientState::select_track`; these drive each
//! one with the master or a bus selected first, and check both the
//! state and what the inspector renders.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ClipMessage, Message, MidiClipMessage, UiMessage};
use resonance_app::state::{ClipState, MidiClipState, ViewMode};
use resonance_app::{theme, Resonance};
use resonance_audio::types::{FadeCurve, TrackType};

const AUDIO: u64 = 1;
const INST: u64 = 2;
const BUS: u64 = 1;
const AUDIO_CLIP: u64 = 700;
const MIDI_CLIP: u64 = 701;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_add_track(INST, TrackType::Instrument);
    app.test_add_bus(BUS, "Drum Bus");
    app.test_push_clip(ClipState {
        id: AUDIO_CLIP,
        track_id: AUDIO,
        start_sample: 0,
        duration_samples: 48_000,
        name: "take".into(),
        total_frames: 48_000,
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
    app.test_push_midi_clip(MidiClipState {
        id: MIDI_CLIP,
        track_id: INST,
        start_sample: 0,
        duration_ticks: 3840,
        name: "riff".into(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

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

/// What the inspector is showing, read off its owner-group header.
fn inspector_owner(app: &Resonance) -> &'static str {
    let mut sim = Simulator::with_size(sim_settings(), Size::new(1440.0, 2000.0), app.view());
    if sim.find("DELETE BUS").is_ok() {
        "bus"
    } else if sim.find("BOUNCE TO WAV").is_ok() {
        "master"
    } else if sim.find("MONO").is_ok() {
        "track"
    } else {
        "none"
    }
}

/// Select the master (or the bus), run `select`, and check the track it
/// names took the inspector.
fn assert_takes_selection(name: &str, track: u64, select: impl Fn(&mut Resonance)) {
    for channel in [UiMessage::SelectMaster, UiMessage::SelectBus(Some(BUS))] {
        let mut app = app();
        let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
        let _ = app.update(Message::Ui(channel.clone()));
        assert_ne!(inspector_owner(&app), "track", "{name}: precondition");
        select(&mut app);
        assert_eq!(app.test_selected_track(), Some(track), "{name}");
        assert!(!app.test_selected_master(), "{name} after {channel:?}: master cleared");
        assert_eq!(app.test_selected_bus(), None, "{name} after {channel:?}: bus cleared");
        assert_eq!(
            inspector_owner(&app),
            "track",
            "{name} after {channel:?}: the inspector shows the track"
        );
    }
}

#[test]
fn clip_drag_selects_the_track_over_master_and_bus() {
    assert_takes_selection("midi clip drag", INST, |app| {
        let _ = app.update(Message::MidiClip(MidiClipMessage::StartMidiClipDrag {
            clip_id: MIDI_CLIP,
            grab_offset_x: 0.0,
            start_x: 10.0,
            start_y: 10.0,
        }));
    });
}

#[test]
fn clip_trim_selects_the_track_over_master_and_bus() {
    assert_takes_selection("audio clip trim", AUDIO, |app| {
        let _ = app.update(Message::Clip(ClipMessage::StartClipTrim {
            clip_id: AUDIO_CLIP,
            edge: resonance_app::state::ClipEdge::Right,
            anchor_x: 10.0,
        }));
    });
}

#[test]
fn fade_and_gain_handles_select_the_track_over_master_and_bus() {
    assert_takes_selection("fade handle", AUDIO, |app| {
        let _ = app.update(Message::Clip(ClipMessage::StartClipFadeDrag {
            clip_id: AUDIO_CLIP,
            edge: resonance_app::state::ClipEdge::Left,
            anchor_x: 10.0,
        }));
    });
    assert_takes_selection("gain bead", AUDIO, |app| {
        let _ = app.update(Message::Clip(ClipMessage::StartClipGainDrag {
            clip_id: AUDIO_CLIP,
            anchor_y: 10.0,
        }));
    });
}

#[test]
fn arrange_track_menu_selects_the_track_over_master_and_bus() {
    assert_takes_selection("track context menu", INST, |app| {
        let _ = app.update(Message::Ui(UiMessage::OpenTrackMenu {
            id: INST,
            x: 0.0,
            y: 0.0,
        }));
    });
}

/// A project load starts with no channel in the inspector: the master
/// selection does not survive it (the master itself does).
#[test]
fn project_load_clears_the_master_selection() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::SelectMaster));
    assert!(app.test_selected_master());
    let file = app.test_build_project_file();
    app.test_replay_loaded_project(file);
    assert!(!app.test_selected_master(), "a load clears the master selection");
}
