//! Regression coverage for the **tempo + time-signature edit cycle**
//! on the Arrange-view global-tracks shelf.
//!
//! Three classes of breakage led to this suite (reported 2026-05-19):
//!
//! 1. **Stale canvas cache.** `TimelineFingerprint` previously only
//!    tracked `tempo_points.len()` / `signature_points.len()`. Editing
//!    an *existing* point's value (BPM via drag / transport-bar
//!    commit, numerator/denominator via signature pick_list) mutated
//!    the underlying data but never invalidated `canvas::Cache`, so
//!    the tempo curve + signature pill markers kept showing the old
//!    values until some unrelated state change (zoom, scroll, clip
//!    move) triggered a fingerprint mismatch.
//!
//! 2. **Stale selection highlight.** `selected_global_event` was not
//!    in the fingerprint either — clicking a tempo dot updated state
//!    but the dot didn't switch to the accent color because the
//!    cache didn't repaint.
//!
//! 3. **`CycleTimeSignature` skipped `rebuild_and_send_tempo`.** The
//!    transport-bar shortcut mutated `signature_events[0]` but never
//!    rebuilt the GUI-side `tempo_map`, so the bar table + the lane's
//!    pill text remained at the pre-cycle value.
//!
//! The fingerprint changes are bracketed by `shelf_after_*` snapshot
//! tests below; the `rebuild_and_send_tempo` call from
//! `CycleTimeSignature` is covered by the data-level tests so a
//! future regression that silently drops the rebuild fails even
//! without a snapshot diff.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{GlobalTrackMessage, Message, TransportMessage, UiMessage};
use resonance_app::state::{GlobalTrackKind, SelectedGlobalEvent, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};

const WINDOW: (f32, f32) = (1440.0, 900.0);

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

fn build_expanded_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    // Expand the shelf so the chord / tempo / signature lanes are
    // visible — every assertion below targets the lane geometry.
    let _ = app.update(Message::Ui(UiMessage::ToggleGlobalTracks));
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
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

// ---------------- Data-level regressions ----------------

/// `CycleTimeSignature` must rebuild the GUI-side tempo map so the
/// signature lane redraws with the new numerator/denominator. Before
/// the fix, `signature_events[0]` was updated but `tempo_map
/// .signature_points[0]` (read by the draw routine) stayed at the
/// pre-cycle value.
#[test]
fn cycle_time_signature_rebuilds_tempo_map() {
    let mut app = build_expanded_app();

    // Demo seeds 6/8 — the next cycle step is 5/4.
    let map = app.test_tempo_map();
    assert_eq!(map.numerator, 6);
    assert_eq!(map.denominator, 8);
    assert_eq!(map.signature_points[0].numerator, 6);
    assert_eq!(map.signature_points[0].denominator, 8);

    let _ = app.update(Message::Transport(TransportMessage::CycleTimeSignature));

    assert_eq!(app.test_transport_time_sig(), (5, 4));
    // GUI-side tempo map fields must mirror the new signature.
    let map = app.test_tempo_map();
    assert_eq!(
        map.numerator, 5,
        "tempo_map.numerator stayed stale after CycleTimeSignature"
    );
    assert_eq!(
        map.denominator, 4,
        "tempo_map.denominator stayed stale after CycleTimeSignature"
    );
    // The signature_points[0] backing the draw routine must also
    // update (this is the value the canvas reads when painting the
    // sig-lane pill marker).
    assert_eq!(
        map.signature_points[0].numerator, 5,
        "tempo_map.signature_points[0].numerator stale; rebuild_and_send_tempo missing"
    );
    assert_eq!(
        map.signature_points[0].denominator, 4,
        "tempo_map.signature_points[0].denominator stale; rebuild_and_send_tempo missing"
    );
}

/// `GlobalTrackMessage::UpdateSignatureEvent` (the pick_list edit path)
/// must propagate to `tempo_map.signature_points` so the on-canvas
/// pill marker label refreshes.
#[test]
fn update_signature_event_propagates_to_tempo_map() {
    let mut app = build_expanded_app();

    let _ = app.update(Message::GlobalTrack(
        GlobalTrackMessage::UpdateSignatureEvent {
            index: 0,
            numerator: 7,
            denominator: 8,
        },
    ));

    assert_eq!(app.test_signature_events()[0].numerator, 7);
    assert_eq!(app.test_signature_events()[0].denominator, 8);
    let map = app.test_tempo_map();
    assert_eq!(map.signature_points[0].numerator, 7);
    assert_eq!(map.signature_points[0].denominator, 8);
    // Because event_sample is 0 and playhead is also 0, the transport
    // bar's display must also bump.
    assert_eq!(app.test_transport_time_sig(), (7, 8));
}

/// `UpdateTempoEvent` must drive the GUI-side tempo map. This is what
/// the dragged tempo dot path goes through every pointer-move during
/// a drag — without `rebuild_tempo_map` the curve never moves.
#[test]
fn update_tempo_event_propagates_to_tempo_map() {
    let mut app = build_expanded_app();

    // Demo seeds a single tempo event at bar 0, 90 BPM.
    assert_eq!(app.test_tempo_map().tempo_points.len(), 1);
    let original = app.test_tempo_map().tempo_points[0].bpm;
    assert!((original - 90.0).abs() < 0.01);

    let _ = app.update(Message::GlobalTrack(
        GlobalTrackMessage::UpdateTempoEvent {
            index: 0,
            // index == 0 is pinned to bar 0 by the reducer regardless
            // of what we pass — but pass a real value so a future bug
            // that drops the pin shows up.
            bar: 4,
            bpm: 140.0,
        },
    ));

    assert_eq!(
        app.test_tempo_events()[0].bar,
        0,
        "first tempo event should stay pinned to bar 0"
    );
    assert!((app.test_tempo_events()[0].bpm - 140.0).abs() < 0.01);
    // The draw routine reads the tempo_map snapshot, not tempo_events
    // — without `rebuild_tempo_map` it'd still see 90 BPM.
    assert!((app.test_tempo_map().tempo_points[0].bpm - 140.0).abs() < 0.01);
    // sync_tempo_display refreshes the transport-bar BPM readout.
    assert!((app.test_transport_bpm() - 140.0).abs() < 0.01);
}

/// Selecting a global event must update interaction state so the
/// canvas can recolor the dot / pill on the next paint.
#[test]
fn select_global_event_updates_state() {
    let mut app = build_expanded_app();

    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(Some(
        SelectedGlobalEvent {
            kind: GlobalTrackKind::Signature,
            index: 0,
        },
    ))));
    let sel = app
        .test_selected_global_event()
        .expect("selection should be set");
    assert_eq!(sel.kind, GlobalTrackKind::Signature);
    assert_eq!(sel.index, 0);

    // Re-select with None to deselect.
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(None)));
    assert!(app.test_selected_global_event().is_none());
}

// ---------------- Visual regressions ----------------

/// Snapshot after `CycleTimeSignature` — locks in the post-edit shelf
/// state so a regression that breaks the `rebuild_and_send_tempo`
/// path *and* the cache fingerprint shows up as a pixel diff.
#[test]
fn shelf_after_cycle_time_signature() {
    let mut app = build_expanded_app();
    let _ = app.update(Message::Transport(TransportMessage::CycleTimeSignature));
    snapshot_to(
        &app,
        "tests/snapshots/global_tracks_shelf_after_cycle_signature.png",
    );
}

/// Snapshot after editing the tempo event's BPM via `UpdateTempoEvent`
/// (the same path the drag handler uses). The tempo lane should redraw
/// at the new BPM; the transport-bar readout should match.
#[test]
fn shelf_after_update_tempo() {
    let mut app = build_expanded_app();
    let _ = app.update(Message::GlobalTrack(
        GlobalTrackMessage::UpdateTempoEvent {
            index: 0,
            bar: 0,
            bpm: 140.0,
        },
    ));
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::EndTempoDrag));
    snapshot_to(
        &app,
        "tests/snapshots/global_tracks_shelf_after_update_tempo.png",
    );
}
