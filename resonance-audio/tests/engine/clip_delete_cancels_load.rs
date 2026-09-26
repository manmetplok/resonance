//! `DeleteClip` against a clip load still on the import worker (code
//! review FU-A13e) or one that never succeeds (FU-A13f).
//!
//! An undo/redo burst over a clip add sends `LoadClipFromWav(id)`,
//! `DeleteClip(id)`, `LoadClipFromWav(id)` inside one load's latency. The
//! delete used to be parked behind the first load (ba doc #276 BUG 1's
//! deferral) and replayed against whichever load of the id reached the
//! clip list first; the second load's duplicate check could drop the one
//! that should have survived. The app — whose A-13i restore mirrors all
//! three at once — was left with a clip the engine did not have, or with
//! the wrong load's audio under it.
//!
//! And a clip whose WAV is missing never lands at all, so a delete of it
//! was parked until it timed out, with an error and no `ClipDeleted`: the
//! app's `RestoreEchoes` ledger owed that echo forever.
//!
//! The import pool is held (`EngineHandlerHarness::hold_imports`), so each
//! test runs the load jobs itself in the completion order it pins, and
//! polls the parked-edit replay after each step as the engine loop would.

use std::path::{Path, PathBuf};
use std::time::Duration;

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::transcode_to_wav;
use resonance_audio::types::{AudioCommand, AudioEvent};

const RATE: usize = 48_000;
/// The first load's WAV: one second.
const FIRST_FRAMES: u64 = RATE as u64;
/// The re-load's WAV: two seconds, so the surviving clip says which load
/// it came from.
const SECOND_FRAMES: u64 = 2 * RATE as u64;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-delete-cancels-load-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_wav(dir: &Path, name: &str, frames: u64) -> PathBuf {
    let samples = vec![0.25f32; frames as usize * 2];
    let path = dir.join(name);
    transcode_to_wav(&path, &samples, RATE as u32).expect("write test wav");
    path
}

fn load(clip_id: u64, path: &Path) -> AudioCommand {
    AudioCommand::LoadClipFromWav {
        clip_id,
        track_id: 1,
        start_sample: 0,
        path: path.to_path_buf(),
        name: format!("clip {clip_id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
    }
}

/// Run one held worker job, then one engine-loop pass.
fn run(engine: &mut EngineHandlerHarness, job: Box<dyn FnOnce() + Send>) {
    job();
    engine.poll_deferred_clip_commands();
}

/// The clip-lifecycle echoes for `clip_id`, in order: `("imported",
/// duration)` / `("deleted", 0)` / `("trimmed", 0)`.
fn lifecycle(events: &[AudioEvent], clip_id: u64) -> Vec<(&'static str, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::ClipImported {
                clip_id: id,
                duration_samples,
                ..
            } if *id == clip_id => Some(("imported", *duration_samples)),
            AudioEvent::ClipDeleted { clip_id: id } if *id == clip_id => Some(("deleted", 0)),
            AudioEvent::ClipTrimmed { clip_id: id, .. } if *id == clip_id => Some(("trimmed", 0)),
            _ => None,
        })
        .collect()
}

/// Load, delete, re-load; the three commands dispatched back to back,
/// then the two held loads run in `order` (indices into submission
/// order), polling after each. Returns the harness's final clips and the
/// echoes.
fn load_delete_reload(tag: &str, order: [usize; 2]) -> (Vec<(u64, u64)>, Vec<AudioEvent>, usize) {
    let dir = make_tempdir(tag);
    let first = write_wav(&dir, "first.wav", FIRST_FRAMES);
    let second = write_wav(&dir, "second.wav", SECOND_FRAMES);

    let mut engine = EngineHandlerHarness::new();
    engine.hold_imports();
    engine.dispatch(load(7, &first));
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 7 });
    engine.dispatch(load(7, &second));
    engine.poll_deferred_clip_commands();

    let mut jobs: Vec<Option<Box<dyn FnOnce() + Send>>> =
        engine.take_held_imports().into_iter().map(Some).collect();
    assert_eq!(jobs.len(), 2, "one worker job per load");
    for i in order {
        let job = jobs[i].take().expect("each job runs once");
        run(&mut engine, job);
    }

    let clips = engine.clip_frame_counts();
    let parked = engine.deferred_clip_command_count();
    let events = engine.finish_and_drain_events(Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&dir);
    (clips, events, parked)
}

fn assert_only_the_reload_survives(clips: &[(u64, u64)], events: &[AudioEvent], parked: usize) {
    assert_eq!(
        clips,
        &[(7, SECOND_FRAMES)],
        "the engine must end with exactly the re-loaded clip"
    );
    assert_eq!(
        lifecycle(events, 7),
        vec![("deleted", 0), ("imported", SECOND_FRAMES)],
        "one delete echo, then one import echo — the re-load's"
    );
    assert_eq!(parked, 0, "nothing left parked");
}

/// The first load finishes after the re-load.
#[test]
fn a_delete_cancels_a_load_that_finishes_after_the_reload() {
    let (clips, events, parked) = load_delete_reload("first-last", [1, 0]);
    assert_only_the_reload_survives(&clips, &events, parked);
}

/// The first load finishes first — after the delete, before the re-load.
#[test]
fn a_delete_cancels_a_load_that_finishes_before_the_reload() {
    let (clips, events, parked) = load_delete_reload("first-first", [0, 1]);
    assert_only_the_reload_survives(&clips, &events, parked);
}

/// Both loads finish before the engine loop gets round to the parked
/// delete — where the re-load's duplicate check met the first load's clip.
#[test]
fn a_delete_cancels_a_load_even_when_both_loads_finish_before_the_loop_polls() {
    let dir = make_tempdir("both-before-poll");
    let first = write_wav(&dir, "first.wav", FIRST_FRAMES);
    let second = write_wav(&dir, "second.wav", SECOND_FRAMES);

    let mut engine = EngineHandlerHarness::new();
    engine.hold_imports();
    engine.dispatch(load(7, &first));
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 7 });
    engine.dispatch(load(7, &second));
    for job in engine.take_held_imports() {
        job();
    }
    engine.poll_deferred_clip_commands();

    let clips = engine.clip_frame_counts();
    let parked = engine.deferred_clip_command_count();
    let events = engine.finish_and_drain_events(Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&dir);
    assert_only_the_reload_survives(&clips, &events, parked);
}

/// An edit of the first instance, parked before the delete, must not land
/// on the re-load that happens to share its id.
#[test]
fn an_edit_parked_before_the_delete_does_not_land_on_the_reload() {
    let dir = make_tempdir("parked-edit");
    let first = write_wav(&dir, "first.wav", FIRST_FRAMES);
    let second = write_wav(&dir, "second.wav", SECOND_FRAMES);

    let mut engine = EngineHandlerHarness::new();
    engine.hold_imports();
    engine.dispatch(load(7, &first));
    engine.dispatch(AudioCommand::TrimClip {
        clip_id: 7,
        new_start_sample: 0,
        trim_start_frames: 1_000,
        trim_end_frames: 0,
    });
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 7 });
    engine.dispatch(load(7, &second));
    for job in engine.take_held_imports() {
        run(&mut engine, job);
    }

    let clips = engine.clip_frame_counts();
    let parked = engine.deferred_clip_command_count();
    let events = engine.finish_and_drain_events(Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&dir);
    assert_only_the_reload_survives(&clips, &events, parked);
}

/// FU-A13f: a clip whose WAV is missing never lands; deleting it must
/// still echo `ClipDeleted`, at once — not after a ten-second timeout, and
/// not never.
#[test]
fn deleting_a_clip_whose_load_failed_still_echoes() {
    let dir = make_tempdir("missing");
    let missing = dir.join("gone.wav");

    let mut engine = EngineHandlerHarness::new();
    engine.hold_imports();
    engine.dispatch(load(7, &missing));
    for job in engine.take_held_imports() {
        run(&mut engine, job);
    }
    let failed = engine.drain_events();
    assert!(
        failed.iter().any(|e| matches!(e, AudioEvent::Error(_))),
        "the load of a missing WAV reports its failure: {failed:?}"
    );

    engine.dispatch(AudioCommand::DeleteClip { clip_id: 7 });
    let parked = engine.deferred_clip_command_count();
    let events = engine.drain_events();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(lifecycle(&events, 7), vec![("deleted", 0)], "the delete echoes");
    assert_eq!(parked, 0, "and is not parked waiting for a clip that is not coming");
}

/// FU-A13f, the other order: the delete lands while the doomed load is
/// still on the worker. One echo, and no failure reported for a load
/// nobody wants any more.
#[test]
fn deleting_a_clip_whose_load_is_still_failing_echoes_once() {
    let dir = make_tempdir("missing-in-flight");
    let missing = dir.join("gone.wav");

    let mut engine = EngineHandlerHarness::new();
    engine.hold_imports();
    engine.dispatch(load(7, &missing));
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 7 });
    for job in engine.take_held_imports() {
        run(&mut engine, job);
    }

    let parked = engine.deferred_clip_command_count();
    let events = engine.finish_and_drain_events(Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(lifecycle(&events, 7), vec![("deleted", 0)], "exactly one delete echo");
    assert_eq!(parked, 0, "nothing left parked");
}

/// A delete of an id the engine never heard of echoes too: the app only
/// ever owes an echo for a clip it mirrored, and the engine cannot tell
/// that clip from one it never loaded.
#[test]
fn deleting_an_unknown_clip_echoes() {
    let mut engine = EngineHandlerHarness::new();
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 42 });
    let parked = engine.deferred_clip_command_count();
    let events = engine.drain_events();
    assert_eq!(lifecycle(&events, 42), vec![("deleted", 0)]);
    assert_eq!(parked, 0);
}

/// The one delete that still waits: of the tail a parked split is about to
/// create. It runs after the split, in order, so the split's tail does not
/// outlive it.
#[test]
fn a_delete_of_a_parked_splits_tail_waits_for_the_split() {
    let dir = make_tempdir("split-tail");
    let wav = write_wav(&dir, "clip.wav", FIRST_FRAMES);

    let mut engine = EngineHandlerHarness::new();
    engine.hold_imports();
    engine.dispatch(load(5, &wav));
    engine.dispatch(AudioCommand::SplitClip {
        clip_id: 5,
        new_clip_id: 9,
        at_sample: FIRST_FRAMES / 2,
    });
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 9 });
    assert!(
        engine.drain_events().is_empty(),
        "nothing echoes before the parent lands"
    );
    for job in engine.take_held_imports() {
        run(&mut engine, job);
    }
    // A second loop pass: this pins the order, not how many passes it takes.
    engine.poll_deferred_clip_commands();

    let ids: Vec<u64> = engine.clip_frame_counts().iter().map(|(id, _)| *id).collect();
    let parked = engine.deferred_clip_command_count();
    let events = engine.finish_and_drain_events(Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(ids, vec![5], "the split ran, then the delete took its tail");
    assert_eq!(lifecycle(&events, 9), vec![("imported", FIRST_FRAMES / 2), ("deleted", 0)]);
    assert_eq!(parked, 0);
}

/// Deleting the parent of a parked split drops the split, and with it the
/// wait of a delete of its tail: that delete echoes at once.
#[test]
fn deleting_a_parked_splits_parent_resolves_the_tails_delete() {
    let dir = make_tempdir("split-parent");
    let wav = write_wav(&dir, "clip.wav", FIRST_FRAMES);

    let mut engine = EngineHandlerHarness::new();
    engine.hold_imports();
    engine.dispatch(load(5, &wav));
    engine.dispatch(AudioCommand::SplitClip {
        clip_id: 5,
        new_clip_id: 9,
        at_sample: FIRST_FRAMES / 2,
    });
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 9 });
    engine.dispatch(AudioCommand::DeleteClip { clip_id: 5 });
    for job in engine.take_held_imports() {
        run(&mut engine, job);
    }

    let clips = engine.clip_frame_counts();
    let parked = engine.deferred_clip_command_count();
    let events = engine.finish_and_drain_events(Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&dir);

    assert!(clips.is_empty(), "neither clip survives: {clips:?}");
    assert_eq!(lifecycle(&events, 5), vec![("deleted", 0)]);
    assert_eq!(lifecycle(&events, 9), vec![("deleted", 0)]);
    assert_eq!(parked, 0);
}
