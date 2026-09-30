//! Dragging a note past a neighbour keeps moving the dragged note (code
//! review VIEW-02).
//!
//! Every move re-sorts the clip by start tick — in the engine and in the
//! app's echo mirror alike — so once note A is dragged past note B, the
//! index A was pressed at names B. The piano roll used to keep sending
//! `MoveNote` with that pressed index, so the rest of the drag moved B.
//!
//! Driven through the real `canvas::Program::update` of the piano roll
//! with synthetic mouse events; the published moves are then replayed as
//! the engine's `MidiNoteMoved` echoes into the app, exactly as they
//! would arrive, and the resulting notes and selection are asserted.

use std::collections::BTreeSet;

use iced::widget::canvas::Program;
use iced::{mouse, Point, Rectangle};
use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::view::midi_editor::{PianoRollCanvas, PianoRollState, QuantizePreview};
use resonance_app::view::piano_roll::{note_rect, PianoRollLayout, PianoRollViewport};
use resonance_app::Resonance;
use resonance_audio::quantize::{Division, GridValue, QuantizeMode};
use resonance_audio::types::{AudioEvent, MidiNote, TempoMap, TrackType};

const TRACK: u64 = 1;
const CLIP: u64 = 100;
const KEYBOARD_W: f32 = 50.0;
/// 128 rows of 10 px plus the 40 px velocity lane.
const BOUNDS: Rectangle = Rectangle { x: 0.0, y: 0.0, width: 4000.0, height: 1320.0 };

fn note(pitch: u8, start: u64) -> MidiNote {
    MidiNote { note: pitch, velocity: 0.8, start_tick: start, duration_ticks: 100 }
}

/// A = C4 at tick 0 (index 0), B = D4 at tick 480 (index 1).
fn clip() -> MidiClipState {
    MidiClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_ticks: 1920,
        name: "clip".to_owned(),
        notes: vec![note(60, 0), note(62, 480)].into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

/// Piano roll over `clip` at 1 px per tick, 10 px per semitone, no snap.
fn canvas<'a>(
    clip: &'a MidiClipState,
    selected: &'a BTreeSet<usize>,
    tempo: &'a TempoMap,
) -> PianoRollCanvas<'a> {
    PianoRollCanvas {
        keys_blocked: false,
        clip,
        track_id: TRACK,
        scroll_x: 0.0,
        scroll_y: 0.0,
        zoom_x: 1.0,
        zoom_y: 10.0,
        snap_ticks: 0,
        selected_notes: selected,
        time_sig_num: 4,
        quantize: QuantizePreview {
            division: Division::straight(GridValue::Sixteenth),
            strength: 1.0,
            swing: 0.0,
            mode: QuantizeMode::StartOnly,
            quantize_ends: false,
            iterative: false,
        },
        tempo_map: tempo,
    }
}

fn published(
    canvas: &PianoRollCanvas<'_>,
    state: &mut PianoRollState,
    event: iced::Event,
    at: Point,
) -> Option<Message> {
    let action = canvas.update(state, &event, BOUNDS, mouse::Cursor::Available(at))?;
    action.into_inner().0
}

#[test]
fn dragging_a_note_past_its_neighbour_moves_only_the_dragged_note() {
    let clip = clip();
    let selected = BTreeSet::new();
    let tempo = TempoMap::default();
    let canvas = canvas(&clip, &selected, &tempo);
    let mut state = PianoRollState::default();

    // Grab A in the middle of its body.
    let layout = PianoRollLayout { keyboard_w: KEYBOARD_W, grid_top: 0.0, grid_h: BOUNDS.height - 40.0 };
    let viewport = PianoRollViewport { zoom_x: 1.0, zoom_y: 10.0, scroll_x: 0.0, scroll_y: 0.0 };
    let a = note_rect(&layout, &viewport, &clip.notes[0]);
    let grab = Point::new(a.x + 50.0, a.y + 5.0);
    let press = published(
        &canvas,
        &mut state,
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        grab,
    );
    assert!(matches!(
        press,
        Some(Message::MidiEditor(MidiEditorMessage::SelectNote { note_index: Some(0) }))
    ));

    // Drag right past B (tick 480) in steps, on A's row. The view is
    // not rebuilt mid-drag, as with a real drag outrunning the echoes.
    let moves: Vec<MidiEditorMessage> = [300.0, 600.0, 700.0]
        .iter()
        .map(|&tick| {
            let at = Point::new(grab.x + tick, grab.y);
            match published(
                &canvas,
                &mut state,
                iced::Event::Mouse(mouse::Event::CursorMoved { position: at }),
                at,
            ) {
                Some(Message::MidiEditor(m @ MidiEditorMessage::MoveNote { .. })) => m,
                other => panic!("expected a MoveNote, got {other:?}"),
            }
        })
        .collect();
    let indices: Vec<usize> = moves
        .iter()
        .map(|m| match m {
            MidiEditorMessage::MoveNote { note_index, .. } => *note_index,
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(indices, vec![0, 0, 1], "after crossing B, A is index 1");

    // Replay: the press's selection, then each move's engine echo.
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_add_track(TRACK, TrackType::Instrument);
    app.test_push_midi_clip(clip.clone());
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(CLIP)));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SelectNote { note_index: Some(0) }));
    for m in moves {
        let MidiEditorMessage::MoveNote { clip_id, note_index, new_start_tick, new_note } = m else {
            unreachable!()
        };
        app.test_apply_engine_event(AudioEvent::MidiNoteMoved {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        });
    }
    let notes: Vec<(u8, u64)> = app
        .test_midi_clips()
        .iter()
        .find(|c| c.id == CLIP)
        .unwrap()
        .notes
        .iter()
        .map(|n| (n.note, n.start_tick))
        .collect();
    assert_eq!(notes, vec![(62, 480), (60, 700)], "B untouched, A at the drop point");
    assert_eq!(
        app.test_editing_selected_notes(),
        Some(BTreeSet::from([1])),
        "the selection follows A to its new index"
    );
}
