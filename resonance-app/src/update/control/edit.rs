//! `edit.*` control methods — undo / redo (ba doc #273, todo #1196).
//!
//! Control edits were already undoable: every mutating handler routes
//! its synthesized domain message through [`super::run_via_update`],
//! which runs the full update path including `record_undo`. This exposes
//! the app's EXISTING stack — `Resonance::try_undo` / `try_redo` — it
//! does not build a parallel history.
//!
//! `edit.status` reads only, but it describes the open project's
//! history, so like `master.summary` it sits below the mutation gate and
//! answers a stable `busy` with no project.

use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::edit::{self, EditStatus, RedoResult, UndoResult};
use resonance_control::{Request, Response, RpcError};

/// Handle an `edit.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        edit::STATUS => (super::success(request, &status(app)), Task::none()),
        edit::UNDO => undo(app, request),
        edit::REDO => redo(app, request),
        _ => return None,
    };
    Some(out)
}

/// The history's two ends, without touching it.
fn status(app: &Resonance) -> EditStatus {
    let undo_label = app.undo.undo_label().map(str::to_owned);
    let redo_label = app.undo.redo_label().map(str::to_owned);
    EditStatus {
        can_undo: undo_label.is_some(),
        can_redo: redo_label.is_some(),
        undo_label,
        redo_label,
    }
}

/// Why the app cannot undo/redo right now even though the stack is not
/// empty. The mutation gate has already ruled out "no project", an
/// offline bounce and a freeze render; what remains is a recording pass
/// and a half-finished drag gesture, both of which a restore would
/// silently discard.
fn blocked(app: &Resonance) -> Option<RpcError> {
    if app.transport.recording {
        return Some(RpcError::busy(
            "a recording is in progress; stop the transport before undoing",
        ));
    }
    if app.undo.has_pending() {
        return Some(RpcError::busy(
            "an edit gesture is still in progress; retry once it finishes",
        ));
    }
    None
}

fn undo(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    if app.undo.can_undo() {
        if let Some(error) = blocked(app) {
            return (super::failure(request, error), Task::none());
        }
    }
    // An empty history is a clean no-op, not an error: "there was
    // nothing to undo" is an answer.
    let undone = app.try_undo();
    let result = UndoResult {
        undone,
        status: status(app),
        revision: app.revision(),
    };
    (super::success(request, &result), Task::none())
}

fn redo(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    if app.undo.can_redo() {
        if let Some(error) = blocked(app) {
            return (super::failure(request, error), Task::none());
        }
    }
    let redone = app.try_redo();
    let result = RedoResult {
        redone,
        status: status(app),
        revision: app.revision(),
    };
    (super::success(request, &result), Task::none())
}
