//! A timeline selection made outside the timeline canvas grants it the
//! keyboard (code review FU-C2).
//!
//! Delete only acts while the timeline owns the keys (`focus::KeyFocus`,
//! VIEW-01), and ownership used to move only on a mouse press — so a clip
//! selected without a press on the canvas was not deletable until the user
//! clicked the timeline. A GUI selection change that targets the timeline
//! now grants it the keys; a later press elsewhere still takes them away,
//! and a control-API call that selects internally does not grant them.

use iced::keyboard::{self, key::Named, Key, Modifiers};
use resonance_app::message::{Message, MidiClipMessage};
use resonance_app::state::{MidiClipState, ViewMode};
use resonance_app::view::timeline::TimelineState;
use resonance_app::Resonance;

const CLIP: u64 = 5;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_push_midi_clip(MidiClipState {
        id: CLIP,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 1920,
        name: "Clip".to_owned(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app
}

fn delete_key() -> iced::Event {
    iced::Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Named(Named::Delete),
        modified_key: Key::Named(Named::Delete),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Delete),
        location: keyboard::Location::Standard,
        modifiers: Modifiers::empty(),
        text: None,
        repeat: false,
    })
}

fn press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}

/// The canvas has been on screen, and the last press landed elsewhere.
/// Its first events also publish its viewport size; drain those so the
/// Delete below is the only thing that can publish.
fn unfocused_timeline(app: &Resonance) -> TimelineState {
    let mut state = TimelineState::default();
    let _ = app.test_timeline_canvas_event(&mut state, &press(), -50.0, -50.0);
    let moved = iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: iced::Point::new(-50.0, -50.0),
    });
    for _ in 0..8 {
        if app
            .test_timeline_canvas_event(&mut state, &moved, -50.0, -50.0)
            .is_none()
        {
            break;
        }
    }
    state
}

/// Select the clip without a press on the canvas.
fn select_clip_elsewhere(app: &mut Resonance) {
    let _ = app.update(Message::MidiClip(MidiClipMessage::StartMidiClipDrag {
        clip_id: CLIP,
        grab_offset_x: 0.0,
        start_x: 0.0,
        start_y: 0.0,
    }));
    let _ = app.update(Message::MidiClip(MidiClipMessage::EndMidiClipDrag));
}

fn is_delete_clip(m: &Option<Message>) -> bool {
    matches!(m, Some(Message::MidiClip(MidiClipMessage::DeleteMidiClip(CLIP))))
}

#[test]
fn a_selection_made_off_canvas_is_deletable_without_a_click() {
    let mut app = app();
    let mut state = unfocused_timeline(&app);
    select_clip_elsewhere(&mut app);
    let out = app.test_timeline_canvas_event(&mut state, &delete_key(), -50.0, -50.0);
    assert!(is_delete_clip(&out), "Delete acts on the new selection: {out:?}");
}

#[test]
fn a_press_elsewhere_after_the_grant_takes_the_keys_back() {
    let mut app = app();
    let mut state = unfocused_timeline(&app);
    select_clip_elsewhere(&mut app);
    let _ = app.test_timeline_canvas_event(&mut state, &press(), -50.0, -50.0);
    let out = app.test_timeline_canvas_event(&mut state, &delete_key(), -50.0, -50.0);
    assert!(out.is_none(), "a later press elsewhere releases the keys: {out:?}");
}

#[test]
fn an_unchanged_selection_grants_nothing() {
    let mut app = app();
    select_clip_elsewhere(&mut app);
    // The canvas first appears after the selection: it adopts the current
    // grant without taking the keys (no selection change happened since).
    let mut state = unfocused_timeline(&app);
    let out = app.test_timeline_canvas_event(&mut state, &delete_key(), -50.0, -50.0);
    assert!(out.is_none(), "an old selection does not steal the keys: {out:?}");
}

#[test]
fn a_control_call_that_selects_internally_grants_nothing() {
    // `global.edit_tempo_event` aims its edit through the GUI's tempo-drag
    // messages, which select the event. A remote client must not aim the
    // user's next Delete/Backspace at it.
    let mut app = app();
    app.test_set_project_path(std::path::PathBuf::from("/tmp/timeline-key-grant.rprj"));
    let mut state = unfocused_timeline(&app);
    let add = crate::common::call(
        &mut app,
        "global.add_tempo_event",
        serde_json::json!({"bar": 5, "bpm": 100.0}),
    );
    assert!(add.error.is_none(), "{:?}", add.error);
    let edit = crate::common::call(
        &mut app,
        "global.edit_tempo_event",
        serde_json::json!({"bar": 5, "bpm": 110.0}),
    );
    assert!(edit.error.is_none(), "{:?}", edit.error);
    let out = app.test_timeline_canvas_event(&mut state, &delete_key(), -50.0, -50.0);
    assert!(out.is_none(), "a control call stole the keys: {out:?}");
}
