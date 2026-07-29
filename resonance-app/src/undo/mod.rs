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

pub use resonance_audio::DEFAULT_HISTORY_CAPACITY;

pub mod classify;
pub mod history;
pub mod snapshot;

pub use classify::{classify, UndoAction};
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
        match action {
            UndoAction::Record | UndoAction::RecordCoalesced(_) | UndoAction::Commit => {
                self.revision = self.revision.wrapping_add(1);
            }
            UndoAction::Begin | UndoAction::Skip => {}
        }

        // Skip every history-mutating branch when the app isn't in a
        // state where a snapshot could be restored (no active project,
        // no saved path, mid-restore). Commit still runs on gesture end
        // even if recording was blocked — it'll be a no-op because
        // `begin` was also blocked, so there's no pending transaction.
        if self.can_record_undo() {
            match action {
                UndoAction::Skip | UndoAction::Commit => {}
                UndoAction::Record => {
                    let snap = self.snapshot_for_undo();
                    self.undo.record(snap);
                }
                UndoAction::RecordCoalesced(key) => {
                    let snap = self.snapshot_for_undo();
                    self.undo.record_coalesced(snap, key);
                }
                UndoAction::Begin => {
                    let snap = self.snapshot_for_undo();
                    self.undo.begin(snap);
                }
            }
        }

        commit_after
    }
}
