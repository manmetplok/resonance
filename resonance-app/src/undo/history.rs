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
    /// Short human labels for what each stack entry would undo/redo,
    /// kept in lockstep with `undo` / `redo` (ba doc #273, todo #1196).
    /// A remote client must be able to see WHAT it is about to undo
    /// before undoing it — the stack is shared with the user's own GUI
    /// edits, so an undo can back out something they did.
    undo_labels: VecDeque<String>,
    redo_labels: VecDeque<String>,
    /// Snapshot captured at the start of an in-progress gesture. Committed
    /// to the undo stack on gesture end, discarded on cancel.
    pending: Option<UndoSnapshot>,
    /// Key of the most recently recorded entry, if it was recorded via
    /// `record_coalesced`. Cleared by every other history operation so
    /// any intervening action breaks a coalesce run.
    coalesce_key: Option<CoalesceKey>,
    /// Label for the snapshot in `pending`, committed with it.
    pending_label: String,
    /// Where the control layer's atomic compound group stands (one
    /// revision bump per mutating call). GUI paths never open one.
    compound: CompoundPhase,
    capacity: usize,
}

/// Phase of the control layer's compound group: while one is open, the
/// first recorded edit takes the snapshot ([`CompoundPhase::Open`] ->
/// [`CompoundPhase::Armed`]) and every later edit is absorbed into that
/// entry — no snapshot, no history entry, no revision bump.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CompoundPhase {
    /// No group open — every edit records individually (the GUI default).
    #[default]
    Closed,
    /// A group is open but nothing has recorded yet.
    Open,
    /// The group's opening edit has recorded; absorb the rest.
    Armed,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            undo_labels: VecDeque::new(),
            redo_labels: VecDeque::new(),
            pending: None,
            coalesce_key: None,
            pending_label: String::new(),
            compound: CompoundPhase::Closed,
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
    pub fn record(&mut self, snapshot: UndoSnapshot, label: String) {
        self.undo.push_back(snapshot);
        self.undo_labels.push_back(label);
        self.redo.clear();
        self.redo_labels.clear();
        self.trim();
        self.coalesce_key = None;
    }

    /// What the next undo would back out, if anything.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo_labels.back().map(String::as_str)
    }

    /// What the next redo would re-apply, if anything.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo_labels.back().map(String::as_str)
    }

    /// Continue an in-progress coalesce run without recording anything
    /// new. Returns `true` — after clearing the redo stack, exactly as a
    /// record would — when the last recorded entry was coalesced under
    /// `key`, so its snapshot (the pre-burst state) already covers this
    /// edit. Returns `false`, touching nothing, when the run is broken
    /// and the caller must record with a fresh snapshot.
    ///
    /// Split out of [`record_coalesced`](Self::record_coalesced) so
    /// `record_undo` can check the run *before* building a snapshot:
    /// `snapshot_for_undo` deep-copies the whole project, and a fader
    /// drag delivers one coalesced message per slider event.
    pub fn try_extend_coalesced(&mut self, key: &CoalesceKey) -> bool {
        if self.coalesce_key.as_ref() == Some(key) && !self.undo.is_empty() {
            self.redo.clear();
            self.redo_labels.clear();
            true
        } else {
            false
        }
    }

    /// Record an entry that can coalesce with subsequent edits to the
    /// same control. If the last recorded entry was also coalesced under
    /// `key`, this call keeps the existing snapshot (which already
    /// represents the pre-burst state) and only clears the redo stack.
    /// Otherwise a new entry is pushed and the key is remembered.
    pub fn record_coalesced(&mut self, snapshot: UndoSnapshot, key: CoalesceKey, label: String) {
        if self.try_extend_coalesced(&key) {
            return;
        }
        self.undo.push_back(snapshot);
        self.undo_labels.push_back(label);
        self.redo.clear();
        self.redo_labels.clear();
        self.trim();
        self.coalesce_key = Some(key);
    }

    /// Pop the newest undo entry. The caller is responsible for pushing
    /// the current state onto the redo stack via `push_redo` before
    /// restoring the popped snapshot.
    pub fn pop_undo(&mut self) -> Option<(UndoSnapshot, String)> {
        self.coalesce_key = None;
        let snapshot = self.undo.pop_back()?;
        let label = self.undo_labels.pop_back().unwrap_or_default();
        Some((snapshot, label))
    }

    /// Pop the newest redo entry. The caller is responsible for pushing
    /// the current state onto the undo stack via `push_undo` before
    /// restoring the popped snapshot.
    pub fn pop_redo(&mut self) -> Option<(UndoSnapshot, String)> {
        self.coalesce_key = None;
        let snapshot = self.redo.pop_back()?;
        let label = self.redo_labels.pop_back().unwrap_or_default();
        Some((snapshot, label))
    }

    /// Push a snapshot onto the redo stack without touching the undo stack.
    /// Used when entering an undo: current state goes to redo so it can be
    /// restored by a subsequent redo.
    pub fn push_redo(&mut self, snapshot: UndoSnapshot, label: String) {
        self.redo.push_back(snapshot);
        self.redo_labels.push_back(label);
        self.coalesce_key = None;
    }

    /// Push a snapshot onto the undo stack without clearing the redo stack.
    /// Used when entering a redo: current state goes to undo so it can be
    /// restored by a subsequent undo.
    pub fn push_undo(&mut self, snapshot: UndoSnapshot, label: String) {
        self.undo.push_back(snapshot);
        self.undo_labels.push_back(label);
        self.trim();
        self.coalesce_key = None;
    }

    /// End any in-progress coalesce run, so the next coalesced record
    /// starts a fresh entry.
    pub fn break_coalesce(&mut self) {
        self.coalesce_key = None;
    }

    // -- Transaction API for multi-message gestures --------------------

    /// Open a transaction. Used at the start of a drag / trim gesture;
    /// the captured snapshot represents the state before the gesture.
    pub fn begin(&mut self, snapshot: UndoSnapshot, label: String) {
        // A drag gesture cannot start inside a compound group: groups
        // open and close synchronously within one control dispatch, and
        // control handlers never dispatch gesture-start messages.
        debug_assert!(
            !self.in_compound(),
            "a Begin/Commit gesture cannot start inside a compound group"
        );
        self.pending = Some(snapshot);
        self.pending_label = label;
        self.coalesce_key = None;
    }

    /// The snapshot of an open gesture (its pre-gesture state), if any.
    /// `commit_undo_gesture` compares it against the current state to tell
    /// a real edit from a click that moved nothing (code review STATE-07).
    pub fn pending_snapshot(&self) -> Option<&UndoSnapshot> {
        self.pending.as_ref()
    }

    /// Drop the open gesture without recording it: the gesture ended
    /// without changing anything, so there is nothing to undo, and the
    /// redo stack must survive.
    pub fn discard_pending(&mut self) {
        self.pending = None;
        self.pending_label.clear();
    }

    /// Commit the pending transaction as a single undo entry. Called at
    /// gesture end when the state actually changed.
    pub fn commit(&mut self) {
        if let Some(snap) = self.pending.take() {
            let label = std::mem::take(&mut self.pending_label);
            self.record(snap, label);
        }
    }

    // -- Compound-group API for multi-dispatch control calls -----------
    //
    // The wire contract promises one revision bump per mutating control
    // call, but several handlers dispatch more than one undoable message
    // per call (`notes.edit` fans a multi-field change out, `clip.set_fade`
    // sets two amounts and two shapes, ...). A compound group makes those
    // dispatches ONE undoable transaction: the first recorded edit inside
    // the group snapshots the pre-call state and becomes the call's single
    // history entry; every later edit is absorbed — no snapshot, no entry.
    // The redo state needs no finalizing because redo is captured lazily:
    // `try_undo` snapshots the current (post-group) state when the entry
    // is undone.

    /// Open a compound group. Breaks any in-progress coalesce run, so
    /// the group's opening edit starts a fresh entry instead of merging
    /// into a preceding fader burst. Groups do not nest — a group opens
    /// and closes synchronously within one control dispatch on the
    /// update loop, so nothing can interleave.
    pub fn begin_compound(&mut self) {
        debug_assert!(!self.in_compound(), "compound groups do not nest");
        self.coalesce_key = None;
        self.compound = CompoundPhase::Open;
    }

    /// Close the compound group opened by [`begin_compound`](Self::begin_compound).
    /// A group that recorded nothing leaves the history untouched.
    pub fn end_compound(&mut self) {
        // Nothing inside a group records via the coalesce path (the
        // opening edit records plain), so no run can leak out of it.
        debug_assert!(
            self.coalesce_key.is_none(),
            "a coalesce run cannot open inside a compound group"
        );
        self.compound = CompoundPhase::Closed;
    }

    /// True while a compound group is open.
    pub fn in_compound(&self) -> bool {
        self.compound != CompoundPhase::Closed
    }

    /// Whether the mutation being classified is absorbed by the open
    /// compound group. The group's first mutation arms it and returns
    /// `false` — that one records (and bumps the revision) normally;
    /// every later call returns `true` and the caller records nothing.
    /// Always `false` with no group open. Called unconditionally by
    /// `record_undo` — even when the history itself cannot record — so
    /// the one-revision-bump-per-call contract holds regardless of
    /// `can_record_undo`.
    pub fn absorb_into_compound(&mut self) -> bool {
        match self.compound {
            CompoundPhase::Closed => false,
            CompoundPhase::Open => {
                self.compound = CompoundPhase::Armed;
                false
            }
            CompoundPhase::Armed => true,
        }
    }

    /// Drop the entire history. Called when a new project is loaded —
    /// undo history does not cross the load boundary.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_labels.clear();
        self.redo_labels.clear();
        self.pending = None;
        self.pending_label = String::new();
        self.coalesce_key = None;
        self.compound = CompoundPhase::Closed;
    }

    fn trim(&mut self) {
        while self.undo.len() > self.capacity {
            self.undo.pop_front();
            self.undo_labels.pop_front();
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
