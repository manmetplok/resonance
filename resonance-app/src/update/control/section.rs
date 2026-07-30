//! `section.*` control methods (ba doc #265, todo #1153): section
//! definitions and arrangement placements.
//!
//! Every mutation synthesizes the existing [`ComposeMessage`] values and
//! routes them through [`super::run_via_update`] — the full gates /
//! frozen-input / undo / dispatch path — so a remote edit is a normal,
//! undoable edit. Handlers pre-validate ids and params so failures come
//! back as precise JSON-RPC errors instead of the silent no-ops the GUI
//! handlers use (`compose.last_error` is drained defensively after every
//! dispatch either way).
//!
//! Wire conventions (doc #265): placement bars are 1-based on the wire,
//! 0-based in the app; `section.delete` is destructive and requires
//! `"confirm": true`.

use std::collections::HashSet;

use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::section as proto;
use resonance_control::{KeyScale, Request, Response, RpcError};
use resonance_music_theory::{parse_chord, ChordQuality, Mode, PitchClass, Scale};

pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::CREATE => create(app, request),
        proto::RENAME => rename(app, request),
        proto::RESIZE => resize(app, request),
        proto::DELETE => delete(app, request),
        proto::PLACE => place(app, request),
        proto::REMOVE_PLACEMENT => remove_placement(app, request),
        proto::SET_SCALE => set_scale(app, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn create(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::CreateParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let name = params.name.trim().to_owned();
    if name.is_empty() {
        return fail(request, RpcError::invalid_params("section name cannot be empty"));
    }
    if params.length_bars == 0 {
        return fail(
            request,
            RpcError::invalid_params("section length must be at least 1 bar"),
        );
    }
    let scale = match params.scale.as_ref().map(parse_key_scale).transpose() {
        Ok(s) => s,
        Err(e) => return fail(request, e),
    };

    let before: HashSet<u64> = app.compose.definitions.iter().map(|d| d.id).collect();
    let color = crate::update::compose::next_default_color(&app.compose);
    let mut task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::CreateSection {
            name,
            length_bars: params.length_bars,
            color,
            // Defaults to true (the historical implicit placement); a
            // client that wants to position sections itself passes
            // `place: false` and follows up with `section.place`
            // (ba doc #269 FR-6).
            place: params.place,
        }),
    );
    if let Some(error) = take_compose_error(app) {
        return fail(request, RpcError::invalid_params(error));
    }
    let Some(section_id) = app
        .compose
        .definitions
        .iter()
        .map(|d| d.id)
        .find(|id| !before.contains(id))
    else {
        return fail(request, RpcError::internal("section was not created"));
    };
    if let Some(scale) = scale {
        let set = super::run_via_update(
            app,
            Message::Compose(ComposeMessage::SetSectionScale {
                definition_id: section_id,
                scale: Some(scale),
            }),
        );
        task = Task::batch([task, set]);
    }
    let result = proto::CreateResult {
        section_id: section_id.into(),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn rename(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::RenameParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    if let Some(e) = definition_missing(app, params.section_id.into()) {
        return fail(request, e);
    }
    let name = params.name.trim().to_owned();
    if name.is_empty() {
        return fail(request, RpcError::invalid_params("section name cannot be empty"));
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::RenameSection {
            definition_id: params.section_id.into(),
            name,
        }),
    );
    ack_or_error(app, request, task)
}

fn resize(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::ResizeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    if let Some(e) = definition_missing(app, params.section_id.into()) {
        return fail(request, e);
    }
    if params.length_bars == 0 {
        return fail(
            request,
            RpcError::invalid_params("section length must be at least 1 bar"),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ResizeSection {
            definition_id: params.section_id.into(),
            length_bars: params.length_bars,
        }),
    );
    ack_or_error(app, request, task)
}

fn delete(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::DeleteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    let Some(def) = app.compose.find_definition(definition_id) else {
        return fail(
            request,
            RpcError::not_found(format!("no section definition with id {definition_id}")),
        );
    };
    if !params.confirm {
        let placements = app
            .compose
            .placements
            .iter()
            .filter(|p| p.definition_id == definition_id)
            .count();
        return fail(
            request,
            RpcError::needs_confirmation(format!(
                "deleting section {:?} removes {} arrangement placement(s), {} chord(s), and \
                 every generated lane for it; pass \"confirm\": true to proceed",
                def.name,
                placements,
                def.chords.len(),
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::DeleteSectionWithPlacements { definition_id }),
    );
    ack_or_error(app, request, task)
}

fn place(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::PlaceParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    if let Some(e) = definition_missing(app, params.definition_id.into()) {
        return fail(request, e);
    }
    // Wire bars are 1-based; the app's placement grid is 0-based.
    if params.start_bar == 0 {
        return fail(
            request,
            RpcError::invalid_params("start_bar is 1-based; bar 0 does not exist"),
        );
    }
    let before: HashSet<u64> = app.compose.placements.iter().map(|p| p.id).collect();
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::PlaceSection {
            definition_id: params.definition_id.into(),
            start_bar: params.start_bar - 1,
        }),
    );
    if let Some(error) = take_compose_error(app) {
        return fail(request, RpcError::invalid_params(error));
    }
    let Some(placement_id) = app
        .compose
        .placements
        .iter()
        .map(|p| p.id)
        .find(|id| !before.contains(id))
    else {
        return fail(request, RpcError::internal("placement was not created"));
    };
    let result = proto::PlaceResult {
        placement_id: placement_id.into(),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn remove_placement(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::RemovePlacementParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let placement_id: u64 = params.placement_id.into();
    if app.compose.find_placement(placement_id).is_none() {
        return fail(
            request,
            RpcError::not_found(format!("no section placement with id {placement_id}")),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::DeleteSectionPlacement { placement_id }),
    );
    ack_or_error(app, request, task)
}

fn set_scale(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::SetScaleParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    if let Some(e) = definition_missing(app, params.section_id.into()) {
        return fail(request, e);
    }
    let scale = match parse_key_scale(&params.scale) {
        Ok(s) => s,
        Err(e) => return fail(request, e),
    };
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::SetSectionScale {
            definition_id: params.section_id.into(),
            scale: Some(scale),
        }),
    );
    ack_or_error(app, request, task)
}

// ---------------------------------------------------------------------------
// Shared helpers (also used by the `harmony.*` sibling)
// ---------------------------------------------------------------------------

/// `not_found` when no section definition has `definition_id`.
pub(super) fn definition_missing(app: &Resonance, definition_id: u64) -> Option<RpcError> {
    if app.compose.find_definition(definition_id).is_none() {
        return Some(RpcError::not_found(format!(
            "no section definition with id {definition_id}"
        )));
    }
    None
}

/// Drain `compose.last_error` after a dispatch, so a validation failure
/// inside the GUI handler surfaces on the wire instead of lingering as
/// an error banner for an edit the GUI user never made.
pub(super) fn take_compose_error(app: &mut Resonance) -> Option<String> {
    app.compose.last_error.take()
}

/// A `MutationAck` reply, unless the dispatch left a compose error
/// behind — then that error, as `invalid_params`.
pub(super) fn ack_or_error(
    app: &mut Resonance,
    request: &Request,
    task: Task<Message>,
) -> (Response, Task<Message>) {
    if let Some(error) = take_compose_error(app) {
        return fail(request, RpcError::invalid_params(error));
    }
    (super::success(request, &super::mutation_ack(app)), task)
}

/// An error reply that still forwards the (usually empty) task.
pub(super) fn fail(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (super::failure(request, error), Task::none())
}

/// Parse the wire `{tonic, scale}` pair into the app's [`Scale`].
///
/// The tonic reuses the chord-symbol note parser (accepting `"A"`,
/// `"F#"`, `"Bb"`, unicode accidentals, ...); the scale name matches the
/// app's lowercase mode names, with `_`/`-` accepted for the spaces in
/// `"harmonic minor"` / `"melodic minor"`.
pub(super) fn parse_key_scale(key: &KeyScale) -> Result<Scale, RpcError> {
    Ok(Scale::new(parse_tonic(&key.tonic)?, parse_mode(&key.scale)?))
}

fn parse_tonic(tonic: &str) -> Result<PitchClass, RpcError> {
    match parse_chord(tonic) {
        Ok(c) if c.quality == ChordQuality::Maj && c.bass.is_none() => Ok(c.root),
        _ => Err(RpcError::invalid_params(format!(
            "invalid tonic {tonic:?} (expected a pitch name like \"A\" or \"F#\")"
        ))),
    }
}

fn parse_mode(name: &str) -> Result<Mode, RpcError> {
    let wanted = name.trim().to_ascii_lowercase().replace(['_', '-'], " ");
    Mode::ALL
        .iter()
        .copied()
        .find(|m| m.as_str() == wanted)
        .ok_or_else(|| {
            let known: Vec<&str> = Mode::ALL.iter().map(|m| m.as_str()).collect();
            RpcError::invalid_params(format!(
                "unknown scale {name:?} (one of: {})",
                known.join(", ")
            ))
        })
}
