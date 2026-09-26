//! Confirming the MIDI Import modal actually imports (code review VIEW-25 /
//! FU-V2a).
//!
//! Confirm used to be a no-op, the Review stage had no Import button and
//! the TempoConflict stage was a placeholder. Now Confirm lands every
//! selected track as a new track + clip, the tempo conflict is a real
//! choice (keep the project tempo — by bars or by time — or adopt the
//! file's), and the whole import is ONE undoable edit, while the dialog's
//! own interactions stay out of the history.

use std::path::PathBuf;

use resonance_app::message::{ImportMessage, Message};
use resonance_app::state::{ImportStage, TempoAlignment, TempoChoice, ViewMode};
use resonance_app::update::import::parse_import_file;
use resonance_app::Resonance;
use resonance_audio::midi_io::{write_midi_project, MidiTrackSource};
use resonance_audio::types::{MidiNote, TempoMap, TICKS_PER_QUARTER_NOTE};

const TPQ: u64 = TICKS_PER_QUARTER_NOTE;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/import-dialog-confirm.rprj"));
    app.test_set_sample_rate(48_000);
    app.test_rebuild_tempo_map();
    app
}

fn note(pitch: u8, beat: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick: beat * TPQ,
        duration_ticks: TPQ,
    }
}

/// A Format-1 file at `bpm`: conductor + "Bass" (3 notes) + "Keys" (2).
fn two_track_file(tag: &str, bpm: f32) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance-import-confirm-{tag}-{}.mid",
        std::process::id()
    ));
    let bass = [note(36, 0), note(38, 1), note(40, 2)];
    let keys = [note(60, 0), note(64, 1)];
    let mut map = TempoMap::default();
    map.bpm = bpm;
    write_midi_project(
        &path,
        &map,
        &[
            MidiTrackSource {
                name: "Bass",
                notes: &bass,
            },
            MidiTrackSource {
                name: "Keys",
                notes: &keys,
            },
        ],
    )
    .expect("write test MIDI file");
    path
}

/// Drive the dialog to its post-parse stage through the real reducer.
fn open_parsed(app: &mut Resonance, path: &PathBuf) {
    let _ = app.update(Message::Import(ImportMessage::FileDropped(path.clone())));
    let project_bpm = app.test_tempo_events()[0].bpm;
    let _ = app.update(Message::Import(ImportMessage::Parsed {
        path: path.clone(),
        result: parse_import_file(path, project_bpm),
    }));
}

fn send(app: &mut Resonance, m: ImportMessage) {
    let _ = app.update(Message::Import(m));
}

#[test]
fn confirm_imports_every_selected_track_as_one_undoable_edit() {
    let mut app = app();
    let path = two_track_file("one-edit", 120.0);
    open_parsed(&mut app, &path);
    assert_eq!(
        app.test_import_dialog().map(|d| d.stage),
        Some(ImportStage::Review)
    );
    // The dialog's own interactions are not edits.
    send(&mut app, ImportMessage::SetAllTracks(true));
    let undo_before = app.test_undo_history().undo_len();
    let revision_before = app.revision();
    assert_eq!(undo_before, 0, "reviewing records nothing");

    send(&mut app, ImportMessage::Confirm);

    let clips = app.test_midi_clips();
    assert_eq!(clips.len(), 2, "one clip per selected track");
    let mut counts: Vec<usize> = clips.iter().map(|c| c.notes.len()).collect();
    counts.sort_unstable();
    assert_eq!(counts, vec![2, 3]);
    assert_ne!(clips[0].track_id, clips[1].track_id, "each on its own track");
    assert_eq!(app.test_undo_history().undo_len(), 1, "ONE undo entry");
    assert_eq!(app.revision(), revision_before + 1, "ONE revision bump");
    let dialog = app.test_import_dialog().expect("dialog shows the outcome");
    assert_eq!(dialog.stage, ImportStage::Imported);
    let result = dialog.result.expect("result summary");
    assert_eq!(result.tracks_created, 2);
    assert_eq!(result.clips_added, 2);
    assert_eq!(result.notes_imported, 5);
}

#[test]
fn deselected_tracks_are_left_out() {
    let mut app = app();
    let path = two_track_file("deselect", 120.0);
    open_parsed(&mut app, &path);
    let keys = app
        .test_import_dialog()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.name == "Keys")
        .expect("Keys row");
    send(&mut app, ImportMessage::ToggleTrack(keys));
    send(&mut app, ImportMessage::Confirm);
    let clips = app.test_midi_clips();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].notes.len(), 3);
    assert_eq!(clips[0].name, "Bass");
}

#[test]
fn confirm_with_nothing_selected_records_nothing() {
    let mut app = app();
    let path = two_track_file("none", 120.0);
    open_parsed(&mut app, &path);
    send(&mut app, ImportMessage::SetAllTracks(false));
    let revision_before = app.revision();
    send(&mut app, ImportMessage::Confirm);
    assert!(app.test_midi_clips().is_empty());
    assert_eq!(app.test_undo_history().undo_len(), 0);
    assert_eq!(app.revision(), revision_before);
}

#[test]
fn keep_project_tempo_matching_bars_keeps_ticks_and_tempo() {
    let mut app = app();
    let path = two_track_file("bars", 90.0);
    open_parsed(&mut app, &path);
    assert_eq!(
        app.test_import_dialog().map(|d| d.stage),
        Some(ImportStage::TempoConflict)
    );
    send(&mut app, ImportMessage::SetTempoChoice(TempoChoice::KeepProject));
    send(&mut app, ImportMessage::SetConflictAlignment(TempoAlignment::MatchBars));
    send(&mut app, ImportMessage::ResolveTempo);
    assert_eq!(
        app.test_import_dialog().map(|d| d.stage),
        Some(ImportStage::Review)
    );
    send(&mut app, ImportMessage::Confirm);
    assert_eq!(app.test_tempo_events()[0].bpm, 120.0, "project tempo kept");
    let bass = app
        .test_midi_clips()
        .iter()
        .find(|c| c.name == "Bass")
        .expect("Bass clip");
    let starts: Vec<u64> = bass.notes.iter().map(|n| n.start_tick).collect();
    assert_eq!(starts, vec![0, TPQ, 2 * TPQ], "bar/beat positions preserved");
}

#[test]
fn keep_project_tempo_matching_time_rescales_onto_the_project_tempo() {
    let mut app = app();
    let path = two_track_file("time", 90.0);
    open_parsed(&mut app, &path);
    send(&mut app, ImportMessage::SetTempoChoice(TempoChoice::KeepProject));
    send(&mut app, ImportMessage::SetConflictAlignment(TempoAlignment::MatchTime));
    send(&mut app, ImportMessage::ResolveTempo);
    send(&mut app, ImportMessage::Confirm);
    assert_eq!(app.test_tempo_events()[0].bpm, 120.0, "project tempo kept");
    let bass = app
        .test_midi_clips()
        .iter()
        .find(|c| c.name == "Bass")
        .expect("Bass clip");
    // One beat at 90 BPM is 2/3 s = 4/3 beats at 120 BPM.
    let starts: Vec<u64> = bass.notes.iter().map(|n| n.start_tick).collect();
    assert_eq!(starts, vec![0, 640, 1280], "wall-clock timing preserved");
    assert_eq!(bass.notes[0].duration_ticks, 640);
}

#[test]
fn adopting_the_file_tempo_rewrites_the_project_tempo_in_the_same_edit() {
    let mut app = app();
    let path = two_track_file("adopt", 90.0);
    open_parsed(&mut app, &path);
    send(&mut app, ImportMessage::SetTempoChoice(TempoChoice::AdoptFile));
    send(&mut app, ImportMessage::ResolveTempo);
    send(&mut app, ImportMessage::Confirm);
    // SMF stores microseconds per quarter, so 90 BPM round-trips as ~89.99995.
    let bpm = app.test_tempo_events()[0].bpm;
    assert!((bpm - 90.0).abs() < 0.01, "file tempo adopted (got {bpm})");
    let bass = app
        .test_midi_clips()
        .iter()
        .find(|c| c.name == "Bass")
        .expect("Bass clip");
    let starts: Vec<u64> = bass.notes.iter().map(|n| n.start_tick).collect();
    assert_eq!(starts, vec![0, TPQ, 2 * TPQ]);
    assert_eq!(app.test_undo_history().undo_len(), 1, "tempo + notes: ONE edit");
}

/// A one-track file at `bpm` whose single note sits on `beat`.
fn one_note_file(tag: &str, bpm: f32, beat: u64) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "resonance-import-confirm-{tag}-{}.mid",
        std::process::id()
    ));
    let lead = [note(72, beat)];
    let mut map = TempoMap::default();
    map.bpm = bpm;
    write_midi_project(&path, &map, &[MidiTrackSource { name: "Lead", notes: &lead }])
        .expect("write test MIDI file");
    path
}

/// FU-V4a: "match time" follows the project's tempo map across the
/// imported span, not just the tempo at the import point.
#[test]
fn matching_time_follows_a_project_tempo_change_inside_the_span() {
    use resonance_app::state::TempoEvent;
    let mut app = app();
    // 120 BPM for bar 1, then a step to 60 BPM from bar 2 on.
    app.test_push_tempo_event(TempoEvent { bar: 1, bpm: 120.0 });
    app.test_push_tempo_event(TempoEvent { bar: 1, bpm: 60.0 });
    app.test_rebuild_tempo_map();
    // Beat 6 at 90 BPM is 4 s in: bar 1 takes 2 s at 120, the remaining
    // 2 s at 60 BPM are 2 more beats — project beat 6, not the 8 beats a
    // constant 120 BPM rescale gives.
    let path = one_note_file("time-change", 90.0, 6);
    open_parsed(&mut app, &path);
    send(&mut app, ImportMessage::SetTempoChoice(TempoChoice::KeepProject));
    send(&mut app, ImportMessage::SetConflictAlignment(TempoAlignment::MatchTime));
    send(&mut app, ImportMessage::ResolveTempo);
    send(&mut app, ImportMessage::Confirm);
    let lead = app
        .test_midi_clips()
        .iter()
        .find(|c| c.name == "Lead")
        .expect("Lead clip");
    let start = lead.notes[0].start_tick as i64;
    assert!((start - 6 * TPQ as i64).abs() <= 2, "wall-clock onset (got {start})");
}

/// FU-V4a: adopting the file tempo for an import placed at the playhead
/// rewrites the tempo from the placement bar on; the bars before it keep
/// the project's tempo.
#[test]
fn adopting_the_file_tempo_at_the_playhead_leaves_earlier_bars_alone() {
    use resonance_app::message::TransportMessage;
    use resonance_app::state::PlacementStart;
    let mut app = app();
    let bar_3 = app.test_tempo_map().bar_to_sample(2);
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(bar_3)));
    let path = one_note_file("adopt-playhead", 90.0, 0);
    open_parsed(&mut app, &path);
    send(&mut app, ImportMessage::SetTempoChoice(TempoChoice::AdoptFile));
    send(&mut app, ImportMessage::ResolveTempo);
    send(&mut app, ImportMessage::SetPlacementStart(PlacementStart::Playhead));
    send(&mut app, ImportMessage::Confirm);

    let events: Vec<(u32, f32)> = app.test_tempo_events().iter().map(|e| (e.bar, e.bpm)).collect();
    assert_eq!(events[0], (0, 120.0), "bars before the placement keep their tempo");
    let at_placement = events
        .iter()
        .rev()
        .find(|(bar, _)| *bar <= 2)
        .expect("a tempo governs the placement bar");
    assert_eq!(at_placement.0, 2, "the file tempo starts at the placement bar");
    assert!((at_placement.1 - 90.0).abs() < 0.01, "file tempo adopted: {events:?}");
    let lead = app
        .test_midi_clips()
        .iter()
        .find(|c| c.name == "Lead")
        .expect("Lead clip");
    assert_eq!(lead.start_sample, bar_3, "the clip lands on the placement bar");
}

#[test]
fn confirm_is_refused_before_the_tempo_conflict_is_resolved() {
    let mut app = app();
    let path = two_track_file("unresolved", 90.0);
    open_parsed(&mut app, &path);
    send(&mut app, ImportMessage::Confirm);
    assert!(app.test_midi_clips().is_empty());
    assert_eq!(app.test_undo_history().undo_len(), 0);
}
