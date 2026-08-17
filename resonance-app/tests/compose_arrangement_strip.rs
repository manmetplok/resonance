//! Golden-image snapshots for the Compose drum lane's ARRANGEMENT strip
//! (todo #488, design doc #170).
//!
//! Three states are locked in, driven purely through
//! [`ArrangementMessage`] edits on the focused section:
//!
//! 1. **exact + fill** — two chained entries covering the 8-bar section
//!    exactly, the second capped with a `▸ FILL` badge. Coverage pill is
//!    the green `8 / 8 bars`; no banner.
//! 2. **gap** — entries cover only 4 of 8 bars. Warm coverage pill
//!    (`4 / 8 · gap 4`) plus the non-destructive **Fill to end** banner.
//! 3. **overflow** — entries total 10 bars over an 8-bar section. Pink
//!    coverage pill (`10 / 8 · overflow 2`) plus the **Trim to fit** error
//!    banner.
//!
//! The pure label / stepper logic is unit-tested in
//! `compose_arrangement_strip_labels.rs`; this file locks in the rendered
//! strip. A tall window is used so the drum lane sits inside the viewport
//! below the synth lanes. On first run `matches_image()` writes the goldens
//! under `tests/snapshots/`; subsequent runs diff against the committed
//! PNGs.

mod common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::messages::ArrangementMessage;
use resonance_app::compose::{ComposeMessage, EntryLength, SelectedLane};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

/// Tall window so the picker/arrangement strip and the drum canvas sit
/// inside the viewport — 1440×900 is too short to fit all the synth lanes
/// plus the drum lane on one screen.
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

/// Build the demo app pinned to Compose with the drum lane focused.
fn build_compose_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new_for_test();
    demo::seed_demo_content(&mut app);

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

/// The focused section's definition id and the first two bank pattern ids.
fn ids(app: &Resonance) -> (u64, u64, u64) {
    let compose = app.compose_state();
    let definition_id = compose.selected_placement().unwrap().definition_id;
    let main = compose.drum_patterns.first().expect("a pattern").id;
    let b = compose
        .drum_patterns
        .get(1)
        .map(|p| p.id)
        .unwrap_or(main);
    (definition_id, main, b)
}

fn send(app: &mut Resonance, msg: ArrangementMessage) {
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(msg)));
}

fn snapshot(app: &Resonance, path: &str) {
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

#[test]
fn arrangement_strip_exact_with_fill() {
    let mut app = build_compose_app();
    let (def, main, b) = ids(&app);
    // Entry 0: Main ×4 (4 bars). Entry 1: B ×4 (4 bars) capped with a Main
    // fill on its last bar → 8/8 exact.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: main });
    send(&mut app, ArrangementMessage::SetEntryLength { definition_id: def, index: 0, length: EntryLength::RepeatN(4) });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: b });
    send(&mut app, ArrangementMessage::SetEntryLength { definition_id: def, index: 1, length: EntryLength::RepeatN(4) });
    send(&mut app, ArrangementMessage::SetEntryFill { definition_id: def, index: 1, fill: Some(main) });
    snapshot(&app, "tests/snapshots/arrangement_strip_exact_with_fill.png");
}

#[test]
fn arrangement_strip_gap() {
    let mut app = build_compose_app();
    let (def, main, _b) = ids(&app);
    // Only 4 of 8 bars covered → warm gap pill + "Fill to end" banner.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: main });
    send(&mut app, ArrangementMessage::SetEntryLength { definition_id: def, index: 0, length: EntryLength::RepeatN(4) });
    snapshot(&app, "tests/snapshots/arrangement_strip_gap.png");
}

#[test]
fn arrangement_strip_overflow() {
    let mut app = build_compose_app();
    let (def, main, _b) = ids(&app);
    // 6 + 4 = 10 bars over an 8-bar section → pink overflow pill + "Trim to
    // fit" banner.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: main });
    send(&mut app, ArrangementMessage::SetEntryLength { definition_id: def, index: 0, length: EntryLength::RepeatN(6) });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: main });
    send(&mut app, ArrangementMessage::SetEntryLength { definition_id: def, index: 1, length: EntryLength::RepeatN(4) });
    snapshot(&app, "tests/snapshots/arrangement_strip_overflow.png");
}
