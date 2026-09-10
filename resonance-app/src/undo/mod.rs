//! Session-local undo/redo history.
//!
//! Each undoable action snapshots the declarative project state using the
//! same shape that save/load already understands (`ProjectFile` plus
//! in-memory MIDI notes and cached plugin state blobs). On undo/redo the
//! audio engine is driven back into sync through `replay_loaded_project`,
//! the exact same code path used when opening a project from disk.
//!
//! Phase 1 scope: data types, a bounded history stack, and a snapshot
//! builder on `Resonance`. Message interception, keyboard shortcuts, and
//! the restore path are added in later phases.
//!
//! # Module layout
//!
//! | Sub-module | Contents |
//! |---|---|
//! | [`snapshot`] | Capture types (`ClipFadeGain`, `UndoExtras`, `UndoSnapshot`, `CoalesceKey`) and the `Resonance` methods that build and apply them |
//! | [`history`] | The bounded `UndoHistory` stack with transaction + coalesce logic |
//! | [`classify`] | `UndoAction` enum and the `classify` function that maps messages to actions |
//! | [`describe`] | Short human labels for history entries, reported by `edit.status` (todo #1196) |

pub use resonance_audio::DEFAULT_HISTORY_CAPACITY;

pub mod classify;
pub mod describe;
pub mod history;
pub mod snapshot;

pub use classify::{classify, UndoAction};
pub use describe::describe;
pub use history::UndoHistory;
pub use snapshot::{ClipFadeGain, CoalesceKey, UndoExtras, UndoSnapshot};
pub(crate) use snapshot::restore_arrangements;

// -------------------------------------------------------------------------
// Record-undo dispatch — bridges message classification (classify.rs),
// snapshot building (snapshot.rs), and the history stack (history.rs).
// -------------------------------------------------------------------------

impl crate::Resonance {
    /// Run the undo-history side effects for a single message dispatch.
    /// Classifies the message, marks the project dirty when appropriate,
    /// and captures a pre-dispatch snapshot for the Record / RecordCoalesced
    /// / Begin actions. Returns `true` when the caller must call
    /// `self.undo.commit()` after dispatch — i.e. when the message is a
    /// gesture-end that closes a transaction opened by an earlier `Begin`.
    pub(crate) fn record_undo(&mut self, message: &crate::message::Message) -> bool {
        let action = classify(message);
        let commit_after = matches!(action, UndoAction::Commit);

        // Mark the project dirty on any state-changing action. This
        // mirrors the undo classification: any action that warrants an
        // undo entry (Record, RecordCoalesced, Begin, Commit) means the
        // project has diverged from the last saved version. The dirty
        // flag is cleared on ProjectSaved(Ok) and on project load.
        if !matches!(action, UndoAction::Skip) {
            self.dirty = true;
        }

        // Bump the control-protocol revision counter (doc #265, todo
        // #1147) once per committed undoable change: immediate records,
        // each coalesced step (every one is a committed state change,
        // even when it merges into one undo entry), and the Commit that
        // closes a Begin…Commit gesture. Begin itself doesn't bump — the
        // transaction commits on gesture end. Unlike the history stack
        // this is not gated on `can_record_undo`: the state mutation
        // happens regardless, and remote clients need to see it.
        //
        // Inside a compound group (`with_compound_undo` — the control
        // API's per-call atomicity) only the group's opening mutation
        // bumps; the rest are absorbed into it, so a multi-dispatch
        // control call is one revision on the wire, matching the one
        // history entry it records below.
        let absorbed = match action {
            UndoAction::Record | UndoAction::RecordCoalesced(_) | UndoAction::Commit => {
                let absorbed = self.undo.absorb_into_compound();
                if !absorbed {
                    self.revision = self.revision.wrapping_add(1);
                }
                absorbed
            }
            UndoAction::Begin | UndoAction::Skip => false,
        };

        // Skip every history-mutating branch when the app isn't in a
        // state where a snapshot could be restored (no active project,
        // no saved path, mid-restore). Commit still runs on gesture end
        // even if recording was blocked — it'll be a no-op because
        // `begin` was also blocked, so there's no pending transaction.
        if self.can_record_undo() {
            match action {
                UndoAction::Skip | UndoAction::Commit => {}
                // Absorbed into the open compound group: its opening
                // entry already snapshots the pre-call state, so
                // recording again would split the call across history
                // entries. Skipping the snapshot build entirely is also
                // the cheap path — same reasoning as the coalesce check
                // below.
                UndoAction::Record | UndoAction::RecordCoalesced(_) if absorbed => {}
                UndoAction::Record => {
                    let snap = self.snapshot_for_undo();
                    self.undo.record(snap, describe(message));
                }
                UndoAction::RecordCoalesced(_) if self.undo.in_compound() => {
                    // The group's opening edit records PLAIN, never as a
                    // coalesce run: a later GUI drag on the same control
                    // must not merge into this call's entry, and
                    // `begin_compound` already broke any preceding run.
                    let snap = self.snapshot_for_undo();
                    self.undo.record(snap, describe(message));
                }
                UndoAction::RecordCoalesced(key) => {
                    // Check the coalesce run before building the snapshot:
                    // a continuing run keeps the run-opening snapshot, so
                    // building one here would deep-copy the whole project
                    // once per slider event only to drop it.
                    if !self.undo.try_extend_coalesced(&key) {
                        let snap = self.snapshot_for_undo();
                        self.undo.record_coalesced(snap, key, describe(message));
                    }
                }
                UndoAction::Begin => {
                    let snap = self.snapshot_for_undo();
                    self.undo.begin(snap, describe(message));
                }
            }
        }

        commit_after
    }

    /// Run `f` with the undo history in a compound group: every
    /// undoable dispatch inside `f` lands in ONE history entry — the
    /// first dispatch snapshots the pre-call state and bumps the
    /// control revision, the rest are absorbed. This is the control
    /// API's per-call atomicity (doc #265's revision contract): a
    /// multi-dispatch handler wraps its dispatches so `edit.undo`
    /// takes back the whole call, `edit.redo` replays it to its final
    /// state (redo snapshots lazily at undo time, so it naturally
    /// captures the state after the last sub-edit), and a remote
    /// client sees exactly one revision bump for its one call.
    ///
    /// Defaults off — GUI paths never open a group, so coalesced drags
    /// and Begin/Commit gestures are untouched. Groups do not nest
    /// (debug-asserted): a group opens and closes synchronously within
    /// a single control dispatch on the update loop, so no other
    /// message can interleave. A nested `update` triggered by a
    /// wrapped dispatch is absorbed into the same group — which is
    /// exactly the atomicity the group exists to provide.
    ///
    /// Mid-group failure: when `f` bails out after some dispatches,
    /// the group still closes and whatever applied stays behind as one
    /// undoable entry (with its one revision bump); the caller's error
    /// reply says what happened, and one `edit.undo` takes the partial
    /// edit back.
    pub fn with_compound_undo<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.undo.begin_compound();
        let out = f(self);
        self.undo.end_compound();
        out
    }
}
