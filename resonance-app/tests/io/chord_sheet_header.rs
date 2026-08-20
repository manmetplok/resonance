//! The chord sheet PDF's page header (ba todo #1390).
//!
//! Two defects, one visible symptom — "the header shows the wrong time
//! signature":
//!
//! 1. The denominator was the literal `/4`, so a 6/8 song printed 6/4.
//! 2. The numerator (and the tempo) came from `r.transport`, which
//!    tracks the PLAYHEAD, not the song. On a song that changes meter
//!    the header printed whatever sat under the cursor at export time,
//!    and seeking changed the exported PDF without the song changing.
//!
//! The header text is asserted directly rather than through the PDF
//! bytes: `build_chord_sheet_pdf` flate-compresses its content streams,
//! so grepping the output for "6/8" proves nothing either way.
//! `SongHeader::info_line` is the exact string `draw_page_header` draws.

use resonance_app::chord_sheet_pdf::SongHeader;
use resonance_app::message::{GlobalTrackMessage, Message, TransportMessage};
use resonance_app::update::project_io;
use resonance_app::Resonance;

/// A test app with a project open, so the global-track edits below get
/// past `gates_message` the way they do in the real app.
fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app
}

/// Set the meter the song starts in (the bar-0 signature event; the add
/// upserts).
fn start_meter(app: &mut Resonance, numerator: u8, denominator: u8) {
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar: 0,
        numerator,
        denominator,
    }));
}

fn change_meter_at(app: &mut Resonance, bar: u32, numerator: u8, denominator: u8) {
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar,
        numerator,
        denominator,
    }));
}

fn start_tempo(app: &mut Resonance, bpm: f32) {
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar: 0,
        bpm,
    }));
}

/// The header the export handler would actually print.
fn exported_line(app: &Resonance) -> String {
    project_io::chord_sheet_header(app).info_line()
}

#[test]
fn six_eight_song_prints_six_eight() {
    let mut app = app_with_project();
    start_tempo(&mut app, 96.0);
    start_meter(&mut app, 6, 8);

    // The bug: this used to be "Tempo: 96 BPM  |  6/4" — the numerator
    // was real, the denominator was hardcoded.
    assert_eq!(exported_line(&app), "Tempo: 96 BPM  |  6/8");
}

#[test]
fn default_project_prints_four_four() {
    let app = app_with_project();

    assert_eq!(exported_line(&app), "Tempo: 120 BPM  |  4/4");
}

#[test]
fn song_that_changes_meter_says_so() {
    let mut app = app_with_project();
    start_tempo(&mut app, 120.0);
    start_meter(&mut app, 6, 8);
    change_meter_at(&mut app, 16, 4, 4);

    // Printing a bare "6/8" here would claim the whole sheet is in 6/8.
    assert_eq!(exported_line(&app), "Tempo: 120 BPM  |  6/8 (changes)");
}

#[test]
fn song_that_changes_tempo_says_so() {
    let mut app = app_with_project();
    start_tempo(&mut app, 90.0);
    start_tempo(&mut app, 90.0); // upsert, same value: not a change
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar: 32,
        bpm: 140.0,
    }));
    start_meter(&mut app, 3, 4);

    assert_eq!(exported_line(&app), "Tempo: 90 BPM (changes)  |  3/4");
}

/// A repeated event that never departs from the opening value is not a
/// change, so it must not earn the "(changes)" note.
#[test]
fn repeated_identical_events_are_not_a_change() {
    let mut app = app_with_project();
    start_meter(&mut app, 5, 4);
    change_meter_at(&mut app, 8, 5, 4);
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar: 8,
        bpm: 120.0,
    }));

    assert_eq!(exported_line(&app), "Tempo: 120 BPM  |  5/4");
}

/// The header must describe the SONG, not the cursor.
///
/// `transport.time_sig_num` follows the playhead: with the cursor sitting
/// past a meter change it reads that later meter, not the one the song
/// opens in. Exporting used to print exactly that reading, so the same
/// project produced a different header depending on where the cursor was.
#[test]
fn header_ignores_the_meter_under_the_playhead() {
    let mut app = app_with_project();
    start_tempo(&mut app, 120.0);
    start_meter(&mut app, 6, 8);

    // Park the cursor at bar 8, then drop a 7/8 change at bar 4 — the
    // playhead is now inside it, so the transport reads 7/8.
    let bar_8 = app.test_tempo_map().bar_to_sample(8);
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(bar_8)));
    change_meter_at(&mut app, 4, 7, 8);

    assert_eq!(
        app.test_transport_time_sig(),
        (7, 8),
        "the playhead really is inside the 7/8 stretch"
    );
    // The song still STARTS in 6/8, and it changes.
    assert_eq!(exported_line(&app), "Tempo: 120 BPM  |  6/8 (changes)");
}

/// The bytes still build with a real header — the threading above must
/// not have broken the one public entry point.
#[test]
fn export_still_produces_a_pdf() {
    let mut app = app_with_project();
    start_meter(&mut app, 6, 8);

    let bytes = resonance_app::chord_sheet_pdf::build_chord_sheet_pdf(
        app.compose_state(),
        SongHeader::from_song(app.test_tempo_events(), app.test_signature_events()),
    );

    assert!(bytes.starts_with(b"%PDF"), "wrote a PDF");
}
