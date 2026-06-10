//! Golden-image snapshots for the **Compose right-rail collapsible
//! panel cards** (`ComposeMessage::ToggleRailPanel`, keyed by
//! `RailPanelKey`).
//!
//! Covered states:
//!
//! 1. **chord rail** — Scale + Chord generator cards folded while the
//!    chords lane is selected.
//! 2. **drum rail, per-group keying** — folding the Kick group's meter
//!    card must not fold the Snare group's: the collapse set is keyed
//!    by `RailPanelKey::DrumMeter(group_id)`, never by list position.
//!    Snapshot A locks the folded Kick meter; snapshot B switches the
//!    group selector to Snare and locks its *open* meter card with the
//!    Kick fold still in the set.
//! 3. **vocal rail** — Lyrics + Lyric draft cards folded. The rhyme /
//!    line-count meta ("ABAB · 4 LINES" style) rides the draft card's
//!    header row, so it must remain findable while the card is folded —
//!    asserted via a text selector, not just pixels.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::messages::DrumGroupsMessage;
use resonance_app::compose::{
    ComposeMessage, LaneGeneratorKind, RailPanelKey, SelectedLane,
};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::TrackId;

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);
/// Taller window used for the drum-rail snapshots so the group
/// selector, meter, articulation, and rhythm cards all sit inside the
/// viewport — same convention as `compose_drum_pattern_picker.rs`.
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

/// Build the demo app pinned to the Compose tab. The demo seed lands
/// on the Lead Vocal lane; tests below re-select the lane they need.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app
}

fn fold(app: &mut Resonance, key: RailPanelKey) {
    let _ = app.update(Message::Compose(ComposeMessage::ToggleRailPanel(key)));
}

fn simulator(app: &Resonance, window: (f32, f32)) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(window.0, window.1), app.view())
}

fn snapshot_to_window(app: &Resonance, path: &str, window: (f32, f32)) {
    let mut ui = simulator(app, window);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

/// Chord rail with the Scale and Chord generator cards folded — only
/// their header rows (▸ caret) remain; the Section motif card below
/// stays open.
#[test]
fn chord_rail_scale_and_generator_folded() {
    let mut app = build_app();
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Chords,
    )));
    fold(&mut app, RailPanelKey::Scale);
    fold(&mut app, RailPanelKey::ChordGenerator);
    snapshot_to_window(
        &app,
        "tests/snapshots/compose_chord_rail_folded.png",
        WINDOW,
    );
}

/// Resolve the demo's drum track id plus the first two drum-group ids
/// (Kick, Snare) of the section's assigned pattern.
fn drum_fixture(app: &Resonance) -> (TrackId, u64, u64) {
    let drum_track_id = app
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
        .expect("demo seeds a drum track");

    let compose = app.compose_state();
    let definition_id = compose
        .selected_placement()
        .expect("demo selects a placement")
        .definition_id;
    let definition = compose
        .definitions
        .iter()
        .find(|d| d.id == definition_id)
        .expect("placement points at a definition");
    let groups = compose.groups_for_definition(definition);
    assert!(
        groups.len() >= 2,
        "demo pattern needs two drum groups for the per-group keying test"
    );
    (drum_track_id, groups[0].id, groups[1].id)
}

/// Folding the first group's meter card must leave the second group's
/// meter open — collapse state is keyed by drum-group id.
#[test]
fn drum_rail_meter_fold_is_per_group() {
    let mut app = build_app();
    let (drum_track_id, kick_group_id, snare_group_id) = drum_fixture(&app);

    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Drums(drum_track_id),
    )));

    // Fold the selected (first) group's meter card.
    fold(&mut app, RailPanelKey::DrumMeter(kick_group_id));
    snapshot_to_window(
        &app,
        "tests/snapshots/compose_drum_rail_kick_meter_folded.png",
        TALL_WINDOW,
    );

    // Switch the rail to the second group: its meter card must render
    // open even though the first group's fold is still in the set.
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::SelectGroup {
            group_id: snare_group_id,
        },
    )));
    snapshot_to_window(
        &app,
        "tests/snapshots/compose_drum_rail_snare_meter_open.png",
        TALL_WINDOW,
    );
}

/// Vocal rail with the Lyrics and Lyric draft cards folded. The draft
/// card's "RHYME · N LINES" meta lives on the header row so it stays
/// visible while folded — asserted via text selector on the rendered
/// tree, plus a golden for the pixels.
#[test]
fn vocal_rail_lyrics_and_draft_folded() {
    let mut app = build_app();

    let vocal_track_id = app
        .track_registry()
        .tracks
        .iter()
        .find(|t| {
            matches!(t.track_type, resonance_audio::types::TrackType::Vocal)
                && t.sub_track.is_none()
        })
        .map(|t| t.id)
        .expect("demo seeds a vocal track");

    // Re-select through the message path (the demo already lands here,
    // but this keeps the test honest if the seed's default changes).
    let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
        SelectedLane::Instrument(vocal_track_id),
    )));

    // Compute the expected header meta from state before folding.
    let meta = {
        let compose = app.compose_state();
        let definition_id = compose
            .selected_placement()
            .expect("demo selects a placement")
            .definition_id;
        let definition = compose
            .definitions
            .iter()
            .find(|d| d.id == definition_id)
            .expect("placement points at a definition");
        let config = definition
            .lane_generators
            .get(&vocal_track_id)
            .expect("vocal lane has a generator config");
        let LaneGeneratorKind::Vocal(params) = &config.kind else {
            panic!("vocal track's lane generator should be Vocal");
        };
        format!("{} · {} LINES", params.rhyme.as_str(), params.draft.len())
    };

    fold(&mut app, RailPanelKey::VocalLyrics(vocal_track_id));
    fold(&mut app, RailPanelKey::VocalDraft(vocal_track_id));

    // The folded draft header must still carry the meta text.
    let mut ui = simulator(&app, TALL_WINDOW);
    ui.find(meta.as_str())
        .unwrap_or_else(|_| panic!("folded Lyric draft header lost its meta: {meta:?}"));

    snapshot_to_window(
        &app,
        "tests/snapshots/compose_vocal_rail_lyrics_draft_folded.png",
        TALL_WINDOW,
    );
}
