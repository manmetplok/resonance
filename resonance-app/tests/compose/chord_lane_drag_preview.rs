//! The chord lane repaints while a chord is dragged (code review VIEW-20).
//!
//! `CursorMoved` updates the drag's pending start / duration and
//! `draw_into` renders them, but the cache fingerprint only knew *whether*
//! a drag was active — so after the first frame the cached geometry was
//! reused and the chord stayed put until mouse-up, then jumped.

use iced::widget::canvas::Program;
use iced::{mouse, Point, Rectangle, Size};

use resonance_app::compose::ComposeMessage;
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::view::compose::chord_lane::{ChordLaneCanvas, ChordLaneState};
use resonance_app::view::compose::tracks::NAME_COLUMN_WIDTH;
use resonance_app::Resonance;
use resonance_music_theory::{ChordQuality, PitchClass};

/// Grid width: 16 beats (a 4-bar 4/4 section) at 40 px a beat.
const BEAT_PX: f32 = 40.0;
const LANE_Y: f32 = 40.0;

fn app_with_chord() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    let _ = app.update(Message::Compose(ComposeMessage::CreateSection {
        name: "Verse".to_owned(),
        length_bars: 4,
        color: [1, 2, 3],
        place: true,
    }));
    let definition_id = app.compose_state().definitions[0].id;
    let _ = app.update(Message::Compose(ComposeMessage::AddChord {
        definition_id,
        start_beat: 0,
        duration_beats: 4,
        root: PitchClass::A,
        quality: ChordQuality::Min,
    }));
    assert_eq!(app.compose_state().definitions[0].chords.len(), 1);
    app
}

fn at_beat(beat: f32) -> mouse::Cursor {
    mouse::Cursor::Available(Point::new(NAME_COLUMN_WIDTH + beat * BEAT_PX, LANE_Y))
}

fn mouse(
    canvas: &ChordLaneCanvas<'_>,
    state: &mut ChordLaneState,
    event: mouse::Event,
    cursor: mouse::Cursor,
) {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(NAME_COLUMN_WIDTH + 16.0 * BEAT_PX, 64.0));
    let _ = canvas.update(state, &iced::Event::Mouse(event), bounds, cursor);
}

fn moved(cursor: mouse::Cursor) -> mouse::Event {
    let mouse::Cursor::Available(position) = cursor else { unreachable!() };
    mouse::Event::CursorMoved { position }
}

#[test]
fn the_fingerprint_follows_a_move_drag() {
    let app = app_with_chord();
    let definition = &app.compose_state().definitions[0];
    let canvas = ChordLaneCanvas {
        definition,
        tempo_map: app.test_tempo_map(),
        start_bar: 0,
        selected_chord_id: None,
        chords_selected: false,
        visible_x: resonance_app::view::compose::visible_x_window(None),
    };
    let mut state = ChordLaneState::default();

    mouse(&canvas, &mut state, mouse::Event::ButtonPressed(mouse::Button::Left), at_beat(1.5));
    mouse(&canvas, &mut state, moved(at_beat(4.5)), at_beat(4.5));
    let first = canvas.fingerprint(&state);
    mouse(&canvas, &mut state, moved(at_beat(9.5)), at_beat(9.5));
    let second = canvas.fingerprint(&state);
    assert_ne!(first, second, "moving the dragged chord must repaint the lane");
}

#[test]
fn the_fingerprint_follows_a_resize_drag() {
    let app = app_with_chord();
    let definition = &app.compose_state().definitions[0];
    let canvas = ChordLaneCanvas {
        definition,
        tempo_map: app.test_tempo_map(),
        start_bar: 0,
        selected_chord_id: None,
        chords_selected: false,
        visible_x: resonance_app::view::compose::visible_x_window(None),
    };
    let mut state = ChordLaneState::default();

    // Grab the right edge (within the 8 px resize handle).
    mouse(&canvas, &mut state, mouse::Event::ButtonPressed(mouse::Button::Left), at_beat(3.9));
    mouse(&canvas, &mut state, moved(at_beat(5.5)), at_beat(5.5));
    let first = canvas.fingerprint(&state);
    mouse(&canvas, &mut state, moved(at_beat(7.5)), at_beat(7.5));
    let second = canvas.fingerprint(&state);
    assert_ne!(first, second, "resizing the dragged chord must repaint the lane");
}
