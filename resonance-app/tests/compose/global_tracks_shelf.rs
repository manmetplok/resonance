//! Golden-image snapshots for the **Arrange-view global-tracks shelf**
//! — the collapsible strip that sits between the section-pill band
//! and the regular track lanes, holding the chord / tempo / signature
//! lanes plus the always-visible "GLOBAL · 6/8 · 90 BPM · …" summary
//! line.
//!
//! Two states are locked in here:
//!
//! 1. **collapsed** — only the 32 px shelf header strip is visible
//!    (caret + `GLOBAL` tag + count badge on the column side,
//!    summary text on the canvas side). Below the strip the regular
//!    tracks start immediately.
//! 2. **expanded** — the shelf header strip plus three lane rows
//!    (chord lane with section tabs + chord blocks, tempo automation
//!    curve, signature pill). Track lanes start at the very bottom
//!    of the shelf.
//!
//! Both snapshots are taken at scroll = 0 so the alignment between
//! the column-side labels (chord / tempo / signature) and the canvas
//! lanes is locked in. The companion
//! `track_header_alignment_scroll_*` suite covers the fractional
//! vertical-scroll variants; this file focuses on the shelf chrome
//! itself.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

/// Window size matches the app's default & minimum window per the
/// design guidelines.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// Build the iced simulator `Settings` with the same font registrations
/// the production app uses — without these the simulator falls back to
/// a default sans and the goldens stop matching the user's reality.
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

/// Build a fully-seeded demo app on the Arrange tab. The
/// `expand_shelf` flag toggles the global-tracks shelf to its
/// expanded state.
fn build_app(expand_shelf: bool) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    if expand_shelf {
        let _ = app.update(Message::Ui(UiMessage::ToggleGlobalTracks));
    }
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(WINDOW.0, WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

/// Collapsed state — only the 32 px summary strip is visible.
#[test]
fn global_tracks_shelf_collapsed() {
    let app = build_app(false);
    snapshot_to(&app, "tests/snapshots/global_tracks_shelf_collapsed.png");
}

/// Expanded state — shelf header + chord / tempo / signature lanes.
#[test]
fn global_tracks_shelf_expanded() {
    let app = build_app(true);
    snapshot_to(&app, "tests/snapshots/global_tracks_shelf_expanded.png");
}

/// Seed two chord-track regions over the demo's first section — bars 1-2
/// pinned to `Gmaj7`, bars 3-4 an unpinned `Dm` — so the chord lane draws
/// its region strip (UX-02).
fn seed_chord_track_regions(app: &mut Resonance) {
    use resonance_app::chord_track::ChordRegion;
    use resonance_music_theory::{Chord, ChordQuality, PitchClass};
    let tm = app.test_tempo_map();
    let (b0, b2, b4) = (tm.bar_to_sample(0), tm.bar_to_sample(2), tm.bar_to_sample(4));
    let track = app.test_chord_track_mut();
    track.insert_region(ChordRegion {
        id: 900_001,
        chord: Chord::new(PitchClass::G, ChordQuality::Maj7),
        start_sample: b0,
        end_sample: b2,
        pinned: true,
    });
    track.insert_region(ChordRegion {
        id: 900_002,
        chord: Chord::new(PitchClass::D, ChordQuality::Min),
        start_sample: b2,
        end_sample: b4,
        pinned: false,
    });
}

/// Expanded shelf with chord-track regions: the strip of region pills
/// under the section chords (pinned = accent + pin mark, unpinned =
/// outline), the section chords a pinned region overrides drawn dimmed,
/// and the header sub-line counting regions and pins (UX-02).
#[test]
fn global_tracks_shelf_chord_track_regions() {
    let mut app = build_app(true);
    seed_chord_track_regions(&mut app);
    snapshot_to(&app, "tests/snapshots/global_tracks_shelf_chord_track_regions.png");
}

/// A rejected chord symbol surfaces as the chord lane's sub-line in the
/// error colour (UX-02: `last_error` used to be set and never shown).
#[test]
fn global_tracks_shelf_chord_track_error_is_shown() {
    use resonance_app::message::ChordTrackMessage;
    let mut app = build_app(true);
    seed_chord_track_regions(&mut app);
    let _ = app.update(Message::ChordTrack(ChordTrackMessage::SetSymbol {
        id: 900_002,
        symbol: "Xq#9".to_string(),
    }));
    assert!(
        app.test_chord_track().last_error.is_some(),
        "a bad symbol must set last_error"
    );
    snapshot_to(&app, "tests/snapshots/global_tracks_shelf_chord_track_error.png");
}
