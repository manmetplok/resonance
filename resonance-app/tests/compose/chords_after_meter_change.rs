//! Chords are revalidated when a global signature change shortens a
//! section (code review FU-V2b).
//!
//! A section is `length_bars` bars in the meter at its start, and every
//! chord edit is checked against that span (`chord_fits_in_section`). A
//! signature change re-measured the section but left its chords alone, so
//! 4/4 → 3/4 turned a chord on the last beat of a 4-bar section into one
//! past its end, which the chord lane and the generators then read. The
//! change now trims a straddling chord to the new end and drops one that
//! starts past it, as one edit with the signature change.

use resonance_app::message::{GlobalTrackMessage, Message, TransportMessage};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_control::ids::SectionDefinitionId;
use resonance_control::methods::section as section_proto;

use crate::common::call;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Compose);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/chords-after-meter.rprj"));
    app.test_set_sample_rate(48_000);
    app.test_rebuild_tempo_map();
    app
}

/// A placed 4-bar 4/4 section (16 beats) with chords at beats 0..4,
/// 8..12 and 12..16.
fn section_with_chords(app: &mut Resonance) -> u64 {
    let created: section_proto::CreateResult = call(
        app,
        "section.create",
        serde_json::json!({"name": "Verse", "length_bars": 4}),
    )
    .result()
    .expect("section.create");
    let SectionDefinitionId(id) = created.section_id;
    for (start, symbol) in [(0, "C"), (8, "F"), (12, "G")] {
        let add = call(
            app,
            "harmony.add_chord",
            serde_json::json!({
                "section_id": id,
                "start_beat": start,
                "duration_beats": 4,
                "symbol": symbol,
            }),
        );
        assert!(add.error.is_none(), "add_chord: {:?}", add.error);
    }
    id
}

#[test]
fn a_shorter_meter_trims_and_drops_chords_past_the_section_end() {
    let mut app = app();
    let id = section_with_chords(&mut app);
    assert_eq!(app.test_section_chord_spans(id), vec![(0, 4), (8, 4), (12, 4)]);

    // 3/4: the section is now 12 beats. 12..16 starts at the end → gone;
    // 8..12 ends exactly on it → kept.
    let _ = app.update(Message::Transport(TransportMessage::SetTimeSignature {
        numerator: 3,
        denominator: 4,
    }));
    assert_eq!(app.test_section_chord_spans(id), vec![(0, 4), (8, 4)]);
}

#[test]
fn a_straddling_chord_is_trimmed_to_the_new_end() {
    let mut app = app();
    let created: section_proto::CreateResult = call(
        &mut app,
        "section.create",
        serde_json::json!({"name": "Bridge", "length_bars": 4}),
    )
    .result()
    .expect("section.create");
    let SectionDefinitionId(id) = created.section_id;
    for (start, symbol) in [(0, "C"), (6, "Am")] {
        let add = call(
            &mut app,
            "harmony.add_chord",
            serde_json::json!({"section_id": id, "start_beat": start, "duration_beats": 4, "symbol": symbol}),
        );
        assert!(add.error.is_none(), "add_chord: {:?}", add.error);
    }

    // A longer meter never grows or restores anything.
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
        index: 0,
        numerator: 5,
        denominator: 8,
    }));
    assert_eq!(app.test_section_chord_spans(id), vec![(0, 4), (6, 4)]);

    // 2/4: 8 beats. 6..10 straddles the end → trimmed to 6..8.
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
        index: 0,
        numerator: 2,
        denominator: 4,
    }));
    assert_eq!(app.test_section_chord_spans(id), vec![(0, 4), (6, 2)]);

    // 1/4: 4 beats. 6..8 starts past the end → dropped.
    let _ = app.update(Message::Transport(TransportMessage::SetTimeSignature {
        numerator: 1,
        denominator: 4,
    }));
    assert_eq!(app.test_section_chord_spans(id), vec![(0, 4)]);
}

/// FU-V4b: a project FILE whose chords run past their section's end (a
/// hand edit, or one saved before FU-V2b) is revalidated on load too.
#[test]
fn chords_past_the_section_end_are_revalidated_on_load() {
    let mut app = app();
    let id = section_with_chords(&mut app);
    let mut file = app.test_build_project_file();
    let def = file
        .section_definitions
        .iter_mut()
        .find(|d| d.id == id)
        .expect("section saved");
    // 16 beats long: one chord straddling the end, one wholly past it.
    let mut straddling = def.chords[2].clone();
    straddling.id = 900;
    straddling.start_beat = 14;
    let mut past = def.chords[2].clone();
    past.id = 901;
    past.start_beat = 20;
    def.chords.retain(|c| c.start_beat < 12);
    def.chords.push(straddling);
    def.chords.push(past);

    app.test_replay_loaded_project(file);
    assert_eq!(app.test_section_chord_spans(id), vec![(0, 4), (8, 4), (14, 2)]);
}
