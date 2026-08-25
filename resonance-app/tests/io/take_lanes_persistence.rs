//! Cycle-record take lanes survive save + reload (epic #15, design doc
//! #165, ba todo #412).
//!
//! Before this landed, a take group existed only in the app's engine-event
//! mirror: three loop passes produced three takes, the user comped them,
//! saved — and reopened a project with no take lanes at all. Doc #165's
//! acceptance is explicit that takes and the comp persist across save/load
//! and that no take is ever silently lost, so these tests take the **real
//! on-disk hop** (`save_project` -> `load_project`) rather than an
//! in-memory serde round trip: the failure being guarded against is a
//! field that never reaches `project.json`.
//!
//! Three things beyond the happy path are pinned here:
//!
//! - **The project-load leak.** `wipe_registry` cleared `aux`, `sidechain`
//!   and `external_instruments` but not `take_groups`, so opening project B
//!   kept project A's lanes and their `clip_ref`s into a different
//!   project's `audio/` directory.
//! - **Missing recorded audio.** A take whose WAV is gone is flagged, not
//!   dropped — dropping it would leave the comp cover referencing a take id
//!   that no longer exists.
//! - **Undo.** Take groups ride the `ProjectFile` snapshot, so a comp edit
//!   reverses through the structure-preserving diff replay.

use std::path::Path;

use resonance_app::project::{load_project, save_project, LoadedProject, ProjectFile};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::{
    Comp, CompSegment, SlotCover, Take, TakeContent, TakeGroup, TakeNote, TimelineRange,
};

const TRACK: u64 = 7;
const OTHER_TRACK: u64 = 8;
const GROUP: u64 = 1;
const OTHER_GROUP: u64 = 2;

/// The loop region every take in these tests was recorded over: two bars
/// at 120 BPM / 48 kHz, i.e. four seconds.
const SLOT: TimelineRange = TimelineRange {
    start: 96_000,
    length: 192_000,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A fresh app with an active project anchored at `dir`, so the take
/// restore can resolve project-relative WAV paths and the undo gate (which
/// needs a saved path) is satisfied.
fn app_at(dir: &Path) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.to_path_buf());
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_add_track(OTHER_TRACK, TrackType::Instrument);
    app
}

/// Mirror one finished cycle-record pass the way the live app gets there:
/// the engine emits `TakeCaptured`, the app folds it into its lanes.
///
/// The take id is now assigned by the engine and carried on the event
/// (todo #409 replaced the app-side `take_id_for` derivation); mirroring
/// `pass_index` into it keeps this suite's existing id expectations.
fn capture_audio_pass(app: &mut Resonance, group_id: u64, pass_index: u32, clip_ref: u64) {
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id,
        take_id: u64::from(pass_index),
        track_id: TRACK,
        slot: SLOT,
        pass_index,
        extent: SLOT,
        content: TakeContent::Audio { clip_ref },
    });
}

fn capture_midi_pass(app: &mut Resonance, group_id: u64, pass_index: u32, note: u8) {
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id,
        take_id: u64::from(pass_index),
        track_id: OTHER_TRACK,
        slot: SLOT,
        pass_index,
        extent: SLOT,
        content: TakeContent::Midi {
            notes: vec![TakeNote {
                note,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 480,
            }],
        },
    });
}

/// Write an empty WAV at the path a take's `clip_ref` resolves to, so the
/// restore path's existence check takes the "present" branch. The engine
/// streams cycle-record passes to exactly this name.
fn write_take_wav(dir: &Path, clip_ref: u64) {
    let audio = dir.join("audio");
    std::fs::create_dir_all(&audio).expect("create audio dir");
    std::fs::write(audio.join(format!("clip_{clip_ref}.wav")), b"").expect("write take wav");
}

/// Three audio passes on TRACK plus two MIDI passes on OTHER_TRACK,
/// comped so the composite plays take 0, then take 2, then take 1 — a
/// cover that is only reconstructible if segments persist in order.
fn authored_project(dir: &Path) -> Resonance {
    let mut app = app_at(dir);
    for pass in 0..3u32 {
        capture_audio_pass(&mut app, GROUP, pass, 100 + u64::from(pass));
        write_take_wav(dir, 100 + u64::from(pass));
    }
    capture_midi_pass(&mut app, OTHER_GROUP, 0, 60);
    capture_midi_pass(&mut app, OTHER_GROUP, 1, 64);

    // Comp / solo arrive the way they do live: as the engine's echoes of
    // the commands an edit sent (todo #411 routes both through dispatch).
    app.test_apply_engine_event(AudioEvent::TakeCompChanged {
        group_id: GROUP,
        segments: vec![
            CompSegment {
                range: TimelineRange::new(96_000, 64_000),
                take_id: 0,
            },
            CompSegment {
                range: TimelineRange::new(160_000, 64_000),
                take_id: 2,
            },
            CompSegment {
                range: TimelineRange::new(224_000, 64_000),
                take_id: 1,
            },
        ],
    });
    app.test_apply_engine_event(AudioEvent::ActiveTakeChanged {
        group_id: OTHER_GROUP,
        take_id: Some(1),
    });
    app
}

/// Replay `file` into a fresh app anchored at `dir`, exactly as opening
/// the project from disk does — `project_dir` is the real directory so the
/// take restore can check whether each recorded WAV is still there.
fn open_into(dir: &Path, file: ProjectFile) -> Resonance {
    let mut app = app_at(dir);
    app.test_replay_loaded_project_from(LoadedProject {
        file,
        project_dir: dir.to_path_buf(),
        midi_notes: std::collections::HashMap::new(),
        plugin_states: std::collections::HashMap::new(),
    });
    app
}

// ---------------------------------------------------------------------------
// Serialized shape
// ---------------------------------------------------------------------------

#[test]
fn build_project_file_captures_takes_comp_and_active_take() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = authored_project(dir.path());

    let file = app.test_build_project_file();
    assert_eq!(file.take_groups.len(), 2, "both groups serialized");

    // Sorted by group id, independent of capture order.
    let audio = &file.take_groups[0];
    assert_eq!(audio.id, GROUP);
    assert_eq!(audio.track_id, TRACK);
    assert_eq!(audio.slot, SLOT);
    assert_eq!(audio.takes.len(), 3, "three loop passes, three takes");
    assert_eq!(
        audio.takes.iter().map(|t| t.pass_index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(
        audio
            .takes
            .iter()
            .map(|t| t.content.clone())
            .collect::<Vec<_>>(),
        vec![
            TakeContent::Audio { clip_ref: 100 },
            TakeContent::Audio { clip_ref: 101 },
            TakeContent::Audio { clip_ref: 102 },
        ],
        "each pass keeps its own recorded clip"
    );
    assert_eq!(
        audio
            .comp
            .segments
            .iter()
            .map(|s| s.take_id)
            .collect::<Vec<_>>(),
        vec![0, 2, 1],
        "the comp's segment ORDER is what makes it a composite"
    );
    assert!(audio.is_full_cover(), "comp covers the slot");

    let midi = &file.take_groups[1];
    assert_eq!(midi.id, OTHER_GROUP);
    assert_eq!(midi.track_id, OTHER_TRACK);
    assert_eq!(midi.active_take, Some(1), "soloed take persists");
    assert!(
        matches!(&midi.takes[0].content, TakeContent::Midi { notes } if notes[0].note == 60),
        "MIDI takes carry their notes inline"
    );
}

#[test]
fn takes_and_comp_survive_the_real_disk_round_trip() {
    let dir = tempfile::tempdir().expect("temp dir");
    let authored = authored_project(dir.path());
    let saved = authored.test_build_project_file();

    // The real hop: through project.json and back.
    save_project(dir.path(), &saved, &[], &[]).expect("save project");
    let loaded = load_project(dir.path()).expect("load project");
    assert_eq!(
        loaded.file.take_groups, saved.take_groups,
        "take lanes reach project.json intact"
    );

    // Replay into a fresh app and re-serialize: a clean round trip means
    // saving the reopened project writes the same durable shape.
    let reopened = open_into(dir.path(), loaded.file);
    assert_eq!(
        reopened.test_take_groups(),
        saved.take_groups.as_slice(),
        "the reopened app mirrors exactly what was saved"
    );
    assert!(
        reopened.test_missing_takes().is_empty(),
        "every recorded WAV travelled with the bundle"
    );
    assert_eq!(
        reopened.test_build_project_file().take_groups,
        saved.take_groups,
        "re-saving the reopened project is idempotent"
    );
}

#[test]
fn legacy_project_without_take_groups_loads_clean() {
    // A project authored before take lanes existed has no `take_groups`
    // key at all; `#[serde(default)]` must supply an empty Vec rather than
    // failing the load.
    let mut json = serde_json::to_value(ProjectFile::default()).expect("to value");
    json.as_object_mut().unwrap().remove("take_groups");

    let file: ProjectFile = serde_json::from_value(json).expect("legacy project deserializes");
    assert!(file.take_groups.is_empty());

    let dir = tempfile::tempdir().expect("temp dir");
    let app = open_into(dir.path(), file);
    assert!(app.test_take_groups().is_empty());
    assert!(app.test_missing_takes().is_empty());
}

// ---------------------------------------------------------------------------
// The project-load leak (doc #292, "#412 — a project-load leak to fix")
// ---------------------------------------------------------------------------

/// Opening project B must not inherit project A's take lanes.
///
/// `replay_take_groups` only *adds* what the file carries — dropping the
/// previous project's mirror is `wipe_registry`'s job, alongside the
/// `aux` / `sidechain` / `external_instruments` clears it already did.
/// Without that clear, project A's groups survived into B carrying
/// `clip_ref`s that name WAVs in A's `audio/` directory: lanes drawn for
/// takes that are not this project's, and a comp the user never made.
#[test]
fn opening_another_project_does_not_keep_the_previous_projects_take_lanes() {
    let dir_a = tempfile::tempdir().expect("temp dir a");
    let project_a = authored_project(dir_a.path()).test_build_project_file();

    // Open A, then open B (which has no take lanes) into the same app.
    let dir_b = tempfile::tempdir().expect("temp dir b");
    let mut app = open_into(dir_a.path(), project_a);
    assert_eq!(app.test_take_groups().len(), 2, "A's lanes loaded");

    app.test_replay_loaded_project_from(LoadedProject {
        file: ProjectFile::default(),
        project_dir: dir_b.path().to_path_buf(),
        midi_notes: std::collections::HashMap::new(),
        plugin_states: std::collections::HashMap::new(),
    });

    assert!(
        app.test_take_groups().is_empty(),
        "project A's take groups must not survive into project B"
    );
    assert!(
        app.test_missing_takes().is_empty(),
        "and neither may their missing-file flags"
    );
    assert!(
        app.test_build_project_file().take_groups.is_empty(),
        "so saving B cannot write A's takes into B's project.json"
    );
}

/// The same leak in the direction that actually corrupts data: B has take
/// lanes of its own, so a surviving group from A would sit *beside* them
/// with clip refs pointing at a different bundle.
#[test]
fn a_second_project_with_its_own_lanes_replaces_rather_than_merges() {
    let dir_a = tempfile::tempdir().expect("temp dir a");
    let project_a = authored_project(dir_a.path()).test_build_project_file();

    let dir_b = tempfile::tempdir().expect("temp dir b");
    let mut b_app = app_at(dir_b.path());
    capture_audio_pass(&mut b_app, GROUP, 0, 500);
    write_take_wav(dir_b.path(), 500);
    let project_b = b_app.test_build_project_file();

    let mut app = open_into(dir_a.path(), project_a);
    app.test_replay_loaded_project_from(LoadedProject {
        file: project_b,
        project_dir: dir_b.path().to_path_buf(),
        midi_notes: std::collections::HashMap::new(),
        plugin_states: std::collections::HashMap::new(),
    });

    let groups = app.test_take_groups();
    assert_eq!(groups.len(), 1, "only B's lane remains");
    assert_eq!(groups[0].takes.len(), 1);
    assert_eq!(
        groups[0].takes[0].content,
        TakeContent::Audio { clip_ref: 500 },
        "and it references B's recording, not A's"
    );
}

// ---------------------------------------------------------------------------
// Missing recorded audio
// ---------------------------------------------------------------------------

/// A take whose WAV is gone is kept and flagged, never dropped.
///
/// The comp addresses takes by id, so dropping the take would leave a
/// segment naming a take that no longer exists — a hole in the composite
/// with nothing to point at, and no way back once the next save rewrites
/// the file without it. Mirrors how `restore_pool` keeps a missing asset.
#[test]
fn a_take_whose_recorded_audio_is_gone_is_flagged_not_dropped() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = authored_project(dir.path()).test_build_project_file();

    // The bundle travelled without take 1's WAV.
    std::fs::remove_file(dir.path().join("audio/clip_101.wav")).expect("remove take wav");

    let app = open_into(dir.path(), file);

    let group = &app.test_take_groups()[0];
    assert_eq!(group.takes.len(), 3, "all three takes survive the load");
    assert_eq!(
        group
            .comp
            .segments
            .iter()
            .map(|s| s.take_id)
            .collect::<Vec<_>>(),
        vec![0, 2, 1],
        "and the comp cover is untouched"
    );
    assert!(group.is_full_cover(), "no hole punched in the composite");

    assert_eq!(
        app.test_missing_takes(),
        vec![(GROUP, 1)],
        "exactly the take with no audio on disk is flagged"
    );

    // Saving again must write the take back out — a flagged take is a
    // relink candidate, not a deletion.
    assert_eq!(
        app.test_build_project_file().take_groups[0].takes.len(),
        3,
        "the missing take is not dropped on the next save"
    );
}

/// MIDI takes carry their notes inline, so they can never be missing —
/// not even in a project directory that has no `audio/` folder at all.
#[test]
fn midi_takes_are_never_flagged_missing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app_at(dir.path());
    capture_midi_pass(&mut app, OTHER_GROUP, 0, 60);
    let file = app.test_build_project_file();

    let reopened = open_into(dir.path(), file);
    assert_eq!(reopened.test_take_groups().len(), 1);
    assert!(reopened.test_missing_takes().is_empty());
}

/// Re-recording over a flagged slot clears the flag: a freshly captured
/// take has its WAV on disk by definition.
#[test]
fn capturing_a_take_clears_a_stale_missing_flag() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = authored_project(dir.path()).test_build_project_file();
    std::fs::remove_file(dir.path().join("audio/clip_101.wav")).expect("remove take wav");

    let mut app = open_into(dir.path(), file);
    assert_eq!(app.test_missing_takes(), vec![(GROUP, 1)]);

    capture_audio_pass(&mut app, GROUP, 1, 999);
    assert!(app.test_missing_takes().is_empty());
    assert_eq!(
        app.test_take_groups()[0].takes[1].content,
        TakeContent::Audio { clip_ref: 999 },
        "the re-capture replaced the take in place"
    );
}

// ---------------------------------------------------------------------------
// Undo / redo (diff replay)
// ---------------------------------------------------------------------------

/// Take lanes are part of `ProjectFile`, so a comp edit reverses through
/// the ordinary undo snapshot — and, because take groups carry no engine
/// instances, through the structure-preserving *fast* path rather than a
/// full clear-and-replay.
#[test]
fn a_comp_edit_reverses_through_the_diff_replay() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = authored_project(dir.path());

    let before = app.test_snapshot_for_undo();
    let original: Vec<u64> = app.test_take_groups()[0]
        .comp
        .segments
        .iter()
        .map(|s| s.take_id)
        .collect();

    // Promote take 1 across the whole slot and solo take 2, echoed back
    // from the engine exactly as todo #411's handlers drive it.
    let mut comp = Comp::new();
    comp.promote(SLOT, 1, SlotCover::NONE);
    app.test_apply_engine_event(AudioEvent::TakeCompChanged {
        group_id: GROUP,
        segments: comp.segments.clone(),
    });
    app.test_apply_engine_event(AudioEvent::ActiveTakeChanged {
        group_id: GROUP,
        take_id: Some(2),
    });
    assert_eq!(app.test_take_groups()[0].comp.segments.len(), 1);

    app.test_begin_restore_from_snapshot(before);

    let restored = &app.test_take_groups()[0];
    assert_eq!(
        restored
            .comp
            .segments
            .iter()
            .map(|s| s.take_id)
            .collect::<Vec<_>>(),
        original,
        "the three-segment comp comes back"
    );
    assert_eq!(
        restored.active_take, None,
        "and so does the un-soloed state"
    );
    assert_eq!(restored.takes.len(), 3, "no take lost across the restore");
}

/// Undoing back past a capture removes the take again, and redoing brings
/// it back — take groups are not a monotonically growing side-table.
#[test]
fn undo_and_redo_move_across_a_captured_pass() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app_at(dir.path());
    capture_audio_pass(&mut app, GROUP, 0, 100);
    write_take_wav(dir.path(), 100);

    let one_take = app.test_snapshot_for_undo();
    capture_audio_pass(&mut app, GROUP, 1, 101);
    write_take_wav(dir.path(), 101);
    let two_takes = app.test_snapshot_for_undo();
    assert_eq!(app.test_take_groups()[0].takes.len(), 2);

    app.test_begin_restore_from_snapshot(one_take);
    assert_eq!(
        app.test_take_groups()[0].takes.len(),
        1,
        "undo drops the second pass"
    );

    app.test_begin_restore_from_snapshot(two_takes);
    assert_eq!(
        app.test_take_groups()[0].takes.len(),
        2,
        "redo brings it back"
    );
}

// ---------------------------------------------------------------------------
// Back-compat / hand-written files
// ---------------------------------------------------------------------------

/// A take group written by hand (or by a build whose comp helpers differ)
/// is restored verbatim rather than normalised, so a reader can always
/// tell what the file said.
#[test]
fn take_groups_round_trip_through_serde_verbatim() {
    let mut group = TakeGroup::new(9, 3, TimelineRange::new(0, 48_000));
    group.add_take(Take::new(
        0,
        0,
        1_700_000_000_000,
        TimelineRange::new(12_000, 36_000),
        TakeContent::Audio { clip_ref: 42 },
    ));
    group.add_take(Take::new(
        1,
        1,
        1_700_000_001_000,
        TimelineRange::new(0, 48_000),
        TakeContent::Midi {
            notes: vec![TakeNote {
                note: 48,
                velocity: 0.5,
                start_tick: 240,
                duration_ticks: 120,
            }],
        },
    ));
    group.comp.segments = vec![CompSegment {
        range: TimelineRange::new(0, 48_000),
        take_id: 1,
    }];
    group.active_take = Some(0);

    let file = ProjectFile {
        take_groups: vec![group.clone()],
        ..ProjectFile::default()
    };
    let json = serde_json::to_string(&file).expect("serialize");
    let back: ProjectFile = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back.take_groups, vec![group]);
}

// ---------------------------------------------------------------------------
// Reaching the engine (ba todo #1394)
// ---------------------------------------------------------------------------
//
// Persisting the lanes app-side is only half of doc #165's "persists across
// save/load": the engine renders the comp, and until `RestoreTakeGroups`
// existed nothing ever wrote its take-group store outside a live capture.
// The lanes came back on screen and the comp came back **silent**, on
// playback and on bounce alike.
//
// Asserting the mirror alone would pass for exactly that build, so these
// assert what the engine is *told* — the same reason `take_comp_edits.rs`
// checks commands rather than state.

/// Every `RestoreTakeGroups` in `cmds`, as the groups each carried.
fn restore_commands(cmds: &[AudioCommand]) -> Vec<Vec<TakeGroup>> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::RestoreTakeGroups { groups } => Some(groups.clone()),
            _ => None,
        })
        .collect()
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// Opening a project pushes its take groups into the engine, comp and all.
#[test]
fn opening_a_project_pushes_its_take_groups_into_the_engine() {
    let dir = tempfile::tempdir().expect("temp dir");
    let saved = authored_project(dir.path()).test_build_project_file();

    let mut app = app_at(dir.path());
    let rx = app.test_capture_engine();
    app.test_replay_loaded_project_from(LoadedProject {
        file: saved.clone(),
        project_dir: dir.path().to_path_buf(),
        midi_notes: std::collections::HashMap::new(),
        plugin_states: std::collections::HashMap::new(),
    });

    let sent = restore_commands(&drain(&rx));
    assert_eq!(sent.len(), 1, "exactly one restore per load");
    assert_eq!(
        sent[0], saved.take_groups,
        "the engine is handed the saved groups verbatim — ids, clip_refs, \
         comp and active take are what make the comp audible"
    );
}

/// A take whose WAV is gone is still sent. Its span renders silent, but the
/// take keeps its id, so every other segment of the cover keeps playing;
/// withholding it would leave the engine's comp naming a take it does not
/// hold, which `build_comp_table` resolves by dropping the *span*.
#[test]
fn a_missing_take_is_still_sent_to_the_engine() {
    let dir = tempfile::tempdir().expect("temp dir");
    let saved = authored_project(dir.path()).test_build_project_file();
    std::fs::remove_file(dir.path().join("audio/clip_101.wav")).expect("remove take wav");

    let mut app = app_at(dir.path());
    let rx = app.test_capture_engine();
    app.test_replay_loaded_project_from(LoadedProject {
        file: saved,
        project_dir: dir.path().to_path_buf(),
        midi_notes: std::collections::HashMap::new(),
        plugin_states: std::collections::HashMap::new(),
    });

    let sent = restore_commands(&drain(&rx));
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0][0].takes.len(),
        3,
        "the flagged take travels with the rest"
    );
    assert_eq!(app.test_missing_takes(), vec![(GROUP, 1)], "and is flagged");
}

/// A project with no take lanes still sends the command, so the engine's
/// store is emptied. Skipping the send for an empty project would leave the
/// previous project's comp governing clip ids the new project reuses —
/// those clips would vanish from the ordinary clip path and the stale comp
/// would play in their place.
#[test]
fn opening_a_lane_free_project_still_clears_the_engines_store() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app_at(dir.path());
    let rx = app.test_capture_engine();

    app.test_replay_loaded_project_from(LoadedProject {
        file: ProjectFile::default(),
        project_dir: dir.path().to_path_buf(),
        midi_notes: std::collections::HashMap::new(),
        plugin_states: std::collections::HashMap::new(),
    });

    let sent = restore_commands(&drain(&rx));
    assert_eq!(sent.len(), 1, "the send is unconditional");
    assert!(sent[0].is_empty(), "and carries nothing, which empties the store");
}

/// Undo/redo re-syncs the engine too. The diff replay sends no `ClearAll`,
/// so this command is the only thing that tells the engine a take was
/// deleted or a comp reversed — and it must carry the *target* snapshot's
/// groups, not the ones it is replacing.
#[test]
fn undoing_a_capture_re_syncs_the_engines_take_groups() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut app = app_at(dir.path());
    capture_audio_pass(&mut app, GROUP, 0, 100);
    write_take_wav(dir.path(), 100);

    let one_take = app.test_snapshot_for_undo();
    capture_audio_pass(&mut app, GROUP, 1, 101);
    write_take_wav(dir.path(), 101);

    let rx = app.test_capture_engine();
    app.test_begin_restore_from_snapshot(one_take);

    let sent = restore_commands(&drain(&rx));
    assert_eq!(sent.len(), 1, "the undo re-syncs the engine once");
    assert_eq!(sent[0].len(), 1, "one group");
    assert_eq!(
        sent[0][0].takes.len(),
        1,
        "the undone pass is gone from what the engine is told, not just from the mirror"
    );
    assert_eq!(
        sent[0][0].takes[0].content,
        TakeContent::Audio { clip_ref: 100 },
        "and the surviving take is the first pass"
    );
}

// ---------------------------------------------------------------------------
// The restored comp is AUDIBLE, not just present (ba todo #1402)
// ---------------------------------------------------------------------------
//
// `RestoreTakeGroups` (todo #1394) put the take *groups* back into the
// engine. It did not put their *audio* back: a take clip enters the engine
// only through the capture path, which pushes the `AudioClip` straight into
// the clip list, so after a reload `build_comp_table` resolved spans to clip
// ids the engine no longer held and the comp rendered silence — on playback
// and on bounce — exactly the bug #1394 was believed to have closed.
//
// #1394's own `a_restored_comp_bounces_identically_to_before_the_save`
// missed it because it simulates the reload at the wrong layer: it serde
// round-trips the take *group* while the fixture keeps handing the renderer
// its clips, so the engine never has to have loaded them. These tests take
// the clips away — the engine below is built from cold and driven by
// nothing but the commands the project load emitted.

use resonance_audio::__test_support::{
    build_comp_table, render_take_comp_for_test, EngineHandlerHarness,
};
use resonance_audio::transcode_to_wav;
use resonance_audio::types::{AudioClip, ClipSource, Track};

/// A short slot at a **non-zero** start, so the restored clip's origin is
/// load-bearing: `mix_track_comp` intersects every span with the clip's
/// `[start_sample, +duration_frames())`, so a clip restored to the wrong
/// origin misses the span entirely and renders silence.
const RSLOT: TimelineRange = TimelineRange {
    start: 8_000,
    length: 4_096,
};
const A_CLIP: u64 = 200;
const B_CLIP: u64 = 201;
/// Distinct DC levels, so "which take is audible where" is readable
/// straight off the rendered samples.
const A_LEVEL: f32 = 1.0;
const B_LEVEL: f32 = 0.25;

/// How far into the slot pass 0 punched in.
///
/// **Take 0's extent is deliberately NOT its slot.** Cycle recording gives
/// pass 0 the punch-in point as its clip start rather than the loop start,
/// which is the whole reason `Take::extent` is persisted (ba todo #1396) —
/// so a fixture where every take fills its slot cannot tell
/// `take.extent.start` apart from `group.slot.start`, and an
/// implementation that reached for the slot would pass. It is the same
/// adjacent-same-typed-values trap either way round; the fixture has to
/// separate them.
const PUNCH: u64 = 512;

/// The takes this suite's reload fixture is built from: clip id, DC level,
/// and the extent capture would have recorded — take 0 punched in, take 1
/// filling its slot.
fn reload_takes() -> [(u64, f32, TimelineRange); 2] {
    [
        (
            A_CLIP,
            A_LEVEL,
            TimelineRange::new(RSLOT.start + PUNCH, RSLOT.length - PUNCH),
        ),
        (B_CLIP, B_LEVEL, RSLOT),
    ]
}

/// Write a real, decodable DC-valued stereo WAV where a take's `clip_ref`
/// resolves. Unlike [`write_take_wav`], which only has to satisfy an
/// existence check, this one is actually mmap'd and rendered.
///
/// `frames` is the take's *extent* length, not the slot's: a punched-in
/// pass recorded fewer frames, and the clip has to be that long or the
/// engine's `[start_sample, +duration_frames())` intersection would not
/// match what capture left behind.
fn write_dc_take_wav(dir: &Path, clip_ref: u64, level: f32, frames: u64) {
    let audio = dir.join("audio");
    std::fs::create_dir_all(&audio).expect("create audio dir");
    let samples = vec![level; frames as usize * 2];
    transcode_to_wav(
        &audio.join(format!("clip_{clip_ref}.wav")),
        &samples,
        48_000,
    )
    .expect("write take wav");
}

/// Mirror the `AudioClip` a finished cycle-record pass leaves in the
/// engine's clip list — `RecordingState::roll_audio_pass` builds exactly
/// this: the rolled clip id, the armed track, the pass's start on the
/// timeline, no trims, no fades, unity gain.
///
/// This is the "before the save" side of the comparison: the engine as
/// capture leaves it, which is the state a reload has to reproduce.
fn captured_clip(clip_ref: u64, dir: &Path, extent: TimelineRange) -> AudioClip {
    let path = dir.join(format!("audio/clip_{clip_ref}.wav"));
    AudioClip {
        id: clip_ref,
        track_id: TRACK,
        // Where the pass really started — the punch-in point for take 0,
        // not the slot start.
        start_sample: extent.start,
        source: ClipSource::open_wav(&path).expect("open take wav"),
        name: format!("Take {clip_ref}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: Default::default(),
        fade_out_frames: 0,
        fade_out_curve: Default::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// An app holding two audio takes over [`RSLOT`], each backed by a real
/// WAV, comped so the first half plays take 0 and the second half take 1.
///
/// Take 0 is a **punched-in** pass ([`PUNCH`] frames into the slot); take 1
/// fills the slot. So the fixture carries one take whose extent differs
/// from its slot and one whose does not, and a restore that reached for
/// `group.slot.start` gets take 0 wrong while still getting take 1 right.
fn comped_project(dir: &Path) -> Resonance {
    let mut app = app_at(dir);
    for (pass, (clip_ref, level, extent)) in reload_takes().into_iter().enumerate() {
        write_dc_take_wav(dir, clip_ref, level, extent.length);
        app.test_apply_engine_event(AudioEvent::TakeCaptured {
            group_id: GROUP,
            take_id: pass as u64,
            track_id: TRACK,
            slot: RSLOT,
            pass_index: pass as u32,
            // What this pass really recorded over — and, for an audio
            // take, the record of where capture put its clip.
            extent,
            content: TakeContent::Audio { clip_ref },
        });
    }
    app.test_apply_engine_event(AudioEvent::TakeCompChanged {
        group_id: GROUP,
        segments: vec![
            CompSegment {
                range: TimelineRange::new(RSLOT.start, RSLOT.length / 2),
                take_id: 0,
            },
            CompSegment {
                range: TimelineRange::new(RSLOT.start + RSLOT.length / 2, RSLOT.length / 2),
                take_id: 1,
            },
        ],
    });
    app
}

/// The commands a real save + reload of `dir`'s project emits, through the
/// on-disk hop rather than an in-memory hand-off.
fn commands_from_a_real_reload(dir: &Path, app: &Resonance) -> Vec<AudioCommand> {
    let file = app.test_build_project_file();
    save_project(dir, &file, &[], &[]).expect("save project");
    let loaded = load_project(dir).expect("load project");

    let mut reopened = app_at(dir);
    let rx = reopened.test_capture_engine();
    reopened.test_replay_loaded_project_from(loaded);
    drain(&rx)
}

/// An engine built **from cold** — no tracks, no clips, no take groups, no
/// capture in its history — and then driven by `cmds`, of which it obeys
/// only the take-lane restore.
///
/// This is what makes these tests bite where #1394's did not: nothing
/// hands the renderer a clip. Every clip the comp finds below got there
/// because the project load told the engine to load it.
fn engine_rebuilt_by(cmds: &[AudioCommand]) -> EngineHandlerHarness {
    let mut engine = EngineHandlerHarness::new();
    for cmd in cmds {
        engine.replay_take_lane_command(cmd);
    }
    engine
}

/// Render `[RSLOT]` on `TRACK` through the production render block.
fn render(clips: Vec<AudioClip>, table: &resonance_audio::__test_support::CompRenderTable, live: bool) -> Vec<f32> {
    let out = render_take_comp_for_test(
        vec![Track::new(TRACK, "comped".into())],
        clips,
        table,
        RSLOT.start,
        RSLOT.length as usize,
        48_000,
        live,
    );
    out.chunks(2).map(|f| f[0]).collect()
}

/// The comp as it sounded before the save: the engine's take-group store
/// as capture left it, and the clips capture pushed into the clip list.
fn render_before_the_save(dir: &Path, app: &Resonance, live: bool) -> Vec<f32> {
    let groups: std::collections::HashMap<u64, TakeGroup> = app
        .test_build_project_file()
        .take_groups
        .into_iter()
        .map(|g| (g.id, g))
        .collect();
    render(
        reload_takes()
            .into_iter()
            .map(|(clip_ref, _, extent)| captured_clip(clip_ref, dir, extent))
            .collect(),
        &build_comp_table(&groups),
        live,
    )
}

/// The comp after a real save + reload, rendered out of an engine that
/// holds only what the load put there.
fn render_after_the_reload(cmds: &[AudioCommand], live: bool) -> Vec<f32> {
    let mut engine = engine_rebuilt_by(cmds);
    assert!(
        engine.wait_for_clips(2, std::time::Duration::from_secs(5)),
        "the reload never got the take clips into the engine — the comp \
         can only render silence; loaded clip ids: {:?}",
        engine.clip_ids()
    );
    let table = engine.published_comp_table();
    render(engine.take_clips(), &table, live)
}

/// **The acceptance test for ba todo #1402.** A comp across two takes
/// plays and bounces the same after a save + reload as it did before.
///
/// Sample-for-sample, out of an engine rebuilt from cold, so losing the
/// clips (the #1402 bug), the segment order, the slot binding or a clip's
/// origin all fail here rather than passing on a "something came out"
/// check.
#[test]
fn a_reloaded_comp_still_plays_and_bounces_what_it_did_before_the_save() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = comped_project(dir.path());

    let before_live = render_before_the_save(dir.path(), &app, true);
    let before_bounce = render_before_the_save(dir.path(), &app, false);
    assert_eq!(
        before_live, before_bounce,
        "the pre-save comp must already play as it bounces, or the \
         comparison below is measuring the wrong thing"
    );

    // Not vacuous: the two takes really are distinguishable, and the comp
    // really does switch between them mid-slot.
    let a_probe = (RSLOT.length / 4) as usize;
    let b_probe = (RSLOT.length * 3 / 4) as usize;
    assert!(
        (before_live[a_probe] - A_LEVEL).abs() < 1e-6,
        "first half must be take 0 at {A_LEVEL}, got {}",
        before_live[a_probe]
    );
    assert!(
        (before_live[b_probe] - B_LEVEL).abs() < 1e-6,
        "second half must be take 1 at {B_LEVEL}, got {}",
        before_live[b_probe]
    );

    // And the punch-in is audible in the render, not just in the metadata:
    // take 0's clip starts PUNCH frames into the slot, so the comp has
    // nothing to read before that and renders silence there. A clip
    // restored at the slot start instead would fill this gap — which is
    // what makes the sample-for-sample comparison below discriminate
    // `take.extent.start` from `group.slot.start`.
    assert!(
        before_live[..(PUNCH as usize)]
            .iter()
            .all(|s| s.abs() < 1e-6),
        "the punched-in head of the slot must be silent before the take starts"
    );
    // Past the take's own anti-click ramp (`CLIP_DECLICK_FRAMES` = 96) and
    // short of the seam crossfade into take 1.
    assert!(
        before_live[(PUNCH + 128) as usize..a_probe]
            .iter()
            .all(|s| (s - A_LEVEL).abs() < 1e-6),
        "and take 0 must be at full level once its clip has begun"
    );

    let cmds = commands_from_a_real_reload(dir.path(), &app);
    let after_live = render_after_the_reload(&cmds, true);
    let after_bounce = render_after_the_reload(&cmds, false);

    assert_eq!(
        after_live, after_bounce,
        "a reloaded comp must bounce exactly as it plays"
    );
    assert_eq!(
        after_live, before_live,
        "a reloaded comp must render sample-for-sample as it did before the save"
    );
}

/// The clips must be **governed** after a reload too. They are ordinary
/// `AudioClip`s in the engine's list, so if the comp table did not claim
/// them the raw, overlapping passes would all play at once on the normal
/// clip path, on top of the composite.
#[test]
fn reloaded_take_clips_are_governed_so_the_raw_passes_never_double_play() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = comped_project(dir.path());
    let cmds = commands_from_a_real_reload(dir.path(), &app);

    let engine = engine_rebuilt_by(&cmds);
    let table = engine.published_comp_table();
    assert!(
        table.is_governed(A_CLIP) && table.is_governed(B_CLIP),
        "both restored take clips must be governed"
    );
}

/// The comp table is published **before** the clips are asked for, so a
/// take clip is governed from the first instant it can exist.
///
/// The reverse order leaves a window in which the raw passes are loaded
/// but unclaimed, and every one of them plays. The window this order does
/// leave — a table naming a clip not yet loaded — is benign: a span whose
/// clip is missing is skipped, and the load is asynchronous regardless, so
/// no send order could close it.
#[test]
fn the_restore_publishes_the_comp_table_before_it_asks_for_the_clips() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = comped_project(dir.path());
    let cmds = commands_from_a_real_reload(dir.path(), &app);

    let restore = cmds
        .iter()
        .position(|c| matches!(c, AudioCommand::RestoreTakeGroups { .. }))
        .expect("the load restores the take groups");
    let first_load = cmds
        .iter()
        .position(|c| matches!(c, AudioCommand::LoadTakeClipFromWav { .. }))
        .expect("the load asks for the take clips");
    assert!(
        restore < first_load,
        "RestoreTakeGroups must precede the take clip loads"
    );
}

/// A take whose WAV is gone is still flagged and still restored, and the
/// load does not fail: no clip command is sent for it, the group keeps the
/// take, and every other segment of the cover keeps playing (#412's rule).
#[test]
fn a_missing_take_wav_is_flagged_and_asked_for_by_nobody() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = comped_project(dir.path());
    let file = app.test_build_project_file();
    save_project(dir.path(), &file, &[], &[]).expect("save project");
    std::fs::remove_file(dir.path().join(format!("audio/clip_{B_CLIP}.wav")))
        .expect("remove take wav");
    let loaded = load_project(dir.path()).expect("load project");

    let mut reopened = app_at(dir.path());
    let rx = reopened.test_capture_engine();
    reopened.test_replay_loaded_project_from(loaded);
    let cmds = drain(&rx);

    let asked_for: Vec<u64> = cmds
        .iter()
        .filter_map(|c| match c {
            AudioCommand::LoadTakeClipFromWav { clip_id, .. } => Some(*clip_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        asked_for,
        vec![A_CLIP],
        "only the take whose audio is still there is asked for"
    );
    assert_eq!(
        reopened.test_missing_takes(),
        vec![(GROUP, 1)],
        "and the one that is gone is flagged, not dropped"
    );
    assert_eq!(
        restore_commands(&cmds)[0][0].takes.len(),
        2,
        "the flagged take still travels to the engine, so the comp stays intact"
    );
}

/// A restored take clip is loaded at the origin capture gave it, taken
/// from the take's persisted `extent` — not at 0, and not at the slot
/// start by assumption.
#[test]
fn a_restored_take_clip_is_loaded_at_the_origin_capture_gave_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = comped_project(dir.path());
    let cmds = commands_from_a_real_reload(dir.path(), &app);

    let loads: Vec<(u64, u64, u64)> = cmds
        .iter()
        .filter_map(|c| match c {
            AudioCommand::LoadTakeClipFromWav {
                clip_id,
                track_id,
                start_sample,
                ..
            } => Some((*clip_id, *track_id, *start_sample)),
            _ => None,
        })
        .collect();
    assert_eq!(
        loads,
        vec![
            // Take 0 punched in, so its clip does NOT start where its slot
            // does. This is the pair that discriminates: reaching for
            // `group.slot.start` would give `RSLOT.start` here...
            (A_CLIP, TRACK, RSLOT.start + PUNCH),
            // ...while still being right for take 1, which filled its slot.
            (B_CLIP, TRACK, RSLOT.start),
        ],
        "each take clip is restored onto its own track at its extent's start"
    );
    assert_ne!(
        loads[0].2, RSLOT.start,
        "the punched-in take's origin must differ from its slot's, or this \
         test cannot tell `take.extent.start` from `group.slot.start`"
    );
}

/// Loading a take clip must **not** put it on the timeline. #1396 ruled
/// that a take clip is not a timeline clip and must never enter
/// `Resonance::clips`; that is the whole reason this load is a command of
/// its own rather than `LoadClipFromWav`, whose `ClipImported` echo would
/// have pushed a `ClipState` for every take.
#[test]
fn restoring_take_clips_does_not_put_them_on_the_timeline() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app = comped_project(dir.path());
    let file = app.test_build_project_file();
    save_project(dir.path(), &file, &[], &[]).expect("save project");
    let loaded = load_project(dir.path()).expect("load project");

    let reopened = open_into(dir.path(), loaded.file);
    assert!(
        reopened.test_clips().is_empty(),
        "take clips must stay off the timeline; found {:?}",
        reopened.test_clips().len()
    );
    assert!(
        file.clips.is_empty(),
        "and they were never in the saved clip list either"
    );
}
