//! Unit coverage for `UndoHistory`: capacity trimming, redo
//! invalidation, transaction commit, and coalesce-key behaviour.
//!
//! Moved out of an inline `#[cfg(test)]` module in `src/undo.rs` once
//! the crate grew a `lib.rs`. The private `capacity` / `undo` fields
//! are reached through the `#[doc(hidden)]` `test_set_capacity` /
//! `test_undo_entries` accessors on `UndoHistory`.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::undo::{CoalesceKey, UndoHistory, UndoSnapshot};

/// Produce a snapshot that carries `id` in its `project.file.bpm` field
/// so tests can distinguish snapshots on the history stack. `bpm` is
/// abused purely as a numeric discriminator here; the rest of the
/// snapshot is a valid default.
fn label(id: f32) -> String {
    format!("edit {id}")
}

fn dummy_snapshot(id: f32) -> UndoSnapshot {
    UndoSnapshot {
        project: LoadedProject {
            file: ProjectFile {
                bpm: id,
                ..ProjectFile::default()
            },
            project_dir: PathBuf::new(),
            midi_notes: HashMap::new(),
            plugin_states: HashMap::new(),
        },
    }
}

#[test]
fn record_clears_redo() {
    let mut h = UndoHistory::new();
    h.record(dummy_snapshot(1.0), label(1.0));
    // Simulate an undo: redo now has one entry.
    let (popped, popped_label) = h.pop_undo().unwrap();
    h.push_redo(popped, popped_label);
    assert!(h.can_redo());
    // Recording a new action must wipe redo.
    h.record(dummy_snapshot(2.0), label(2.0));
    assert!(!h.can_redo());
}

#[test]
fn capacity_trims_oldest() {
    let mut h = UndoHistory::new();
    h.test_set_capacity(3);
    h.record(dummy_snapshot(1.0), label(1.0));
    h.record(dummy_snapshot(2.0), label(2.0));
    h.record(dummy_snapshot(3.0), label(3.0));
    h.record(dummy_snapshot(4.0), label(4.0));
    assert_eq!(h.test_undo_entries().len(), 3);
    // The oldest (1.0) should have been trimmed; top of stack is 4.0.
    assert_eq!(h.pop_undo().unwrap().0.project.file.bpm, 4.0);
    assert_eq!(h.pop_undo().unwrap().0.project.file.bpm, 3.0);
    assert_eq!(h.pop_undo().unwrap().0.project.file.bpm, 2.0);
    assert!(h.pop_undo().is_none());
}

#[test]
fn commit_records_pending_transaction() {
    let mut h = UndoHistory::new();
    h.begin(dummy_snapshot(1.0), label(1.0));
    assert!(h.has_pending());
    h.commit();
    assert!(!h.has_pending());
    assert!(h.can_undo());
    assert_eq!(h.test_undo_entries()[0].project.file.bpm, 1.0);
}

#[test]
fn coalesces_same_key_and_breaks_on_intervening_action() {
    let mut h = UndoHistory::new();
    let key = CoalesceKey::TrackVolume(7);

    // First entry under `key` pushes normally.
    h.record_coalesced(dummy_snapshot(1.0), key.clone(), label(1.0));
    assert_eq!(h.test_undo_entries().len(), 1);
    // Subsequent entries under the same key do not push.
    h.record_coalesced(dummy_snapshot(2.0), key.clone(), label(2.0));
    h.record_coalesced(dummy_snapshot(3.0), key.clone(), label(3.0));
    assert_eq!(h.test_undo_entries().len(), 1);
    // The retained entry is the original (pre-burst) snapshot.
    assert_eq!(h.test_undo_entries()[0].project.file.bpm, 1.0);

    // A different coalesce key breaks the run and pushes a new entry.
    h.record_coalesced(dummy_snapshot(10.0), CoalesceKey::TrackPan(7), label(10.0));
    assert_eq!(h.test_undo_entries().len(), 2);

    // An atomic record also breaks any subsequent coalesce run.
    h.record(dummy_snapshot(20.0), label(20.0));
    h.record_coalesced(dummy_snapshot(4.0), key, label(4.0));
    assert_eq!(h.test_undo_entries().len(), 4);
}

/// `try_extend_coalesced` is the snapshot-free fast path `record_undo`
/// checks before building an O(project) snapshot: it must say yes only
/// when `record_coalesced` would have merged, and must never touch the
/// run-opening snapshot.
#[test]
fn try_extend_coalesced_only_continues_a_matching_run() {
    let mut h = UndoHistory::new();
    let key = CoalesceKey::TrackVolume(7);

    // Nothing recorded yet: no run to continue.
    assert!(!h.try_extend_coalesced(&key));

    h.record_coalesced(dummy_snapshot(1.0), key.clone(), label(1.0));
    assert!(h.try_extend_coalesced(&key), "same key continues the run");
    assert_eq!(h.test_undo_entries().len(), 1);
    assert_eq!(
        h.test_undo_entries()[0].project.file.bpm,
        1.0,
        "the run keeps its opening snapshot"
    );

    // A different control is not part of the run.
    assert!(!h.try_extend_coalesced(&CoalesceKey::TrackPan(7)));

    // An atomic record breaks the run...
    h.record(dummy_snapshot(2.0), label(2.0));
    assert!(!h.try_extend_coalesced(&key));

    // ...and so does popping an entry off the stack.
    h.record_coalesced(dummy_snapshot(3.0), key.clone(), label(3.0));
    h.pop_undo();
    assert!(!h.try_extend_coalesced(&key));
}

#[test]
fn coalesce_run_is_broken_by_pop() {
    let mut h = UndoHistory::new();
    let key = CoalesceKey::MasterVolume;
    h.record_coalesced(dummy_snapshot(1.0), key.clone(), label(1.0));
    h.pop_undo();
    // After popping, the next coalesced record must push fresh.
    h.record_coalesced(dummy_snapshot(2.0), key, label(2.0));
    assert_eq!(h.test_undo_entries().len(), 1);
    assert_eq!(h.test_undo_entries()[0].project.file.bpm, 2.0);
}

// ---- Compound groups (the control API's per-call atomicity) ----------

#[test]
fn compound_group_absorbs_everything_after_its_opening_mutation() {
    let mut h = UndoHistory::new();
    h.begin_compound();
    assert!(h.in_compound());
    // The first mutation arms the group and records normally...
    assert!(!h.absorb_into_compound(), "the opening mutation records");
    h.record(dummy_snapshot(1.0), label(1.0));
    // ...and every later mutation inside the group is absorbed.
    assert!(h.absorb_into_compound());
    assert!(h.absorb_into_compound());
    h.end_compound();
    assert!(!h.in_compound());
    assert_eq!(h.test_undo_entries().len(), 1, "one entry for the group");
    // Closed again: back to per-edit recording.
    assert!(!h.absorb_into_compound());
}

/// A group that saw no mutation leaves no trace: nothing armed, nothing
/// recorded, and the next edit records individually.
#[test]
fn an_empty_compound_group_records_nothing() {
    let mut h = UndoHistory::new();
    h.begin_compound();
    h.end_compound();
    assert_eq!(h.test_undo_entries().len(), 0);
    assert!(!h.absorb_into_compound(), "no group is open anymore");
}

/// Opening a group breaks an in-progress coalesce run, so a control
/// call landing mid-fader-drag records its own entry instead of merging
/// into the user's gesture — and the drag cannot merge into the group's
/// entry afterwards either.
#[test]
fn begin_compound_breaks_a_coalesce_run_in_both_directions() {
    let mut h = UndoHistory::new();
    let key = CoalesceKey::TrackVolume(7);
    h.record_coalesced(dummy_snapshot(1.0), key.clone(), label(1.0));
    assert!(h.try_extend_coalesced(&key), "the run is live");

    h.begin_compound();
    assert!(
        !h.try_extend_coalesced(&key),
        "the group's opening edit starts fresh"
    );
    h.record(dummy_snapshot(2.0), label(2.0));
    h.end_compound();

    // The group's entry recorded plain, so the resumed drag cannot
    // extend into it.
    assert!(!h.try_extend_coalesced(&key));
    assert_eq!(h.test_undo_entries().len(), 2);
}

#[test]
fn clear_empties_everything() {
    let mut h = UndoHistory::new();
    h.record(dummy_snapshot(1.0), label(1.0));
    h.begin(dummy_snapshot(2.0), label(2.0));
    let (snap, snap_label) = h.pop_undo().unwrap();
    h.push_redo(snap, snap_label);
    h.clear();
    assert!(!h.can_undo());
    assert!(!h.can_redo());
    assert!(!h.has_pending());
}

// ---------------------------------------------------------------------------
// Snapshot cost probe (ARCH-09)
// ---------------------------------------------------------------------------

/// Rough cost probe, not a benchmark: how long one undo snapshot of the
/// demo project takes, and how long the STATE-07 gesture-end check
/// (`commit_undo_gesture`'s "did anything change?") takes, with a 1 MiB
/// state blob parked on every demo plugin instance. Prints, never
/// asserts on time — run with `--nocapture` and read the numbers.
#[test]
fn snapshot_cost_probe_on_demo_project() {
    use std::hint::black_box;
    use std::time::Instant;

    let (mut app, _task, _rx) = resonance_app::Resonance::new_for_test_with_capture();
    resonance_app::demo::seed_demo_content(&mut app);
    let instance_ids: Vec<u64> = app
        .test_registry()
        .tracks
        .iter()
        .flat_map(|t| t.plugins.iter().map(|p| p.instance_id))
        .chain(
            app.test_registry()
                .busses
                .iter()
                .flat_map(|b| b.plugins.iter().map(|p| p.instance_id)),
        )
        .collect();
    for id in &instance_ids {
        app.test_seed_plugin_state(*id, vec![0xA5; 1 << 20]);
    }

    const N: u32 = 200;
    let t = Instant::now();
    for _ in 0..N {
        black_box(app.test_build_project_file());
    }
    let per_file = t.elapsed() / N;
    let file = app.test_build_project_file();
    let t = Instant::now();
    for _ in 0..N {
        black_box(serde_json::to_value(&file).unwrap());
    }
    let per_json = t.elapsed() / N;
    let t = Instant::now();
    for _ in 0..N {
        black_box(app.test_snapshot_for_undo());
    }
    let per_snapshot = t.elapsed() / N;

    let before = app.test_snapshot_for_undo();
    let t = Instant::now();
    for _ in 0..N {
        black_box(app.test_gesture_changed_since(&before));
    }
    let per_gesture_check = t.elapsed() / N;

    eprintln!(
        "snapshot cost probe: {} plugin blobs x 1 MiB, {} midi clips; \
         build_project_file = {per_file:?}, serde_json::to_value(file) = {per_json:?}, \
         snapshot_for_undo = {per_snapshot:?}, gesture-end change check = {per_gesture_check:?}",
        instance_ids.len(),
        app.test_midi_clips().len(),
    );
}

/// Consecutive snapshots share a plugin's state blob instead of each
/// carrying its own copy (ARCH-09 A9-2): with 200 retained entries and
/// KB–MB NAM/IR/wavetable blobs that is the difference between the
/// history costing Σ(blobs) and 200 × Σ(blobs).
#[test]
fn consecutive_snapshots_share_plugin_state_blobs() {
    use std::sync::Arc;

    let (mut app, _task, _rx) = resonance_app::Resonance::new_for_test_with_capture();
    resonance_app::demo::seed_demo_content(&mut app);
    let track = app.test_registry().tracks[0].id;
    let instance = app.test_track_plugin_instance_ids(track)[0];
    app.test_seed_plugin_state(instance, vec![0x5A; 1 << 20]);

    let a = app.test_snapshot_for_undo();
    // An edit between the two snapshots; the blob is untouched by it.
    app.test_dispatch(resonance_app::message::Message::Transport(
        resonance_app::message::TransportMessage::ToggleMetronome,
    ));
    let b = app.test_snapshot_for_undo();

    let (blob_a, blob_b) = (&a.project.plugin_states[&instance], &b.project.plugin_states[&instance]);
    assert_eq!(blob_a.len(), 1 << 20);
    assert!(
        Arc::ptr_eq(blob_a, blob_b),
        "two snapshots of an unchanged plugin must point at the same blob"
    );
}
