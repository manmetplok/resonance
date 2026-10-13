//! The GUI side of BUG-70 (vocal-melody-on-chord-change): a chord edit
//! that would rewrite a vocal lane's melody asks what to do with it. The
//! edit lands with the lane kept; Apply carries out the rows' choices
//! inside the same undo entry, Cancel undoes the edit, "Remember for this
//! project" skips the question afterwards, and a chord drag asks once at
//! its end. Goldens for the question, the lane badge and the vocal-roll
//! chip.

use crate::common;

use iced::widget::canvas::Program;
use iced::{mouse, Point, Rectangle, Size};
use iced_test::simulator::Simulator;
use resonance_app::compose::vocal_melody::VocalMelodyMsg;
use resonance_app::compose::{
    ComposeMessage, RailPanelKey, SelectedLane, VocalMelodyChoice,
};
use resonance_app::message::{Message, MidiEditorMessage};
use resonance_app::state::overlay::Overlay;
use resonance_app::state::ViewMode;
use resonance_app::view::compose::chord_lane::{ChordLaneCanvas, ChordLaneState};
use resonance_app::view::compose::tracks::NAME_COLUMN_WIDTH;
use resonance_app::{demo, theme, Resonance};

const WINDOW: (f32, f32) = (1440.0, 900.0);
/// Demo Lead Vocal track and its derived melody clip.
const LEAD_VOCAL: u64 = 6;
const VOCAL_CLIP: u64 = 16;

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

fn snapshot(app: &Resonance, path: &str, expect_text: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    assert!(ui.find(expect_text).is_ok(), "the view reads {expect_text:?}");
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

fn send(app: &mut Resonance, m: VocalMelodyMsg) {
    let _ = app.update(Message::Compose(ComposeMessage::VocalMelody(m)));
}

/// The demo on the Compose tab, saved somewhere (so edits record undo
/// entries), with the Lead Vocal lane selected and its draft, melody and
/// voice cards folded (the badge sits at the top of the rail).
fn demo_app() -> (Resonance, u64) {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/vocal-melody-prompt.rprj"));
    app.test_install_shipped_voicebanks();
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(LEAD_VOCAL),
    )));
    for key in [
        RailPanelKey::VocalDraft(LEAD_VOCAL),
        RailPanelKey::VocalMelody(LEAD_VOCAL),
        RailPanelKey::VocalLyrics(LEAD_VOCAL),
    ] {
        let _ = app.update(Message::Compose(ComposeMessage::ToggleRailPanel(key)));
    }
    let def = app
        .compose_state()
        .selected_placement()
        .expect("demo selects a placement")
        .definition_id;
    assert!(app.compose_state().vocal_melody.prompt.is_none());
    (app, def)
}

fn lead_notes(app: &Resonance) -> Vec<(u8, u64, u64)> {
    app.test_midi_clips()
        .iter()
        .find(|c| c.id == VOCAL_CLIP)
        .or_else(|| app.test_midi_clips().iter().find(|c| c.track_id == LEAD_VOCAL))
        .expect("the lead vocal has a clip")
        .notes
        .iter()
        .map(|n| (n.note, n.start_tick, n.duration_ticks))
        .collect()
}

fn chords(app: &Resonance, def: u64) -> Vec<String> {
    app.compose_state()
        .find_definition(def)
        .unwrap()
        .chords
        .iter()
        .map(|c| format!("{}@{}+{}", c.chord, c.start_beat, c.duration_beats))
        .collect()
}

fn undo_len(app: &Resonance) -> usize {
    app.test_undo_history().undo_len()
}

/// Transpose the section's first chord up a fourth from the GUI.
fn edit_first_chord(app: &mut Resonance, def: u64) {
    let first = app.compose_state().find_definition(def).unwrap().chords[0].clone();
    let mut chord = first.chord;
    chord.root = chord.root.transpose(5);
    let _ = app.update(Message::Compose(ComposeMessage::EditChord {
        definition_id: def,
        chord_id: first.id,
        chord,
    }));
}

fn lane_flags(app: &Resonance, def: u64) -> resonance_app::compose::LaneMelodyFlags {
    app.compose_state().find_definition(def).unwrap().lane_generators[&LEAD_VOCAL].melody
}

#[test]
fn a_gui_chord_edit_asks_and_keeps_the_melody_until_answered() {
    let (mut app, def) = demo_app();
    let notes = lead_notes(&app);
    let entries = undo_len(&app);
    edit_first_chord(&mut app, def);

    let prompt = app.compose_state().vocal_melody.prompt.as_ref().expect("the edit asks");
    assert_eq!(prompt.rows.len(), 1);
    let row = &prompt.rows[0];
    assert_eq!((row.definition_id, row.track_id), (def, LEAD_VOCAL));
    assert!(!row.hand_edited);
    assert_eq!(row.choice, VocalMelodyChoice::Regenerate, "untouched lanes default to Regenerate");
    assert_eq!(app.root_overlay(), Some(Overlay::VocalMelodyPrompt));
    assert_eq!(lead_notes(&app), notes, "nothing rewritten before the answer");
    assert_eq!(undo_len(&app), entries + 1, "the chord edit recorded its entry");
}

#[test]
fn apply_and_the_chord_edit_are_one_undo_entry() {
    let (mut app, def) = demo_app();
    let notes = lead_notes(&app);
    let chords_before = chords(&app, def);
    edit_first_chord(&mut app, def);
    let entries = undo_len(&app);
    let chords_after = chords(&app, def);

    send(&mut app, VocalMelodyMsg::Apply);
    assert!(app.compose_state().vocal_melody.prompt.is_none());
    let regenerated = lead_notes(&app);
    assert_ne!(regenerated, notes, "Regenerate re-derived the melody");
    assert_eq!(undo_len(&app), entries, "Apply adds no entry of its own");

    let _ = app.update(Message::Undo);
    assert_eq!(chords(&app, def), chords_before, "one undo restores the chords");
    assert_eq!(lead_notes(&app), notes, "and the melody");
    let _ = app.update(Message::Redo);
    assert_eq!(chords(&app, def), chords_after);
    assert_eq!(lead_notes(&app), regenerated, "redo brings both back");
}

#[test]
fn cancel_undoes_the_chord_edit() {
    let (mut app, def) = demo_app();
    let notes = lead_notes(&app);
    let chords_before = chords(&app, def);
    let entries = undo_len(&app);
    edit_first_chord(&mut app, def);

    // Esc sends what Cancel sends.
    let esc = Overlay::VocalMelodyPrompt.dismiss_message(&app).expect("Esc dismisses");
    let _ = app.update(esc);
    assert!(app.compose_state().vocal_melody.prompt.is_none());
    assert_eq!(chords(&app, def), chords_before, "the old chords are back");
    assert_eq!(lead_notes(&app), notes);
    assert_eq!(undo_len(&app), entries);
    assert!(!app.test_undo_history().can_redo(), "nothing to redo: the edit is gone");
    assert!(!lane_flags(&app, def).melody_predates_chords);
}

#[test]
fn remember_skips_the_question_until_settings_asks_again() {
    let (mut app, def) = demo_app();
    edit_first_chord(&mut app, def);
    send(&mut app, VocalMelodyMsg::SetAllChoices(VocalMelodyChoice::Keep));
    send(&mut app, VocalMelodyMsg::ToggleRemember);
    send(&mut app, VocalMelodyMsg::Apply);
    assert_eq!(app.compose_state().vocal_melody.remembered, Some(VocalMelodyChoice::Keep));
    assert!(lane_flags(&app, def).melody_predates_chords);

    let notes = lead_notes(&app);
    edit_first_chord(&mut app, def);
    assert!(app.compose_state().vocal_melody.prompt.is_none(), "remembered: no question");
    assert_eq!(lead_notes(&app), notes, "the remembered Keep applied");

    send(&mut app, VocalMelodyMsg::ForgetRememberedChoice);
    assert_eq!(app.compose_state().vocal_melody.remembered, None);
    edit_first_chord(&mut app, def);
    assert!(app.compose_state().vocal_melody.prompt.is_some(), "asked again");
}

/// Remember needs one choice for every row; with one row it is always
/// available, and changing the row keeps it.
#[test]
fn a_hand_edited_lane_defaults_to_keep() {
    let (mut app, def) = demo_app();
    let first = lead_notes(&app)[0];
    let _ = app.update(Message::MidiEditor(MidiEditorMessage::MoveNote {
        clip_id: VOCAL_CLIP,
        note_index: 0,
        new_start_tick: first.1,
        new_note: first.0 + 1,
    }));
    assert!(lane_flags(&app, def).hand_edited, "a vocal-roll edit marks the lane");
    edit_first_chord(&mut app, def);
    let prompt = app.compose_state().vocal_melody.prompt.as_ref().expect("asks");
    assert!(prompt.rows[0].hand_edited);
    assert_eq!(prompt.rows[0].choice, VocalMelodyChoice::Keep);
}

/// A chord drag publishes nothing while it moves and one edit at the
/// release: one question, one undo entry.
#[test]
fn a_chord_drag_asks_once_at_its_end() {
    let (mut app, def) = demo_app();
    let entries = undo_len(&app);
    let published: Vec<Message> = {
        let definition = app.compose_state().find_definition(def).unwrap();
        let first = definition.chords[0].clone();
        assert!(first.duration_beats >= 2, "the demo's first chord can shrink");
        let total_beats = resonance_app::view::compose::section_total_beats(
            app.test_tempo_map(),
            0,
            definition.length_bars,
        );
        let beat_px = 40.0;
        let bounds = Rectangle::new(
            Point::ORIGIN,
            Size::new(NAME_COLUMN_WIDTH + total_beats as f32 * beat_px, 64.0),
        );
        let canvas = ChordLaneCanvas {
            definition,
            tempo_map: app.test_tempo_map(),
            start_bar: 0,
            selected_chord_id: None,
            chords_selected: false,
            visible_x: resonance_app::view::compose::visible_x_window(None),
        };
        let at = |beat: f32| Point::new(NAME_COLUMN_WIDTH + beat * beat_px, 40.0);
        let end = (first.start_beat + first.duration_beats) as f32;
        let mut state = ChordLaneState::default();
        let mut feed = |event: mouse::Event, point: Point| -> Option<Message> {
            let action = canvas.update(
                &mut state,
                &iced::Event::Mouse(event),
                bounds,
                mouse::Cursor::Available(point),
            )?;
            action.into_inner().0
        };
        let mut out = Vec::new();
        // Grab the right edge, drag it in by a beat over several moves.
        out.extend(feed(mouse::Event::ButtonPressed(mouse::Button::Left), at(end - 0.05)));
        let moving = out.len();
        for beat in [end - 0.4, end - 0.9, end - 1.4] {
            out.extend(feed(mouse::Event::CursorMoved { position: at(beat) }, at(beat)));
        }
        assert_eq!(out.len(), moving, "moving publishes nothing");
        out.extend(feed(mouse::Event::ButtonReleased(mouse::Button::Left), at(end - 1.4)));
        out
    };
    let mut asked = 0;
    for message in published {
        let _ = app.update(message);
        if app.compose_state().vocal_melody.prompt.is_some() {
            asked += 1;
        }
    }
    assert_eq!(asked, 1, "asked once, at the release");
    assert_eq!(undo_len(&app), entries + 1, "one entry for the drag");
}

#[test]
fn vocal_melody_prompt_golden() {
    let (mut app, def) = demo_app();
    edit_first_chord(&mut app, def);
    snapshot(
        &app,
        "tests/snapshots/compose_vocal_melody_prompt.png",
        "Keep the vocal melody?",
    );
}

fn kept_lane() -> (Resonance, u64) {
    let (mut app, def) = demo_app();
    edit_first_chord(&mut app, def);
    send(&mut app, VocalMelodyMsg::SetAllChoices(VocalMelodyChoice::Keep));
    send(&mut app, VocalMelodyMsg::Apply);
    assert!(lane_flags(&app, def).melody_predates_chords);
    (app, def)
}

#[test]
fn vocal_rail_melody_predates_badge_golden() {
    let (app, _) = kept_lane();
    snapshot(
        &app,
        "tests/snapshots/compose_vocal_rail_melody_predates.png",
        "MELODY PREDATES CHORD CHANGE",
    );
}

#[test]
fn vocal_roll_melody_predates_chip_golden() {
    let (mut app, _) = kept_lane();
    let _ = app.update(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(VOCAL_CLIP)));
    snapshot(
        &app,
        "tests/snapshots/compose_vocal_roll_melody_predates.png",
        "MELODY PREDATES CHORD CHANGE",
    );
}

/// The badge's buttons: Re-fit clears the flag and keeps the rhythm;
/// Regenerate clears both flags.
#[test]
fn the_badge_buttons_refit_and_regenerate() {
    let (mut app, def) = kept_lane();
    let before = lead_notes(&app);
    send(&mut app, VocalMelodyMsg::Refit { definition_id: def, track_id: LEAD_VOCAL });
    assert!(!lane_flags(&app, def).melody_predates_chords);
    let refit = lead_notes(&app);
    assert_eq!(
        refit.iter().map(|n| (n.1, n.2)).collect::<Vec<_>>(),
        before.iter().map(|n| (n.1, n.2)).collect::<Vec<_>>(),
        "a re-fit keeps the rhythm"
    );

    let (mut app, def) = kept_lane();
    send(&mut app, VocalMelodyMsg::Regenerate { definition_id: def, track_id: LEAD_VOCAL });
    assert_eq!(lane_flags(&app, def), Default::default());
}
