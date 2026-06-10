//! Unit coverage for `UndoHistory`: capacity trimming, redo
//! invalidation, transaction commit, and coalesce-key behaviour.
//!
//! Moved out of an inline `#[cfg(test)]` module in `src/undo.rs` once
//! the crate grew a `lib.rs`. The private `capacity` / `undo` fields
//! are reached through the `#[doc(hidden)]` `test_set_capacity` /
//! `test_undo_entries` accessors on `UndoHistory`.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::project::{ProjectFile, PROJECT_FORMAT_VERSION};
use resonance_app::undo::{CoalesceKey, UndoExtras, UndoHistory, UndoSnapshot};

/// Produce a snapshot that carries `id` in its `file.bpm` field so
/// tests can distinguish snapshots on the history stack. `bpm` is
/// abused purely as a numeric discriminator here; the rest of the
/// snapshot is a valid default.
fn dummy_snapshot(id: f32) -> UndoSnapshot {
    UndoSnapshot {
        file: ProjectFile {
            version: PROJECT_FORMAT_VERSION,
            sample_rate: 44100,
            bpm: id,
            time_sig_num: 4,
            time_sig_den: 4,
            metronome_enabled: false,
            master_volume: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            tracks: Vec::new(),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            busses: Vec::new(),
            section_definitions: Vec::new(),
            section_placements: Vec::new(),
            tempo_events: Vec::new(),
            signature_events: Vec::new(),
            midi_clock_send_enabled: false,
            midi_clock_send_device: None,
            midi_clock_recv_enabled: false,
            midi_clock_recv_device: None,
            drum_groups: Vec::new(),
            drum_patterns: Vec::new(),
        },
        project_dir: PathBuf::new(),
        midi_notes: HashMap::new(),
        plugin_states: HashMap::new(),
        extras: UndoExtras::default(),
    }
}

#[test]
fn record_clears_redo() {
    let mut h = UndoHistory::new();
    h.record(dummy_snapshot(1.0));
    // Simulate an undo: redo now has one entry.
    let popped = h.pop_undo().unwrap();
    h.push_redo(popped);
    assert!(h.can_redo());
    // Recording a new action must wipe redo.
    h.record(dummy_snapshot(2.0));
    assert!(!h.can_redo());
}

#[test]
fn capacity_trims_oldest() {
    let mut h = UndoHistory::new();
    h.test_set_capacity(3);
    h.record(dummy_snapshot(1.0));
    h.record(dummy_snapshot(2.0));
    h.record(dummy_snapshot(3.0));
    h.record(dummy_snapshot(4.0));
    assert_eq!(h.test_undo_entries().len(), 3);
    // The oldest (1.0) should have been trimmed; top of stack is 4.0.
    assert_eq!(h.pop_undo().unwrap().file.bpm, 4.0);
    assert_eq!(h.pop_undo().unwrap().file.bpm, 3.0);
    assert_eq!(h.pop_undo().unwrap().file.bpm, 2.0);
    assert!(h.pop_undo().is_none());
}

#[test]
fn commit_records_pending_transaction() {
    let mut h = UndoHistory::new();
    h.begin(dummy_snapshot(1.0));
    assert!(h.has_pending());
    h.commit();
    assert!(!h.has_pending());
    assert!(h.can_undo());
    assert_eq!(h.test_undo_entries()[0].file.bpm, 1.0);
}

#[test]
fn coalesces_same_key_and_breaks_on_intervening_action() {
    let mut h = UndoHistory::new();
    let key = CoalesceKey::TrackVolume(7);

    // First entry under `key` pushes normally.
    h.record_coalesced(dummy_snapshot(1.0), key.clone());
    assert_eq!(h.test_undo_entries().len(), 1);
    // Subsequent entries under the same key do not push.
    h.record_coalesced(dummy_snapshot(2.0), key.clone());
    h.record_coalesced(dummy_snapshot(3.0), key.clone());
    assert_eq!(h.test_undo_entries().len(), 1);
    // The retained entry is the original (pre-burst) snapshot.
    assert_eq!(h.test_undo_entries()[0].file.bpm, 1.0);

    // A different coalesce key breaks the run and pushes a new entry.
    h.record_coalesced(dummy_snapshot(10.0), CoalesceKey::TrackPan(7));
    assert_eq!(h.test_undo_entries().len(), 2);

    // An atomic record also breaks any subsequent coalesce run.
    h.record(dummy_snapshot(20.0));
    h.record_coalesced(dummy_snapshot(4.0), key);
    assert_eq!(h.test_undo_entries().len(), 4);
}

#[test]
fn coalesce_run_is_broken_by_pop() {
    let mut h = UndoHistory::new();
    let key = CoalesceKey::MasterVolume;
    h.record_coalesced(dummy_snapshot(1.0), key.clone());
    h.pop_undo();
    // After popping, the next coalesced record must push fresh.
    h.record_coalesced(dummy_snapshot(2.0), key);
    assert_eq!(h.test_undo_entries().len(), 1);
    assert_eq!(h.test_undo_entries()[0].file.bpm, 2.0);
}

#[test]
fn clear_empties_everything() {
    let mut h = UndoHistory::new();
    h.record(dummy_snapshot(1.0));
    h.begin(dummy_snapshot(2.0));
    let snap = h.pop_undo().unwrap();
    h.push_redo(snap);
    h.clear();
    assert!(!h.can_undo());
    assert!(!h.can_redo());
    assert!(!h.has_pending());
}
