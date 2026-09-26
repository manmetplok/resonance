//! Delete/Backspace must only act on the editor surface the user is
//! working in (code review VIEW-01).
//!
//! iced hands every `KeyPressed` to every widget, and the timeline canvas
//! sits above the piano roll in the same column. Before the fix, pressing
//! Delete with a note selected in the piano roll made the timeline publish
//! `DeleteMidiClip` for the clip being edited — ahead of the piano roll's
//! `RemoveSelectedNotes` — so the whole clip vanished. Each canvas now owns
//! the keyboard only while it was the target of the latest mouse press
//! (`resonance_app::focus::KeyFocus`).
//!
//! Driven through the real widget tree with `iced_test`, so the assertions
//! cover exactly what the canvases publish for a click + key tap.

use iced::keyboard::key::Named;
use iced::Size;
use iced_test::simulator::{self, Simulator};
use resonance_app::message::{Message, MidiClipMessage, MidiEditorMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

const WINDOW: (f32, f32) = (1440.0, 900.0);
/// Demo "Bm progression" clip on the Synth Bass track.
const CLIP: u64 = 12;

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

/// Demo session with clip [`CLIP`] selected on the timeline (the first
/// click of the double-click that opens it), open in the piano roll, and
/// its first note selected.
fn app_editing_clip() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    app.test_dispatch(Message::MidiClip(MidiClipMessage::StartMidiClipDrag {
        clip_id: CLIP,
        grab_offset_x: 0.0,
        start_x: 0.0,
        start_y: 0.0,
    }));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(CLIP)));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SelectNote {
        note_index: Some(0),
    }));
    app
}

/// Press (and release) the mouse at `at`, then tap Delete; return every
/// message the widget tree published.
fn click_then_delete(app: &Resonance, at: (f32, f32), key: Named) -> Vec<Message> {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.point_at(iced::Point::new(at.0, at.1));
    ui.simulate(simulator::click());
    ui.tap_key(key);
    ui.into_messages().collect()
}

fn is_delete_clip(m: &Message) -> bool {
    matches!(m, Message::MidiClip(MidiClipMessage::DeleteMidiClip(_)))
}

fn is_remove_notes(m: &Message) -> bool {
    matches!(
        m,
        Message::MidiEditor(MidiEditorMessage::RemoveSelectedNotes { clip_id: CLIP })
    )
}

/// A point on the piano roll's empty note grid (bottom editor panel).
const PIANO_ROLL_GRID: (f32, f32) = (700.0, 800.0);
/// A point on clip [`CLIP`] in the arrangement timeline.
const TIMELINE_CLIP: (f32, f32) = (700.0, 350.0);

#[test]
fn delete_in_piano_roll_removes_the_notes_not_the_clip() {
    let app = app_editing_clip();
    for key in [Named::Delete, Named::Backspace] {
        let msgs = click_then_delete(&app, PIANO_ROLL_GRID, key);
        assert!(
            msgs.iter().any(is_remove_notes),
            "{key:?} in the piano roll must remove the selected notes: {msgs:?}"
        );
        assert!(
            !msgs.iter().any(is_delete_clip),
            "{key:?} in the piano roll must not delete the clip being edited: {msgs:?}"
        );
    }
}

#[test]
fn delete_after_clicking_the_timeline_deletes_the_clip_not_the_notes() {
    let app = app_editing_clip();
    let msgs = click_then_delete(&app, TIMELINE_CLIP, Named::Delete);
    assert!(
        msgs.iter().any(is_delete_clip),
        "Delete after a timeline click still deletes the selected clip: {msgs:?}"
    );
    assert!(
        !msgs.iter().any(is_remove_notes),
        "the piano roll must not act on a key the timeline owns: {msgs:?}"
    );
}
