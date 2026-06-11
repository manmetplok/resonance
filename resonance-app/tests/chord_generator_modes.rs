//! Golden-image snapshots for the **Compose chord-lane inspector's
//! generator-mode wiring** (`ChordInspectorMsg::SetGeneratorKind` and
//! friends).
//!
//! Covered states:
//!
//! 1. **Markov (default)** — the new GENERATOR dropdown sits at the top
//!    of the Chord generator card showing "Style table", with the
//!    existing STYLE / CHORDS / BEATS-PER-CHORD / START° / END° controls
//!    below it.
//! 2. **Schema mode** — switching the dropdown to "Schema" swaps the
//!    mode block for SCHEMA / CHORDS / BEATS-PER-CHORD / ROTATION /
//!    SUBSTITUTION. Uses Circle of Fifths (8-chord loop) so the
//!    rotation preview labels are elided with `…` and must still fit
//!    the 324px rail; the substitution value is echoed in its label.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::messages::{ChordInspectorMsg, GeneratorKind};
use resonance_app::compose::{ComposeMessage, SelectedLane};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_music_theory::SchemaKind;

/// Tall window (same convention as `compose_drum_pattern_picker.rs`) so
/// the whole Chord generator card sits inside the viewport instead of
/// behind the rail's scrollbar.
const TALL_WINDOW: (f32, f32) = (1440.0, 1600.0);

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

/// Build the demo app pinned to the Compose tab with the chords lane
/// selected; returns the app plus the first section's definition id.
fn build_app() -> (Resonance, u64) {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Chords,
    )));
    let def_id = app.compose_state().definitions[0].id;
    (app, def_id)
}

fn send(app: &mut Resonance, definition_id: u64, msg: ChordInspectorMsg) {
    let _ = app.update(Message::Compose(ComposeMessage::ChordInspector {
        definition_id,
        msg,
    }));
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(
        sim_settings(),
        Size::new(TALL_WINDOW.0, TALL_WINDOW.1),
        app.view(),
    )
}

fn snapshot(app: &Resonance, path: &str) {
    let mut ui = simulator(app);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

/// Default (Markov) state: the GENERATOR dropdown reads "Style table"
/// and sits above the STYLE table picker; the Markov-only START° / END°
/// constraints are present.
///
/// Note: pick_list selected values ("Style table", schema names,
/// rotation previews) are drawn by the widget itself, not as child
/// `text` nodes, so `find` can't see them — the goldens carry that
/// coverage.
#[test]
fn chord_rail_markov_shows_generator_dropdown_above_style() {
    let (app, _def_id) = build_app();

    let mut ui = simulator(&app);
    ui.find("GENERATOR")
        .expect("GENERATOR label should be on the chord generator card");
    ui.find("STYLE")
        .expect("Markov mode keeps the STYLE table picker");
    ui.find("START °")
        .expect("Markov mode keeps the START ° degree constraint");
    ui.find("END °")
        .expect("Markov mode keeps the END ° degree constraint");

    snapshot(
        &app,
        "tests/snapshots/compose_chord_rail_generator_markov.png",
    );
}

/// Schema mode with Circle of Fifths (8-chord loop): SCHEMA / ROTATION /
/// SUBSTITUTION replace the Markov controls, the selected rotation's
/// preview label is elided with `…`, and the substitution value is
/// echoed in its label.
#[test]
fn chord_rail_schema_circle_of_fifths_with_elided_rotation() {
    let (mut app, def_id) = build_app();

    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetGeneratorKind(GeneratorKind::Schema),
    );
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetSchemaKind(SchemaKind::CircleOfFifths),
    );
    send(&mut app, def_id, ChordInspectorMsg::SetSchemaRotation(2));
    send(
        &mut app,
        def_id,
        ChordInspectorMsg::SetSchemaSubstitution(0.35),
    );

    let mut ui = simulator(&app);
    ui.find("SCHEMA")
        .expect("Schema mode shows the SCHEMA picker label");
    ui.find("ROTATION")
        .expect("Schema mode shows the ROTATION picker label");
    ui.find("SUBSTITUTION · 0.35")
        .expect("SUBSTITUTION label echoes the slider value");
    // Markov-only controls must be gone.
    assert!(
        ui.find("START °").is_err(),
        "START ° is a Markov-only control and must not render in Schema mode"
    );
    assert!(
        ui.find("END °").is_err(),
        "END ° is a Markov-only control and must not render in Schema mode"
    );

    snapshot(
        &app,
        "tests/snapshots/compose_chord_rail_generator_schema.png",
    );
}
