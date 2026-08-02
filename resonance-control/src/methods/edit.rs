//! `edit.*` — undo / redo over the control API.
//!
//! Control edits already land in the app's undo history exactly like
//! manual ones: every mutating handler routes its synthesized domain
//! message through the full update path, which runs `record_undo`. What
//! was missing is any way to pop one back off, so a mis-scoped edit
//! stranded the caller with no recovery path (ba doc #270 §10, doc
//! #273).
//!
//! The stack is **shared with the user's own GUI edits** — an undo may
//! back out something they did — which is why [`STATUS`] exists: a
//! client must be able to see what is on top before undoing it.

use serde::{Deserialize, Serialize};

/// `edit.undo` — undo the newest edit ([`UndoResult`]). No params.
pub const UNDO: &str = "edit.undo";
/// `edit.redo` — redo the newest undone edit ([`RedoResult`]). No params.
pub const REDO: &str = "edit.redo";
/// `edit.status` — what undo/redo would do, without doing it
/// ([`EditStatus`]). No params.
pub const STATUS: &str = "edit.status";

/// All `edit.*` method names.
pub const METHODS: &[&str] = &[UNDO, REDO, STATUS];

/// Result of `edit.status` — the history's two ends, so a client never
/// has to undo blindly to find out what is there.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct EditStatus {
    /// Short label for the edit `edit.undo` would back out, e.g.
    /// `"track volume"`. `null` when the history is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_label: Option<String>,
    /// Short label for the edit `edit.redo` would re-apply. `null` when
    /// nothing has been undone (or a new edit cleared the redo stack).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redo_label: Option<String>,
    /// `true` when [`Self::undo_label`] is present.
    pub can_undo: bool,
    /// `true` when [`Self::redo_label`] is present.
    pub can_redo: bool,
}

/// Result of `edit.undo`.
///
/// An empty history is a clean no-op — `undone: null`, `revision`
/// unchanged — never an error: "there was nothing to undo" is an answer,
/// not a failure.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct UndoResult {
    /// Label of the edit that was undone, or `null` if there was none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone: Option<String>,
    /// The history's state after this call.
    #[serde(flatten)]
    pub status: EditStatus,
    pub revision: u64,
}

/// Result of `edit.redo`; mirrors [`UndoResult`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RedoResult {
    /// Label of the edit that was re-applied, or `null` if there was
    /// nothing on the redo stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redone: Option<String>,
    #[serde(flatten)]
    pub status: EditStatus,
    pub revision: u64,
}
