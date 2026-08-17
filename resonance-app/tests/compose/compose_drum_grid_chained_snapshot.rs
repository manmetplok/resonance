//! Golden-PNG snapshot of the drum grid lane rendering a **chained**
//! arrangement (todo #487).
//!
//! The test sets up a 2-entry arrangement on the focused section —
//! pattern P1 over the first half of the section and pattern P2 over
//! the second half — then captures the Compose view. The resulting
//! golden locks in:
//!
//! - **Pattern tints**: a faint per-pattern color overlay over each
//!   span's bar range.
//! - **Span separator**: the 1-px vertical line at the boundary between
//!   the two spans.
//! - **Correct cell data per bar**: cells in the first half come from
//!   P1's groups; cells in the second half come from P2's groups.
//!
//! On first run the golden PNG is written to `tests/snapshots/`;
//! subsequent runs diff against it via [`common::assert_golden`], which
//! honours `RESONANCE_SKIP_GOLDENS` in non-conformant environments.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;

use resonance_app::compose::messages::{ArrangementMessage, DrumGroupsMessage};
use resonance_app::compose::{ComposeMessage, EntryLength, SelectedLane};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

/// Taller window so the drum canvas fits below the synth tracks.
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

fn build_compose_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    demo::seed_demo_content(&mut app);

    // Focus the drum lane so the drum canvas renders in its active state.
    if let Some(drum_track_id) = app
        .track_registry()
        .tracks
        .iter()
        .find(|t| {
            use resonance_app::state::InstrumentType;
            use resonance_audio::types::TrackType;
            matches!(t.track_type, TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type == InstrumentType::Drum
        })
        .map(|t| t.id)
    {
        let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
            SelectedLane::Drums(drum_track_id),
        )));
    }
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(TALL_WINDOW.0, TALL_WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Lock in the drum grid rendering a chained arrangement.
///
/// P1 covers the first half of the section; P2 covers the second half.
/// The snapshot shows the tinted bar ranges and the 1-px separator at
/// the midpoint — verifying design item 3 visually.
#[test]
fn drum_grid_chained_arrangement_tints_and_separator() {
    let mut app = build_compose_app();

    let (def_id, section_bars, p1_id, p2_id) = {
        let compose = app.compose_state();
        let def_id = compose
            .selected_placement()
            .expect("demo seeds a selected placement")
            .definition_id;
        let def = compose.find_definition(def_id).expect("def exists");
        let section_bars = def.length_bars;
        let bank = &compose.drum_patterns;
        assert!(
            bank.len() >= 2,
            "demo must seed at least 2 drum patterns for this test"
        );
        (def_id, section_bars, bank[0].id, bank[1].id)
    };

    assert!(
        section_bars >= 2,
        "demo section must be at least 2 bars for a chained arrangement"
    );
    let half = section_bars / 2;

    // Assign P1 as the primary pattern for entry 0.
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def_id,
            pattern_id: Some(p1_id),
        },
    )));
    // Resize entry 0 to cover the first half.
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(
        ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index: 0,
            length: EntryLength::Bars(half),
        },
    )));
    // Add P2 for the second half.
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(
        ArrangementMessage::AddEntry {
            definition_id: def_id,
            pattern_id: p2_id,
        },
    )));
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(
        ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index: 1,
            length: EntryLength::Bars(section_bars - half),
        },
    )));

    snapshot_to(
        &app,
        "tests/snapshots/drum_grid_chained_arrangement_tints_separator.png",
    );
}
