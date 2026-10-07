//! The project pronunciation dictionary and per-syllable overrides
//! survive save → reopen and every undo/redo (BUG-50,
//! vocal-mcp-control-and-quality slice 11).
//!
//! Before `ProjectFile::pronunciation` existed both lived only in
//! `PronunciationState`, which the `ComposeSections` restore cleared — so
//! a `vocal.set_pronunciation` was gone after a reload and after *any*
//! undo, even of an unrelated edit.

use std::path::PathBuf;

use resonance_app::project::{load_project, save_project, ProjectFile};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackType};
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::edit::{RedoResult, UndoResult};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::song::VocalView;
use resonance_control::methods::track as track_proto;
use resonance_control::methods::vocal as proto;
use resonance_control::{MutationAck, Request, Response};

use crate::common::roundtrip;

const TRACK: u64 = 30;
const TOMATO: [&str; 6] = ["t", "ah", "m", "aa", "t", "ow"];

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/control-vocal-pronunciation.rprj"));
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// A placed section with a vocal lane on `TRACK`; returns its definition id.
fn vocal_lane(app: &mut Resonance) -> u64 {
    let section_id = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create")
    .section_id;
    app.test_add_track(TRACK, TrackType::Vocal);
    let def = u64::from(section_id);
    app.test_install_vocal_lane(def, TRACK);
    def
}

fn set_lyrics(app: &mut Resonance, text: &str) {
    let _: MutationAck = call(
        app,
        "vocal.set_lyrics",
        &proto::SetLyricsParams {
            syllabify: false,
            track_id: ProtoTrackId(TRACK),
            section_id: None,
            text: text.to_owned(),
        },
    )
    .result()
    .expect("vocal.set_lyrics succeeds");
}

fn set_pronunciation(app: &mut Resonance, word: &str, phonemes: &[&str]) {
    let _: MutationAck = call(
        app,
        "vocal.set_pronunciation",
        &proto::SetPronunciationParams {
            word: word.to_owned(),
            phonemes: phonemes.iter().map(|p| (*p).to_owned()).collect(),
        },
    )
    .result()
    .expect("vocal.set_pronunciation succeeds");
}

fn undo(app: &mut Resonance) -> UndoResult {
    roundtrip(app, Request::without_params(98, "edit.undo"))
        .result()
        .expect("edit.undo succeeds")
}

fn redo(app: &mut Resonance) -> RedoResult {
    roundtrip(app, Request::without_params(97, "edit.redo"))
        .result()
        .expect("edit.redo succeeds")
}

/// Give the lane `notes` quarter notes at its placement — the clip
/// `song.vocal`'s per-note readout resolves against.
fn add_clip(app: &mut Resonance, track: u64, notes: usize) {
    use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
    let bar = app.compose_state().placements[0].start_bar;
    let start_sample = app.test_tempo_map().bar_to_sample(bar);
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id: 900_000 + track,
        track_id: track,
        start_sample,
        duration_ticks: 4 * 4 * TICKS_PER_QUARTER_NOTE,
        name: "Verse · Vox".to_owned(),
        notes: (0..notes as u64)
            .map(|i| MidiNote {
                note: 64,
                velocity: 0.8,
                start_tick: i * TICKS_PER_QUARTER_NOTE,
                duration_ticks: TICKS_PER_QUARTER_NOTE,
            })
            .collect(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
}

/// The lane's per-note phonemes as `song.vocal` reports them — the
/// production resolve (`PronunciationState::resolve_lane` with the
/// lane's bank dictionary, then the bank's gate), the same one the render
/// plans with.
fn sung(app: &mut Resonance, track: u64) -> Vec<Vec<String>> {
    let view: VocalView = roundtrip(
        app,
        Request::new(9, "song.vocal", &serde_json::json!({ "track_id": track })).unwrap(),
    )
    .result()
    .expect("song.vocal succeeds");
    view.lanes[0].notes.iter().map(|n| n.phonemes.clone()).collect()
}

fn strings(phonemes: &[&str]) -> Vec<String> {
    phonemes.iter().map(|p| (*p).to_owned()).collect()
}

fn dictionary(app: &Resonance) -> Vec<(String, Vec<String>)> {
    app.test_pronunciation_dictionary()
}

fn tomato_entry() -> Vec<(String, Vec<String>)> {
    vec![(
        "tomato".to_owned(),
        TOMATO.iter().map(|p| (*p).to_owned()).collect(),
    )]
}

fn reopen(app: &Resonance, tag: &str) -> Resonance {
    let dir = std::env::temp_dir().join(format!(
        "resonance-pronunciation-persist-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create project dir");
    let file = app.test_build_project_file();
    save_project(&dir, &file, &[], &[]).expect("save project");
    let json = std::fs::read_to_string(dir.join("project.json")).expect("read project.json");
    assert!(json.contains("\"pronunciation\""), "pronunciation written to disk");

    let loaded = load_project(&dir).expect("load project");
    let (mut reopened, _task) = Resonance::new_for_test();
    reopened.test_replay_loaded_project_from(loaded);
    let _ = std::fs::remove_dir_all(&dir);
    reopened
}

#[test]
fn dictionary_and_overrides_survive_save_and_reopen() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    set_lyrics(&mut app, "tomato red");
    set_pronunciation(&mut app, "Tomato", &TOMATO);
    app.test_set_syllable_override(def, TRACK, 1, "red", &["r", "iy", "d"]);

    let mut reopened = reopen(&app, "save-reopen");
    reopened.test_set_active_project(true);

    assert_eq!(dictionary(&reopened), tomato_entry());
    assert!(reopened
        .compose_state()
        .pronunciation
        .lane_overrides(def, TRACK)
        .is_some());
    add_clip(&mut reopened, TRACK, 2);
    assert_eq!(
        sung(&mut reopened, TRACK),
        vec![strings(&TOMATO), strings(&["r", "iy", "d"])]
    );
}

#[test]
fn undo_of_an_unrelated_edit_keeps_pronunciation() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    set_lyrics(&mut app, "tomato red");
    set_pronunciation(&mut app, "tomato", &TOMATO);
    app.test_set_syllable_override(def, TRACK, 1, "red", &["r", "iy", "d"]);

    // An unrelated edit, then undo it: the restore runs the whole
    // reconcile table, which used to clear pronunciation on the way.
    set_lyrics(&mut app, "tomato red again");
    undo(&mut app);

    assert_eq!(dictionary(&app), tomato_entry());
    assert!(app.compose_state().pronunciation.lane_overrides(def, TRACK).is_some());
    add_clip(&mut app, TRACK, 2);
    assert_eq!(
        sung(&mut app, TRACK),
        vec![strings(&TOMATO), strings(&["r", "iy", "d"])]
    );

    redo(&mut app);
    assert_eq!(dictionary(&app), tomato_entry(), "and on redo");
}

#[test]
fn set_and_clear_pronunciation_are_one_undo_step_each() {
    let mut app = app_with_project();
    vocal_lane(&mut app);

    set_pronunciation(&mut app, "tomato", &TOMATO);
    let undone = undo(&mut app);
    assert!(undone.undone.is_some(), "set_pronunciation recorded an undo entry");
    assert!(dictionary(&app).is_empty(), "undo takes the entry back");
    redo(&mut app);
    assert_eq!(dictionary(&app), tomato_entry(), "redo restores it");

    // Replacing an entry undoes to the previous phonemes.
    set_pronunciation(&mut app, "tomato", &["t", "ah", "m", "ey", "t", "ow"]);
    undo(&mut app);
    assert_eq!(dictionary(&app), tomato_entry());

    let _: MutationAck = call(
        &mut app,
        "vocal.clear_pronunciation",
        &proto::ClearPronunciationParams { word: "tomato".to_owned() },
    )
    .result()
    .expect("vocal.clear_pronunciation succeeds");
    assert!(dictionary(&app).is_empty());
    undo(&mut app);
    assert_eq!(dictionary(&app), tomato_entry(), "undoing a clear brings it back");
    redo(&mut app);
    assert!(dictionary(&app).is_empty());
}

#[test]
fn an_override_is_dropped_when_its_syllable_text_changes() {
    let mut app = app_with_project();
    let def = vocal_lane(&mut app);
    set_lyrics(&mut app, "tomato red");
    app.test_set_syllable_override(def, TRACK, 1, "red", &["r", "iy", "d"]);
    add_clip(&mut app, TRACK, 2);
    assert_eq!(sung(&mut app, TRACK)[1], strings(&["r", "iy", "d"]));

    // Same syllable index, different text: the override was made for
    // other lyrics and no longer applies.
    set_lyrics(&mut app, "tomato blue");
    assert_eq!(sung(&mut app, TRACK)[1], strings(&["b", "l", "uw"]));

    // Writing the old text back revives it (it is kept, just inert).
    set_lyrics(&mut app, "tomato red");
    assert_eq!(sung(&mut app, TRACK)[1], strings(&["r", "iy", "d"]));
}

#[test]
fn a_project_without_the_field_loads_with_no_pronunciation() {
    // Every project saved before the field existed, and every project
    // that never touched pronunciation (the field is skipped when empty).
    let json = serde_json::to_value(ProjectFile::default()).unwrap();
    assert!(json.get("pronunciation").is_none(), "empty is left out of the file");
    let file: ProjectFile = serde_json::from_value(json).expect("legacy project deserializes");
    assert!(file.pronunciation.is_empty());

    // Loading it over a project that had entries leaves none behind.
    let mut app = app_with_project();
    vocal_lane(&mut app);
    set_pronunciation(&mut app, "tomato", &TOMATO);
    app.test_replay_loaded_project(file);
    assert!(dictionary(&app).is_empty());
    assert!(app.compose_state().pronunciation.overrides.is_empty());
}

/// A deleted lane's overrides stay in the live state (undoing the delete
/// brings them back) and so in a save, but a reopen drops them: otherwise
/// a new vocal track, which takes the freed highest id back, would sing
/// them on its own lane.
#[test]
fn a_deleted_lanes_overrides_do_not_revive_on_a_reused_track_id() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/control-vocal-pronunciation-orphan.rprj"));
    // `track.add`, then the engine's reply that registers the track.
    let add = |app: &mut Resonance, kind: &str| -> u64 {
        let id: u64 = call(app, track_proto::ADD, &serde_json::json!({ "kind": kind }))
            .result::<track_proto::AddResult>()
            .expect("track.add")
            .track_id
            .into();
        app.test_apply_engine_event(match kind {
            "vocal" => AudioEvent::VocalTrackAdded { track_id: id },
            _ => AudioEvent::TrackAdded { track_id: id },
        });
        id
    };
    let _kept = add(&mut app, "audio");
    let section_id = call(
        &mut app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create")
    .section_id;
    let def = u64::from(section_id);
    let lane_on = |app: &mut Resonance| -> u64 {
        let track = add(app, "vocal");
        app.test_install_vocal_lane(def, track);
        let _: MutationAck = call(
            app,
            "vocal.set_lyrics",
            &proto::SetLyricsParams {
                syllabify: false,
                track_id: ProtoTrackId(track),
                section_id: None,
                text: "tomato red".to_owned(),
            },
        )
        .result()
        .expect("vocal.set_lyrics succeeds");
        track
    };
    let vocal = lane_on(&mut app);
    app.test_set_syllable_override(def, vocal, 1, "red", &["r", "iy", "d"]);

    let deleted = call(
        &mut app,
        track_proto::DELETE,
        &serde_json::json!({ "track_id": vocal, "confirm": true }),
    );
    assert!(deleted.error.is_none(), "{:?}", deleted.error);
    // Undo keeps them: the delete's undo brings the lane back with them.
    undo(&mut app);
    assert!(app
        .compose_state()
        .pronunciation
        .lane_overrides(def, vocal)
        .is_some());
    redo(&mut app);

    let mut reopened = reopen(&app, "orphan");
    reopened.test_set_active_project(true);
    assert!(
        reopened.compose_state().pronunciation.overrides.is_empty(),
        "the deleted track's override was dropped on load"
    );
    let reused = lane_on(&mut reopened);
    assert_eq!(reused, vocal, "precondition: the new track takes the freed id");
    add_clip(&mut reopened, reused, 2);
    assert_eq!(
        sung(&mut reopened, reused)[1],
        strings(&["r", "eh", "d"]),
        "CMU's red: no override applies"
    );
}
