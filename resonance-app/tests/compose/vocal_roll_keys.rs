//! The vocal roll's registry keys (command-palette.md §4.3): while it owns
//! the keyboard, `s` / Delete with no note selected do nothing — they are
//! captured, so `s` never falls through to the global Solo Selected.

use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::canvas::Program;
use iced::{mouse, Point, Rectangle, Size};
use resonance_app::state::MidiClipState;
use resonance_app::view::compose::vocal_roll::{VocalRollCanvas, VocalRollState};
use resonance_music_theory::VocalParams;

fn clip() -> MidiClipState {
    MidiClipState {
        id: 7,
        track_id: 3,
        start_sample: 0,
        duration_ticks: 3840,
        name: "vocal".to_owned(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn canvas<'a>(clip: &'a MidiClipState, params: &'a VocalParams) -> VocalRollCanvas<'a> {
    VocalRollCanvas {
        keys_blocked: false,
        keymap: resonance_app::commands::BindingMap::default_ref(),
        clip,
        track_id: 3,
        params,
        chords: &[],
        section_beats: 16,
        scroll_y: 0.0,
        zoom_x: 1.0,
        zoom_y: 1.0,
        snap_ticks: 120,
        selected_note: None,
        time_sig_num: 4,
        bpm: 120.0,
        voice_label: "alto",
        lyrics: &[],
    }
}

fn key(k: Key, text: Option<&str>) -> iced::Event {
    iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: k.clone(),
        modified_key: k,
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyS),
        location: keyboard::Location::Standard,
        modifiers: Modifiers::empty(),
        text: text.map(Into::into),
        repeat: false,
    })
}

#[test]
fn s_without_a_selected_note_is_captured_and_does_nothing() {
    let (c, p) = (clip(), VocalParams::default());
    let canvas = canvas(&c, &p);
    let mut state = VocalRollState::default();
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(800.0, 600.0));
    let cursor = mouse::Cursor::Available(Point::new(400.0, 300.0));
    // A press on the roll gives it the keys.
    let press = iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
    let _ = canvas.update(&mut state, &press, bounds, cursor);
    let release = iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
    let _ = canvas.update(&mut state, &release, bounds, cursor);

    for event in [
        key(Key::Character("s".into()), Some("s")),
        key(Key::Named(Named::Delete), None),
    ] {
        let action = canvas.update(&mut state, &event, bounds, cursor).expect("captured");
        let (message, _redraw, status) = action.into_inner();
        assert!(message.is_none(), "nothing to act on: {message:?}");
        assert_eq!(status, iced::event::Status::Captured, "must not fall through to Solo");
    }
}
