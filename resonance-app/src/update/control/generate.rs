//! `generate.*` control methods (ba doc #265, todo #1154): run the
//! app's generators into a section + track.
//!
//! - `generate.part` installs a Bass / Melody / Pad lane generator on a
//!   track within a section and derives its MIDI onto every placement,
//!   via [`ComposeMessage::GenerateSectionPart`].
//! - `generate.drums` assigns a drum pattern to the section, re-seeds and
//!   generates its groups, and materialises the drum clips onto the
//!   project's drum tracks, via [`ComposeMessage::GenerateSectionDrums`].
//!
//! Both route their state mutation through [`super::run_via_update`] by
//! synthesizing a [`ComposeMessage`] carrier, so the edit is one
//! undoable transaction on the normal update path. Validation (section /
//! track existence and kind, presence of chords) happens here *before*
//! dispatch so failures come back as precise JSON-RPC errors and the
//! synthesized message only ever runs on valid input.
//!
//! The generators write **derived** MIDI clips — one per placement of
//! the target section, rebuilt wholesale on every regenerate rather than
//! edited in place. Their ids are nonetheless the ids `song.tracks` and
//! `song.notes` address, so [`GenerateResult`] reports them: `clip_ids`
//! in arrangement order, and `clip_id` as the first, which is the only
//! one for a section placed once. A caller no longer has to re-read
//! `song.tracks` just to find out where its own material landed.

use crate::compose::{ComposeMessage, LaneGeneratorConfig, LaneGeneratorKind};
use crate::message::Message;
use crate::state::InstrumentType;
use crate::update::compose::drum_groups::pattern_id_by_name;
use crate::util::seed_from_id;
use crate::Resonance;
use iced::Task;
use resonance_audio::types::TrackType;
use resonance_control::methods::generate::{self as proto, GenerateResult, GenerateRole};
use resonance_control::{Request, Response, RpcError};
use resonance_music_theory::{BassParams, MelodyParams, PadParams};

use super::section::fail;

pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::PART => part(app, request),
        proto::DRUMS => drums(app, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// generate.part
// ---------------------------------------------------------------------------

fn part(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::PartParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    if let Some(e) = super::section::definition_missing(app, definition_id) {
        return fail(request, e);
    }
    let track_id: u64 = params.track_id.into();
    // The melodic generators target a synth (instrument) track; a drum or
    // vocal track would be a category error (drums have their own method,
    // vocals their own namespace).
    if let Some(e) = require_instrument_track(app, track_id) {
        return fail(request, e);
    }
    // The generators read the section's chord grid; with no chords they
    // would silently produce nothing, so refuse up front.
    let has_chords = app
        .compose
        .find_definition(definition_id)
        .is_some_and(|d| !d.chords.is_empty());
    if !has_chords {
        return fail(
            request,
            RpcError::invalid_params(
                "section has no chords to generate from; add chords (harmony.*) first",
            ),
        );
    }

    // The lane seed: explicit for reproducibility, else derived from the
    // section id (matching the GUI's per-tag deterministic default).
    let seed = params.seed.unwrap_or_else(|| seed_from_id(definition_id));
    let kind = match build_kind(params.role, params.options.as_ref()) {
        Ok(k) => k,
        Err(e) => return fail(request, e),
    };
    let config = LaneGeneratorConfig { kind, seed };

    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::GenerateSectionPart {
            definition_id,
            track_id,
            config: Box::new(config),
        }),
    );
    if let Some(error) = app.compose.last_error.take() {
        return fail(request, RpcError::invalid_params(error));
    }
    let result = GenerateResult::new(generated_clips(app, definition_id, track_id), app.revision());
    (super::success(request, &result), task)
}

/// The derived clips holding this `(section, track)` lane's material,
/// one per placement of the section, ordered by the placement's start
/// bar. Read after the generate has run through the update path, which
/// repopulates the derived-clip map synchronously.
fn generated_clips(
    app: &Resonance,
    definition_id: u64,
    track_id: u64,
) -> Vec<resonance_control::ids::ClipId> {
    let mut placements: Vec<(u32, u64)> = app
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.start_bar, p.id))
        .collect();
    placements.sort_unstable();
    placements
        .iter()
        .filter_map(|(_, placement_id)| {
            app.compose
                .derived_clips
                .get(&(definition_id, *placement_id, track_id))
                .copied()
        })
        .map(resonance_control::ids::ClipId)
        .collect()
}

/// Build a melodic lane-generator kind from the wire role, deserializing
/// the optional per-role `options` object straight into the matching
/// params struct (all three derive `Deserialize`). Absent options use
/// the generator defaults.
fn build_kind(
    role: GenerateRole,
    options: Option<&serde_json::Value>,
) -> Result<LaneGeneratorKind, RpcError> {
    Ok(match role {
        GenerateRole::Bass => LaneGeneratorKind::Bass(parse_options::<BassParams>(role, options)?),
        GenerateRole::Lead => {
            LaneGeneratorKind::Melody(parse_options::<MelodyParams>(role, options)?)
        }
        GenerateRole::Pad => LaneGeneratorKind::Pad(parse_options::<PadParams>(role, options)?),
    })
}

fn parse_options<T: serde::de::DeserializeOwned + Default>(
    role: GenerateRole,
    options: Option<&serde_json::Value>,
) -> Result<T, RpcError> {
    match options {
        None | Some(serde_json::Value::Null) => Ok(T::default()),
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
            RpcError::invalid_params(format!("invalid options for role {role:?}: {e}"))
        }),
    }
}

// ---------------------------------------------------------------------------
// generate.drums
// ---------------------------------------------------------------------------

fn drums(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::DrumsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return fail(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    if let Some(e) = super::section::definition_missing(app, definition_id) {
        return fail(request, e);
    }
    let track_id: u64 = params.track_id.into();
    if let Some(e) = require_drum_track(app, track_id) {
        return fail(request, e);
    }

    if let Some(d) = params.density {
        if !d.is_finite() || !(0.0..=1.0).contains(&d) {
            return fail(
                request,
                RpcError::invalid_params(format!("density must be within 0.0..=1.0 (got {d})")),
            );
        }
    }

    // Resolve a named pattern up front so an unknown name is a precise
    // error, not a silent fall-through to the default. The project's own
    // bank wins over the built-in library, so a user pattern named
    // "halftime" stays theirs.
    let (pattern_id, builtin) = match &params.pattern {
        Some(name) => match pattern_id_by_name(app, name) {
            Some(id) => (Some(id), None),
            None if crate::compose::is_builtin_pattern(name) => (None, Some(name.clone())),
            None => {
                let known: Vec<String> = app
                    .compose
                    .drum_patterns
                    .iter()
                    .map(|p| p.name.clone())
                    .collect();
                return fail(
                    request,
                    RpcError::not_found(format!(
                        "no drum pattern named {name:?} (project: {}; built-in: {})",
                        known.join(", "),
                        crate::compose::builtin_pattern_names().join(", ")
                    )),
                );
            }
        },
        None => (None, None),
    };
    // An empty bank is only a problem when nothing else can supply the
    // groove — a built-in brings its own.
    if builtin.is_none() && app.compose.drum_patterns.is_empty() {
        return fail(
            request,
            RpcError::unsupported(format!(
                "the project has no drum pattern to generate from; name a built-in groove \
                 instead (one of: {})",
                crate::compose::builtin_pattern_names().join(", ")
            )),
        );
    }

    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::GenerateSectionDrums {
            definition_id,
            pattern_id,
            builtin,
            density: params.density,
            seed: params.seed,
        }),
    );
    if let Some(error) = app.compose.last_error.take() {
        return fail(request, RpcError::invalid_params(error));
    }
    let result = GenerateResult::new(generated_clips(app, definition_id, track_id), app.revision());
    (super::success(request, &result), task)
}

// ---------------------------------------------------------------------------
// Track-kind guards
// ---------------------------------------------------------------------------

pub(super) fn require_instrument_track(app: &Resonance, track_id: u64) -> Option<RpcError> {
    match track_kind(app, track_id) {
        None => Some(missing_track(track_id)),
        Some((TrackType::Instrument, InstrumentType::Synth)) => None,
        Some((TrackType::Instrument, InstrumentType::Drum)) => Some(RpcError::invalid_params(
            format!("track {track_id} is a drum track; use generate.drums"),
        )),
        Some(_) => Some(RpcError::invalid_params(format!(
            "track {track_id} is not a synth instrument track"
        ))),
    }
}

/// A Vocal generator only makes sense on a vocal track — that is the
/// track type the SVS render path reads (`section.set_lane_generator`,
/// ba doc #268).
pub(super) fn require_vocal_track(app: &Resonance, track_id: u64) -> Option<RpcError> {
    match track_kind(app, track_id) {
        None => Some(missing_track(track_id)),
        Some((TrackType::Vocal, _)) => None,
        Some(_) => Some(RpcError::invalid_params(format!(
            "track {track_id} is not a vocal track; a vocal lane generator needs one"
        ))),
    }
}

/// Existence only — for the kinds that impose no track-type constraint
/// (clearing a lane back to Manual).
pub(super) fn require_track(app: &Resonance, track_id: u64) -> Option<RpcError> {
    track_kind(app, track_id)
        .is_none()
        .then(|| missing_track(track_id))
}

fn require_drum_track(app: &Resonance, track_id: u64) -> Option<RpcError> {
    match track_kind(app, track_id) {
        None => Some(missing_track(track_id)),
        Some((TrackType::Instrument, InstrumentType::Drum)) => None,
        Some(_) => Some(RpcError::invalid_params(format!(
            "track {track_id} is not a drum track"
        ))),
    }
}

fn track_kind(app: &Resonance, track_id: u64) -> Option<(TrackType, InstrumentType)> {
    app.registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| (t.track_type, t.instrument_type))
}

fn missing_track(track_id: u64) -> RpcError {
    RpcError::not_found(format!("no track with id {track_id}"))
}
