//! Clips the engine creates are named per track (ARCH-04 D-7d, design doc
//! D-6 §7a.1).
//!
//! The engine used to put the clip id in the name ("Recording 7"). Its
//! ids come from the app's one clip counter now, which starts at 2^40, so
//! it sends a bare kind ("Recording", "MIDI Take") and the app numbers the
//! clip on the echo: one more than the highest number that kind already
//! carries on the same track. The number is read off the names, so it
//! follows the project through an undo and a reload with no counter of
//! its own.

use std::collections::HashMap;
use std::sync::Arc;

use resonance_app::message::{Message, TransportMessage};
use resonance_app::project::LoadedProject;
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ClipId, MidiNote, TrackId, TrackType};

const A: TrackId = 1;
const B: TrackId = 2;
const KEYS: TrackId = 3;
/// Where the engine's granted ids live (the app's counter starts here).
const G: ClipId = 1 << 40;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/timeline-engine-clip-names.rprj"));
    app.test_add_track(A, TrackType::Audio);
    app.test_add_track(B, TrackType::Audio);
    app.test_add_track(KEYS, TrackType::Instrument);
    app
}

/// One recording session on `track`, as the engine reports it.
fn record(app: &mut Resonance, clip_id: ClipId, track_id: TrackId) {
    app.test_apply_engine_event(AudioEvent::RecordingStarted { start_sample: 0 });
    app.test_apply_engine_event(AudioEvent::RecordingFinished {
        clip_id,
        track_id,
        start_sample: 0,
        duration_samples: 48_000,
        name: "Recording".into(),
        waveform_peaks: Vec::new(),
    });
}

fn name_of(app: &Resonance, clip_id: ClipId) -> String {
    app.test_clips()
        .iter()
        .find(|c| c.id == clip_id)
        .unwrap_or_else(|| panic!("clip {clip_id} is mirrored"))
        .name
        .clone()
}

fn midi_name_of(app: &Resonance, clip_id: ClipId) -> String {
    app.test_midi_clips()
        .iter()
        .find(|c| c.id == clip_id)
        .unwrap_or_else(|| panic!("MIDI clip {clip_id} is mirrored"))
        .name
        .clone()
}

/// Reload the project from its own file shape, the way a disk open
/// replays it.
fn reload(app: &mut Resonance) {
    let file = app.test_build_project_file();
    let midi_notes: HashMap<ClipId, Arc<Vec<MidiNote>>> = app
        .test_midi_clips()
        .iter()
        .map(|c| (c.id, c.notes.clone()))
        .collect();
    app.test_replay_loaded_project_from(LoadedProject {
        file,
        project_dir: std::env::temp_dir().join("timeline-engine-clip-names.rprj"),
        midi_notes,
        plugin_states: HashMap::new(),
    });
    app.test_set_active_project(true);
}

#[test]
fn recordings_are_numbered_per_track_across_undo_and_reload() {
    let mut app = app();
    record(&mut app, G, A);
    record(&mut app, G + 1, A);
    record(&mut app, G + 2, B);
    assert_eq!(name_of(&app, G), "Recording 1");
    assert_eq!(name_of(&app, G + 1), "Recording 2");
    assert_eq!(name_of(&app, G + 2), "Recording 1", "track B counts on its own");

    // Undo B's take, then A's second: A's next take is its second again.
    let _ = app.update(Message::Undo);
    let _ = app.update(Message::Undo);
    assert!(!app.test_clips().iter().any(|c| c.id == G + 1));
    record(&mut app, G + 3, A);
    assert_eq!(name_of(&app, G + 3), "Recording 2");

    // The names are saved; the numbering picks up from them.
    reload(&mut app);
    assert_eq!(name_of(&app, G), "Recording 1");
    assert_eq!(name_of(&app, G + 3), "Recording 2");
    record(&mut app, G + 4, A);
    record(&mut app, G + 5, B);
    assert_eq!(name_of(&app, G + 4), "Recording 3");
    assert_eq!(name_of(&app, G + 5), "Recording 1");
}

/// The number follows the highest one still on the track, so a deleted or
/// renamed take never lets two clips share a name.
#[test]
fn a_gap_or_a_renamed_take_never_repeats_a_number() {
    let mut app = app();
    record(&mut app, G, A);
    record(&mut app, G + 1, A);
    record(&mut app, G + 2, A);
    // Rename the first and the second away (saved that way, reopened);
    // "Recording 3" is still there.
    let mut file = app.test_build_project_file();
    file.clips
        .iter_mut()
        .filter(|c| c.id == G || c.id == G + 1)
        .for_each(|c| c.name = "vocal comp".into());
    app.test_replay_loaded_project_from(LoadedProject {
        file,
        project_dir: std::env::temp_dir().join("timeline-engine-clip-names.rprj"),
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
    });
    assert_eq!(name_of(&app, G), "vocal comp");
    record(&mut app, G + 3, A);
    assert_eq!(name_of(&app, G + 3), "Recording 4");
}

/// A live-MIDI capture (FU-D6a: its own undo entry, on plain Play too) is
/// numbered the same way, among the MIDI clips on its track; each capture
/// still records its entry.
#[test]
fn live_midi_captures_are_numbered_per_track_and_stay_undoable() {
    let mut app = app();
    let entries = |app: &Resonance| app.test_undo_history().test_undo_entries().len();
    let capture = |app: &mut Resonance, clip_id: ClipId, track_id: TrackId| {
        let _ = app.update(Message::Transport(TransportMessage::Play));
        app.test_apply_engine_event(AudioEvent::MidiClipCreated {
            clip_id,
            track_id,
            start_sample: 0,
            duration_ticks: 0,
            name: "MIDI Take".into(),
            notes: Vec::new(),
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        });
        let _ = app.update(Message::Transport(TransportMessage::Stop));
    };
    capture(&mut app, G, KEYS);
    capture(&mut app, G + 1, KEYS);
    assert_eq!(midi_name_of(&app, G), "MIDI Take 1");
    assert_eq!(midi_name_of(&app, G + 1), "MIDI Take 2");
    assert_eq!(entries(&app), 2, "one undo entry per capture run");

    let _ = app.update(Message::Undo);
    assert!(!app.test_midi_clips().iter().any(|c| c.id == G + 1));
    capture(&mut app, G + 2, KEYS);
    assert_eq!(midi_name_of(&app, G + 2), "MIDI Take 2");

    reload(&mut app);
    capture(&mut app, G + 3, KEYS);
    assert_eq!(midi_name_of(&app, G + 3), "MIDI Take 3");
}
