//! Vocal-roll shortcuts must not fire while the user types into a text
//! field on the same page (code review VIEW-09).
//!
//! The vocal roll acts on Delete/Backspace (remove the selected note) and
//! on the text `s`/`+` (toggle a slur). iced hands every key press to every
//! widget, so before the fix typing "sun" into a right-rail lyric field
//! toggled a slur on the selected note, and Backspace deleted it. The
//! canvas now owns the keyboard only while it was the target of the latest
//! mouse press (`resonance_app::focus::KeyFocus`).

use iced::keyboard::key::Named;
use iced::Size;
use iced_test::simulator::{self, Simulator};
use resonance_app::compose::{ComposeMessage, SelectedLane};
use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

const WINDOW: (f32, f32) = (1440.0, 900.0);
/// Demo Lead Vocal track and its derived melody clip.
const LEAD_VOCAL: u64 = 6;
const VOCAL_CLIP: u64 = 16;
/// `VocalParams::default().theme`, the value the demo's theme field shows.
const THEME: &str = "A house made of glass \u{2014} fragile loves, the stones we can't take back.";

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

/// Compose tab with the Lead Vocal lane selected (its inspector, with the
/// lyric fields, fills the right rail), the vocal clip open in the vocal
/// roll, and its first note selected.
fn app_editing_vocal() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    app.test_dispatch(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(LEAD_VOCAL),
    )));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(VOCAL_CLIP)));
    app.test_dispatch(Message::MidiEditor(MidiEditorMessage::SelectNote {
        note_index: Some(0),
    }));
    app
}

fn is_note_edit(m: &Message) -> bool {
    matches!(
        m,
        Message::MidiEditor(
            MidiEditorMessage::ToggleSlur { .. } | MidiEditorMessage::RemoveNote { .. }
        )
    )
}

#[test]
fn typing_into_a_lyric_field_leaves_the_selected_note_alone() {
    let app = app_editing_vocal();
    // Tall enough that the right rail's Lyrics group clears the editor
    // panel docked at the bottom.
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, 1800.0), app.view());
    // The Lyrics group's "Theme / prompt" field, found by its (default) value.
    ui.click(THEME).expect("the vocal inspector's theme field");
    ui.typewrite("sun+");
    ui.tap_key(Named::Backspace);
    let msgs: Vec<Message> = ui.into_messages().collect();
    assert!(
        msgs.iter().any(|m| matches!(m, Message::Compose(ComposeMessage::LaneInspector { .. }))),
        "the click must have focused a vocal-inspector text field: {msgs:?}"
    );
    assert!(
        !msgs.iter().any(is_note_edit),
        "typing into a text field must not slur or delete the selected note: {msgs:?}"
    );
}

#[test]
fn shortcuts_still_work_after_clicking_the_vocal_roll() {
    let app = app_editing_vocal();
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    // A press on the vocal roll's keyboard strip (bottom-left of the
    // editor panel) claims the keys without editing a note.
    ui.point_at(iced::Point::new(20.0, 820.0));
    ui.simulate(simulator::click());
    ui.typewrite("s");
    ui.tap_key(Named::Delete);
    let msgs: Vec<Message> = ui.into_messages().collect();
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Message::MidiEditor(MidiEditorMessage::ToggleSlur { clip_id: VOCAL_CLIP, note_index: 0 })
        )),
        "`s` over an owned vocal roll toggles the slur: {msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| matches!(
            m,
            Message::MidiEditor(MidiEditorMessage::RemoveNote { clip_id: VOCAL_CLIP, note_index: 0 })
        )),
        "Delete over an owned vocal roll removes the selected note: {msgs:?}"
    );
}
