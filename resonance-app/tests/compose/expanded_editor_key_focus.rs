//! The expanded compose editor's `+` / `-` / Escape follow KeyFocus, not
//! the hovering cursor (code review FU-C3).
//!
//! `+`/`-` fired whenever the cursor merely hovered the editor — so typing
//! a "-" into a text field with the mouse resting over it zoomed — and
//! Escape was not gated at all: it collapsed the editor from anywhere,
//! including a text field. Like the timeline, piano roll and vocal roll,
//! the editor now owns the keys only while the latest mouse press landed
//! on it (`resonance_app::focus::KeyFocus`).

use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::canvas::Program as _;
use iced::{mouse, Point, Rectangle};
use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::view::compose::expanded_editor::{ExpandedEditorCanvas, ExpandedEditorState};
use resonance_audio::types::TempoMap;

const BOUNDS: Rectangle = Rectangle {
    x: 0.0,
    y: 0.0,
    width: 800.0,
    height: 400.0,
};
/// On the note grid (right of the keyboard, below the toolbar).
const GRID: (f32, f32) = (400.0, 200.0);
/// Outside the editor.
const ELSEWHERE: (f32, f32) = (400.0, 900.0);

fn run(
    state: &mut ExpandedEditorState,
    tempo_map: &TempoMap,
    event: iced::Event,
    at: (f32, f32),
) -> Option<Message> {
    let canvas = ExpandedEditorCanvas {
        keys_blocked: false,
        keymap: resonance_app::commands::BindingMap::default_ref(),
        track_id: 1,
        midi_clips: &[],
        section_start: 0,
        section_end: 48_000 * 8,
        section_length_bars: 4,
        sample_rate: 48_000,
        tempo_map,
        start_bar: 0,
        scale: None,
        zoom_y: 12.0,
        scroll_x: 0.0,
        scroll_y: 0.0,
    };
    let cursor = mouse::Cursor::Available(Point::new(at.0, at.1));
    canvas
        .update(state, &event, BOUNDS, cursor)
        .and_then(|action| action.into_inner().0)
}

fn press() -> iced::Event {
    iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
}

fn key(key: Key, text: Option<&str>) -> iced::Event {
    iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: key.clone(),
        modified_key: key,
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: Modifiers::empty(),
        text: text.map(Into::into),
        repeat: false,
    })
}

fn plus() -> iced::Event {
    key(Key::Character("+".into()), Some("+"))
}

fn escape() -> iced::Event {
    key(Key::Named(Named::Escape), None)
}

fn is_zoom(m: &Option<Message>) -> bool {
    matches!(m, Some(Message::Compose(ComposeMessage::ExpandedZoomY(_))))
}

fn is_collapse(m: &Option<Message>) -> bool {
    matches!(m, Some(Message::Compose(ComposeMessage::CollapseTrack)))
}

#[test]
fn hovering_alone_does_not_take_the_keys() {
    let map = TempoMap::default();
    let mut state = ExpandedEditorState::default();
    let _ = run(&mut state, &map, press(), ELSEWHERE);
    assert!(!is_zoom(&run(&mut state, &map, plus(), GRID)), "hover zoomed");
    assert!(!is_collapse(&run(&mut state, &map, escape(), GRID)), "hover collapsed");
}

#[test]
fn escape_after_a_press_elsewhere_does_not_collapse() {
    let map = TempoMap::default();
    let mut state = ExpandedEditorState::default();
    let _ = run(&mut state, &map, press(), GRID);
    let _ = run(&mut state, &map, press(), ELSEWHERE);
    assert!(!is_collapse(&run(&mut state, &map, escape(), ELSEWHERE)));
}

#[test]
fn after_a_press_on_the_editor_the_keys_work_wherever_the_cursor_is() {
    let map = TempoMap::default();
    let mut state = ExpandedEditorState::default();
    let _ = run(&mut state, &map, press(), GRID);
    assert!(is_zoom(&run(&mut state, &map, plus(), ELSEWHERE)), "+ zooms");
    assert!(is_collapse(&run(&mut state, &map, escape(), ELSEWHERE)), "Esc collapses");
}
