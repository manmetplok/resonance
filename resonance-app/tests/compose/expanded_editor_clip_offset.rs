//! The expanded Compose editor maps clip-relative note ticks through the
//! clip's offset from the section (code review VIEW-16).
//!
//! It used to draw, hit-test, add and move notes as if every clip started
//! at the section's first bar: a clip starting at song bar 1 edited in a
//! section placed at bar 9 had its notes drawn eight bars early (off the
//! grid), and a click added a note at clip tick 0 — song bar 1, outside
//! the section.

use iced::widget::canvas::Program;
use iced::{mouse, Point, Rectangle, Size};

use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::MidiClipState;
use resonance_app::view::compose::expanded_editor::ExpandedEditorCanvas;
use resonance_audio::types::{MidiNote, TempoMap, TICKS_PER_QUARTER_NOTE};

const SAMPLE_RATE: u32 = 48_000;
const TPQ: u64 = TICKS_PER_QUARTER_NOTE;
const BAR: u64 = 4 * TPQ;
/// 0-based bar the section is placed at (song bar 9).
const SECTION_BAR: u32 = 8;
const CLIP_ID: u64 = 7;
const PITCH: u8 = 60;

// Canvas geometry: a 52 px keyboard column, a 24 px toolbar, and a grid
// 1000 px wide that the 4-bar section fills.
const KEYBOARD_W: f32 = 52.0;
const TOOLBAR_H: f32 = 24.0;
const GRID_W: f32 = 1000.0;
const ZOOM_Y: f32 = 14.0;
const SCROLL_Y: f32 = 800.0;

fn tempo_map() -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = 120.0;
    tm.rebuild_bar_table(SAMPLE_RATE);
    tm
}

/// One clip that starts at song bar 1 and runs 16 bars, so it covers the
/// whole section at bar 9.
fn clip(notes: Vec<MidiNote>) -> Vec<MidiClipState> {
    vec![MidiClipState {
        id: CLIP_ID,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 16 * BAR,
        name: "long clip".to_owned(),
        notes: notes.into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }]
}

fn canvas<'a>(clips: &'a [MidiClipState], tm: &'a TempoMap) -> ExpandedEditorCanvas<'a> {
    ExpandedEditorCanvas {
        track_id: 1,
        midi_clips: clips,
        section_start: tm.bar_to_sample(SECTION_BAR),
        section_end: tm.bar_to_sample(SECTION_BAR + 4),
        section_length_bars: 4,
        sample_rate: SAMPLE_RATE,
        tempo_map: tm,
        start_bar: SECTION_BAR,
        scale: None,
        zoom_y: ZOOM_Y,
        scroll_x: 0.0,
        scroll_y: SCROLL_Y,
    }
}

fn bounds() -> Rectangle {
    Rectangle::new(Point::ORIGIN, Size::new(KEYBOARD_W + GRID_W, 400.0))
}

/// Canvas point of section-relative `tick` on the `PITCH` row.
fn at(section_tick: u64) -> Point {
    let x = KEYBOARD_W + section_tick as f32 * GRID_W / (4 * BAR) as f32;
    let y = TOOLBAR_H + (127 - PITCH) as f32 * ZOOM_Y - SCROLL_Y + ZOOM_Y / 2.0;
    Point::new(x + 1.0, y)
}

fn send(
    c: &ExpandedEditorCanvas<'_>,
    state: &mut <ExpandedEditorCanvas<'_> as Program<Message>>::State,
    event: mouse::Event,
    pos: Point,
) -> Option<Message> {
    c.update(state, &iced::Event::Mouse(event), bounds(), mouse::Cursor::Available(pos))
        .and_then(|a| a.into_inner().0)
}

#[test]
fn a_click_adds_the_note_at_the_clip_relative_tick() {
    let tm = tempo_map();
    let clips = clip(Vec::new());
    let c = canvas(&clips, &tm);
    let mut state = Default::default();
    // Beat 3 of the section's first bar.
    let msg = send(&c, &mut state, mouse::Event::ButtonPressed(mouse::Button::Left), at(2 * TPQ))
        .expect("an empty-grid click adds a note");
    match msg {
        Message::MidiEditor(MidiEditorMessage::AddNote { clip_id, note, start_tick, .. }) => {
            assert_eq!((clip_id, note), (CLIP_ID, PITCH));
            assert_eq!(start_tick, SECTION_BAR as u64 * BAR + 2 * TPQ, "clip-relative tick");
        }
        other => panic!("expected AddNote, got {other:?}"),
    }
}

#[test]
fn an_offset_note_is_hit_where_it_is_drawn_and_moves_in_clip_ticks() {
    let tm = tempo_map();
    // A note on beat 2 of the section's first bar, in clip ticks.
    let note_tick = SECTION_BAR as u64 * BAR + TPQ;
    let clips = clip(vec![MidiNote {
        note: PITCH,
        velocity: 0.8,
        start_tick: note_tick,
        duration_ticks: 2 * TPQ,
    }]);
    let c = canvas(&clips, &tm);
    let mut state = Default::default();

    // Pressing on the note starts a drag instead of adding a note.
    let press = send(&c, &mut state, mouse::Event::ButtonPressed(mouse::Button::Left), at(TPQ));
    assert!(press.is_none(), "the press should grab the drawn note, got {press:?}");

    // Drag one beat to the right.
    let moved = send(
        &c,
        &mut state,
        mouse::Event::CursorMoved { position: at(2 * TPQ) },
        at(2 * TPQ),
    )
    .expect("dragging the note moves it");
    match moved {
        Message::MidiEditor(MidiEditorMessage::MoveNote { clip_id, new_start_tick, .. }) => {
            assert_eq!(clip_id, CLIP_ID);
            assert_eq!(new_start_tick, note_tick + TPQ, "moved in clip ticks");
        }
        other => panic!("expected MoveNote, got {other:?}"),
    }
}
