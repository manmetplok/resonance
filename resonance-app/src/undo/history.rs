//! Bounded undo / redo stack with transaction and coalesce support.
//!
//! `UndoHistory` is the sole owner of the two `VecDeque` stacks plus the
//! pending-transaction slot. All mutation goes through its methods so
//! capacity trimming and coalesce-key bookkeeping stay in one place.

use std::collections::VecDeque;

use resonance_audio::DEFAULT_HISTORY_CAPACITY;

use super::snapshot::{CoalesceKey, UndoSnapshot};

/// Bounded undo / redo stack with a pending-transaction slot for
/// multi-message gestures (clip drag, trim, loop drag, MIDI note drag)
/// and a coalesce slot for knob/fader bursts.
#[derive(Debug, Default)]
pub struct UndoHistory {
    undo: VecDeque<UndoSnapshot>,
    redo: VecDeque<UndoSnapshot>,
    /// Snapshot captured at the start of an in-progress gesture. Committed
    /// to the undo stack on gesture end, discarded on cancel.
    pending: Option<UndoSnapshot>,
    /// Key of the most recently recorded entry, if it was recorded via
    /// `record_coalesced`. Cleared by every other history operation so
    /// any intervening action breaks a coalesce run.
    coalesce_key: Option<CoalesceKey>,
    capacity: usize,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            pending: None,
            coalesce_key: None,
            capacity: DEFAULT_HISTORY_CAPACITY,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Record a finished action. Clears the redo stack — any new mutation
    /// invalidates the redo history — and trims to `capacity`.
    pub fn record(&mut self, snapshot: UndoSnapshot) {
        self.undo.push_back(snapshot);
        self.redo.clear();
        self.trim();
        self.coalesce_key = None;
    }

    /// Record an entry that can coalesce with subsequent edits to the
    /// same control. If the last recorded entry was also coalesced under
    /// `key`, this call keeps the existing snapshot (which already
    /// represents the pre-burst state) and only clears the redo stack.
    /// Otherwise a new entry is pushed and the key is remembered.
    pub fn record_coalesced(&mut self, snapshot: UndoSnapshot, key: CoalesceKey) {
        if self.coalesce_key.as_ref() == Some(&key) && !self.undo.is_empty() {
            self.redo.clear();
            return;
        }
        self.undo.push_back(snapshot);
        self.redo.clear();
        self.trim();
        self.coalesce_key = Some(key);
    }

    /// Pop the newest undo entry. The caller is responsible for pushing
    /// the current state onto the redo stack via `push_redo` before
    /// restoring the popped snapshot.
    pub fn pop_undo(&mut self) -> Option<UndoSnapshot> {
        self.coalesce_key = None;
        self.undo.pop_back()
    }

    /// Pop the newest redo entry. The caller is responsible for pushing
    /// the current state onto the undo stack via `push_undo` before
    /// restoring the popped snapshot.
    pub fn pop_redo(&mut self) -> Option<UndoSnapshot> {
        self.coalesce_key = None;
        self.redo.pop_back()
    }

    /// Push a snapshot onto the redo stack without touching the undo stack.
    /// Used when entering an undo: current state goes to redo so it can be
    /// restored by a subsequent redo.
    pub fn push_redo(&mut self, snapshot: UndoSnapshot) {
        self.redo.push_back(snapshot);
        self.coalesce_key = None;
    }

    /// Push a snapshot onto the undo stack without clearing the redo stack.
    /// Used when entering a redo: current state goes to undo so it can be
    /// restored by a subsequent undo.
    pub fn push_undo(&mut self, snapshot: UndoSnapshot) {
        self.undo.push_back(snapshot);
        self.trim();
        self.coalesce_key = None;
    }

    // -- Transaction API for multi-message gestures --------------------

    /// Open a transaction. Used at the start of a drag / trim gesture;
    /// the captured snapshot represents the state before the gesture.
    pub fn begin(&mut self, snapshot: UndoSnapshot) {
        self.pending = Some(snapshot);
        self.coalesce_key = None;
    }

    /// Commit the pending transaction as a single undo entry. Called at
    /// gesture end when the state actually changed.
    pub fn commit(&mut self) {
        if let Some(snap) = self.pending.take() {
            self.record(snap);
        }
    }

    /// Drop the entire history. Called when a new project is loaded —
    /// undo history does not cross the load boundary.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
        self.coalesce_key = None;
    }

    fn trim(&mut self) {
        while self.undo.len() > self.capacity {
            self.undo.pop_front();
        }
    }

    // ---- Test-only accessors for integration tests -------------------
    //
    // `tests/undo_history.rs` verifies capacity trimming and coalesce-key
    // behaviour, which requires poking the private `capacity` / `undo`
    // fields. Same `#[doc(hidden)]` convention as the `test_*` accessors
    // on `Resonance` in `lib.rs`: not part of the user-facing surface,
    // crate-internal code keeps using the private fields directly.

    /// Test-only: override the history capacity so trimming is testable
    /// without recording `DEFAULT_HISTORY_CAPACITY` snapshots.
    #[doc(hidden)]
    pub fn test_set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
    }

    /// Test-only: read the undo stack (oldest first) so tests can assert
    /// entry counts and inspect retained snapshots.
    #[doc(hidden)]
    pub fn test_undo_entries(&self) -> &VecDeque<UndoSnapshot> {
        &self.undo
    }
}
