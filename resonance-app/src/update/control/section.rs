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

use crate::compose::{ComposeMessage, LaneGeneratorConfig, LaneGeneratorKind};
use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::section as proto;
use resonance_control::{KeyScale, Request, Response, RpcError};
use resonance_music_theory::{
    parse_chord, BassParams, ChordQuality, MelodyParams, Mode, PadParams, PitchClass, Scale,
    VocalParams,
};

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
        proto::SET_LANE_GENERATOR => set_lane_generator(app, request),
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

/// `section.set_lane_generator` (ba doc #268): configure — or with
/// `manual`, clear — the generator on a `(section definition, track)`
/// lane. Idempotent, and unlike `generate.part` it neither requires
/// chords nor derives any MIDI; it is the only way to install the Vocal
/// lane the whole `vocal.*` namespace requires.
fn set_lane_generator(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::SetLaneGeneratorParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    if let Some(e) = definition_missing(app, definition_id) {
        return fail(request, e);
    }
    let track_id: u64 = params.track_id.into();
    // Kind decides the track a lane may live on: vocal generators only
    // on a vocal track, the melodic kinds only on a synth instrument
    // track. Manual only needs the track to exist — it is a removal.
    if let Some(e) = match params.kind {
        proto::LaneKind::Manual => super::generate::require_track(app, track_id),
        proto::LaneKind::Vocal => super::generate::require_vocal_track(app, track_id),
        _ => super::generate::require_instrument_track(app, track_id),
    } {
        return fail(request, e);
    }

    let config = match build_lane_config(definition_id, &params) {
        Ok(c) => c,
        Err(e) => return fail(request, e),
    };
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::SetLaneGenerator {
            definition_id,
            track_id,
            config: config.map(Box::new),
        }),
    );
    if let Some(error) = take_compose_error(app) {
        return fail(request, RpcError::invalid_params(error));
    }
    let result = proto::SetLaneGeneratorResult {
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

/// Build the lane config for `section.set_lane_generator`, or `None` for
/// `manual` (which removes the lane's generator).
///
/// The seed is explicit for reproducibility, else the GUI's
/// deterministic per-kind default derived from the section id — the same
/// multipliers `lane_inspector::set_generator` uses, so a lane installed
/// over the wire regenerates identically to one installed by hand.
fn build_lane_config(
    definition_id: u64,
    params: &proto::SetLaneGeneratorParams,
) -> Result<Option<LaneGeneratorConfig>, RpcError> {
    use crate::util::seed_from_id;
    let options = params.options.as_ref();
    let (kind, default_seed) = match params.kind {
        proto::LaneKind::Manual => return Ok(None),
        proto::LaneKind::Bass => (
            LaneGeneratorKind::Bass(parse_options::<BassParams>(params.kind, options)?),
            seed_from_id(definition_id),
        ),
        proto::LaneKind::Melody => (
            LaneGeneratorKind::Melody(parse_options::<MelodyParams>(params.kind, options)?),
            definition_id.wrapping_mul(0x517CC1B727220A95),
        ),
        proto::LaneKind::Pad => (
            LaneGeneratorKind::Pad(parse_options::<PadParams>(params.kind, options)?),
            definition_id.wrapping_mul(0x6C62272E07BB0142),
        ),
        proto::LaneKind::Vocal => (
            LaneGeneratorKind::Vocal(parse_options::<VocalParams>(params.kind, options)?),
            definition_id.wrapping_mul(0xBF58476D1CE4E5B9),
        ),
    };
    Ok(Some(LaneGeneratorConfig {
        kind,
        seed: params.seed.unwrap_or(default_seed),
    }))
}

/// Deserialize the optional per-kind `options` object into the matching
/// params struct; absent/null uses the generator defaults.
fn parse_options<T: serde::de::DeserializeOwned + Default>(
    kind: proto::LaneKind,
    options: Option<&serde_json::Value>,
) -> Result<T, RpcError> {
    match options {
        None | Some(serde_json::Value::Null) => Ok(T::default()),
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
            RpcError::invalid_params(format!("invalid options for kind {kind:?}: {e}"))
        }),
    }
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
