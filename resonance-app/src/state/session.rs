//! Session-meta bookkeeping (ARCH-06 second tier, A-12g survey): is the
//! project modified, the monotonic edit counter, the per-process session
//! id, and the undo/redo history that drives the first two.
//!
//! Grouped together per the A-12 survey (`docs/design/A-12-resonance-fields.md`),
//! with one adjustment. The survey's `SessionMetaState` proposal also
//! included `control: ControlEndpointState` (the unix-socket control
//! endpoint), but that field is left on [`Resonance`](crate::Resonance)
//! instead:
//!
//! - it is a different domain — a network/RPC transport endpoint (listener
//!   lifecycle, per-connection handshake state, the async job board), not
//!   part of tracking *this session's edits*;
//! - `ControlEndpointState` has its own `sessions: HashMap<ConnId,
//!   ControlSession>` field (per-connection sessions), which would read as
//!   `r.session.control.sessions` — a confusing stutter of "session"
//!   meaning two unrelated things one level apart;
//! - it holds a `ControlServer` / `Arc<JobBoard>` and derives only
//!   `Default` (no `Debug`), so folding it in here would also cost this
//!   struct its `Debug` derive.
//!
//! `dirty`, `revision` and `undo` *are* cohesive on their own merits, not
//! just to hit the field-count goal: every place that bumps the revision
//! counter (`undo/mod.rs`, `undo/snapshot.rs`) also sets `dirty` and
//! touches `undo` in the same breath — they are one piece of bookkeeping
//! updated together. `session_id` is unrelated in behaviour (it only
//! namespaces the autosave scratch dir) but small enough that a
//! standalone field for it isn't worth keeping.
//!
//! No `Clone` / `Default`: [`crate::undo::UndoHistory`] is `Debug, Default`
//! only, and its derived `Default` doesn't match its own `new()` (capacity
//! defaults to `0`, not `DEFAULT_HISTORY_CAPACITY`) — deriving `Default`
//! here would silently inherit that trap. `Resonance` itself was never
//! `Clone` or `Default` either.

use crate::undo::UndoHistory;

/// Session-level edit-tracking bookkeeping: is the project modified, the
/// monotonic edit counter, the per-process session id, and the undo/redo
/// history.
#[derive(Debug)]
pub struct SessionMetaState {
    /// True when the project has been modified since the last save.
    pub dirty: bool,
    /// Monotonic edit counter (doc #265, todo #1147): bumped once per
    /// committed undoable transaction via [`Resonance::bump_revision`]
    /// (immediate records, each coalesced step, gesture commits, and
    /// undo/redo restores). Every mutating control-protocol reply carries
    /// it so remote clients can detect concurrent GUI edits; it never
    /// resets while the app runs. Read through [`Resonance::revision`],
    /// never this field directly, outside `state::session` and `undo::*`.
    ///
    /// [`Resonance::bump_revision`]: crate::Resonance::bump_revision
    /// [`Resonance::revision`]: crate::Resonance::revision
    pub revision: u64,
    /// Stable per-process identifier (pid + startup timestamp). Used to
    /// namespace the autosave scratch dir for a never-saved project so
    /// concurrent app instances never collide (epic #32 / doc #171).
    pub session_id: String,
    /// Session-local undo/redo history. Cleared on project load.
    pub undo: UndoHistory,
}
