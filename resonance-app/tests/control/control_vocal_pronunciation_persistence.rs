//! The project pronunciation dictionary and per-syllable overrides
//! survive save → reopen and every undo/redo (BUG-50,
//! vocal-mcp-control-and-quality slice 11).
//!
//! Before `ProjectFile::pronunciation` existed both lived only in
//! `PronunciationState`, which the `ComposeSections` restore cleared — so
//! a `vocal.set_pronunciation` was gone after a reload and after *any*
//! undo, even of an unrelated edit.

use std::path::PathBuf;

use resonance_app::compose::vocal_svs::PhonemeProvenance;
use resonance_app::compose::LaneGeneratorKind;
use resonance_app::project::{load_project, save_project, ProjectFile};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::TrackType;
use resonance_control::ids::TrackId as ProtoTrackId;
use resonance_control::methods::edit::{RedoResult, UndoResult};
use resonance_control::methods::section as section_proto;
use resonance_control::methods::vocal as proto;
use resonance_control::{MutationAck, Request, Response};
use resonance_music_theory::g2p::AssignedSyllable;

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

/// The lane's pronunciation the way the render resolves it: the lane's
/// draft, its overrides and the project dictionary.
fn resolve_lane(app: &Resonance, def: u64, note_count: usize) -> Vec<AssignedSyllable> {
    let compose = app.compose_state();
    let config = compose
        .find_definition(def)
        .and_then(|d| d.lane_generators.get(&TRACK))
        .expect("vocal lane");
    let LaneGeneratorKind::Vocal(params) = &config.kind else {
        panic!("not a vocal lane");
    };
    compose
        .pronunciation
        .resolve_lane(def, TRACK, &params.draft, &[], note_count, &[])
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

    let reopened = reopen(&app, "save-reopen");

    assert_eq!(dictionary(&reopened), tomato_entry());
    let resolved = resolve_lane(&reopened, def, 2);
    assert_eq!(resolved[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(resolved[0].phonemes, TOMATO.to_vec());
    assert_eq!(resolved[1].provenance, PhonemeProvenance::Edited);
    assert_eq!(resolved[1].phonemes, vec!["r", "iy", "d"]);
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
    let resolved = resolve_lane(&app, def, 2);
    assert_eq!(resolved[0].provenance, PhonemeProvenance::Dict);
    assert_eq!(resolved[1].provenance, PhonemeProvenance::Edited);

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
    assert_eq!(resolve_lane(&app, def, 2)[1].provenance, PhonemeProvenance::Edited);

    // Same syllable index, different text: the override was made for
    // other lyrics and no longer applies.
    set_lyrics(&mut app, "tomato blue");
    let resolved = resolve_lane(&app, def, 2);
    assert_ne!(resolved[1].provenance, PhonemeProvenance::Edited);
    assert_ne!(resolved[1].phonemes, vec!["r", "iy", "d"]);

    // Writing the old text back revives it (it is kept, just inert).
    set_lyrics(&mut app, "tomato red");
    assert_eq!(resolve_lane(&app, def, 2)[1].provenance, PhonemeProvenance::Edited);
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
