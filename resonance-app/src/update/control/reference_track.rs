//! `reference.load` — put a pooled asset on the project's A/B reference
//! list (warmth-width-depth.md §7.5).
//!
//! It is the GUI's own reference load
//! ([`ReferenceMessage::LoadRequested`]) routed through the full update
//! path, so it lands in the undo history, is saved with the project and
//! shows in the reference panel like a load the user made. The file it
//! loads is the asset's POOLED copy — the project-rate WAV a clip placed
//! from the asset plays — so a reference measures exactly like that clip
//! (`meter.measure {target: {reference}}`, see `meter.rs`).

use crate::message::Message;
use crate::reference::ReferenceMessage;
use crate::Resonance;
use iced::Task;
use resonance_control::ids::ReferenceId;
use resonance_control::methods::reference::{self as proto, LoadParams, LoadResult};
use resonance_control::{Request, Response, RpcError};

/// Handle a `reference.*` request, or `None` for another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    match request.method.as_str() {
        proto::LOAD => Some(load(app, request)),
        _ => None,
    }
}

fn load(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: LoadParams = match request.params() {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    let (path, name) = match super::assist::pooled_asset_file(app, params.pool_asset_id.0) {
        Ok(found) => found,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    let path_text = path.to_string_lossy().into_owned();
    let task = super::run_via_update(
        app,
        Message::Reference(ReferenceMessage::LoadRequested(path)),
    );
    // The load lists the reference at once (analysing), last.
    let Some(entry) = app
        .reference
        .entries
        .iter_mut()
        .rev()
        .find(|e| e.path == path_text)
    else {
        return (
            super::failure(
                request,
                RpcError::internal("the reference load was not recorded"),
            ),
            task,
        );
    };
    // List it under the asset's name, not the pooled file's `asset_N`,
    // now and once the engine's echo lands.
    entry.name = name.clone();
    let id = entry.id;
    app.control.reference_names.insert(id, name.clone());
    let result = LoadResult {
        reference_id: ReferenceId(u64::from(id.0)),
        name,
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}
