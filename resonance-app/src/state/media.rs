//! Media-import/browse state (ARCH-06 second tier, A-12e): the media
//! pool, in-flight import/transcode bookkeeping, the docked browser panel,
//! drag-to-timeline placement, and the missing-file relink flow — doc
//! #175's whole media pipeline, previously seven loose fields on
//! [`Resonance`](crate::Resonance).
//!
//! Grouped together per the A-12 survey
//! (`docs/design/A-12-resonance-fields.md`): every field here is already
//! doc-#175 media/browse state, and several of their own doc comments
//! already say so explicitly ("same rule as collapse state", "the twin of
//! `RelinkState`"). Only `pool` and `pool_import` persist / undo; the rest
//! are transient session state, same as before this fold — grouping them
//! doesn't change what's durable, only where the fields live.

use crate::state;

/// Media pool, import bookkeeping, the docked browser panel, an in-flight
/// drag-to-timeline placement, and the missing-file relink flow.
#[derive(Debug, Clone, Default)]
pub struct MediaState {
    /// Media pool: imported audio assets referenced by clips, plus the
    /// browser's favourite / recent folder lists (doc #175). Asset list
    /// and clip asset-refs persist in the project file; favourites and
    /// recent folders persist in user settings. See `state::pool`.
    pub pool: state::MediaPool,
    /// In-flight import -> placement bookkeeping (doc #175, ba todo #598):
    /// per queued source file, what to do once its `AssetImported` event
    /// lands (place a clip on a target track, or nothing for a pool-only
    /// import). Transient — not persisted, not in the undo snapshot; the
    /// resulting pool asset + placed clip are what ride persistence/undo.
    pub pool_import: state::PendingImports,
    /// Per-file import-progress tracking for the audio-import transcode
    /// modal (doc #175, ba todo #597 / #606). Populated from
    /// `ImportProgress` / `ImportFailed` engine events; cleared when the
    /// modal is dismissed. Transient — not undoable, not persisted.
    pub import_progress: state::ImportProgressTracker,
    /// Whether the audio-import transcode-progress modal is open (doc #175,
    /// ba todo #606). Set to `true` when an import batch is kicked off and
    /// cleared by `UiMessage::DismissImportProgress`. Transient — not
    /// undoable, not persisted.
    pub import_progress_modal_open: bool,
    /// Transient media-browser interaction state (doc #175): current
    /// folder + cached scan, per-folder filter, Files/Pool tab, and the
    /// audition preview transport. Not undoable, not persisted in the
    /// project — same rule as collapse state. See `state::browser`.
    pub browser: state::BrowserState,
    /// In-flight drag-to-timeline placement (doc #175, todo #605): the file
    /// being dragged from the media browser, the cursor, and the resolved
    /// drop target driving the pill / lit lane / ghost clip / tooltip.
    /// `None` when no drag is happening. Transient — never undoable, never
    /// persisted; the drop itself fans out into a `Pool(ImportAndPlace)`.
    /// See `state::drag`.
    pub drag_placement: Option<state::DragPlacement>,
    /// Session-level state for the missing-file relink flow (doc #175,
    /// todo #600): which missing assets are currently being re-imported,
    /// plus the last relink failure to surface. The durable "missing" flag
    /// lives on each pool asset; this only tracks the in-flight resolve.
    /// Not undoable, not persisted. See `state::relink`.
    pub relink: state::RelinkState,
}
