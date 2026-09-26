//! `harmony.*` control methods (ba doc #265, todo #1153): chords on a
//! section definition's grid, plus music-theory progression application.
//!
//! All four methods funnel into the single
//! [`ComposeMessage::ReplaceSectionChords`] domain message, so each call
//! is one undoable transaction with one lane-regeneration cascade — and
//! chord symbols keep full fidelity (quality *and* slash bass) where the
//! GUI's incremental `AddChord` cannot carry a bass note.
//!
//! Wire conventions (doc #265): beats are section-relative `f64` on the
//! wire; the app's chord grid is whole beats, so fractional positions
//! are rejected as `invalid_params`. Chord symbols round-trip through
//! `resonance_music_theory::parse_chord` / `Display` (the same pair
//! `song.sections` uses to render them).
//!
//! Frozen tracks are not refused here (code review FU-M4b): chords are
//! the section's shared harmony, not one track's input, and the GUI's
//! chord edits are not gated either. A frozen lane the cascade rewrites
//! goes Stale for a refreeze. `generate.*`, which targets tracks, does
//! refuse — see `generate::frozen_reject`.

use std::collections::HashSet;

use crate::compose::{ChordState, ComposeMessage, SectionChordSpec};
use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::ids::ChordId;
use resonance_control::methods::harmony as proto;
use resonance_control::{Request, Response, RpcError};
use resonance_music_theory::{diatonic_chord, parse_chord, Chord, Scale};

use super::reply::{ack_or_compose_error, ack_task, reject};
use super::section::{definition_missing, parse_key_scale, take_compose_error};

pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::ADD_CHORD => add_chord(app, request),
        proto::EDIT_CHORD => edit_chord(app, request),
        proto::DELETE_CHORD => delete_chord(app, request),
        proto::APPLY_PROGRESSION => apply_progression(app, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

fn add_chord(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::AddChordParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    if let Some(e) = definition_missing(app, definition_id) {
        return reject(request, e);
    }
    let (start_beat, duration_beats) =
        match beats(params.start_beat, params.duration_beats) {
            Ok(v) => v,
            Err(e) => return reject(request, e),
        };
    let chord = match parse_symbol(&params.symbol) {
        Ok(c) => c,
        Err(e) => return reject(request, e),
    };

    let before: HashSet<u64> = chord_ids(app, definition_id).into_iter().collect();
    let mut specs = existing_specs(app, definition_id);
    specs.push(SectionChordSpec {
        id: None,
        start_beat,
        duration_beats,
        chord,
    });
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ReplaceSectionChords {
            definition_id,
            chords: specs,
        }),
    );
    if let Some(error) = take_compose_error(app) {
        return reject(request, RpcError::invalid_params(error));
    }
    let Some(chord_id) = chord_ids(app, definition_id)
        .into_iter()
        .find(|id| !before.contains(id))
    else {
        return reject(request, RpcError::internal("chord was not created"));
    };
    let result = proto::AddChordResult {
        chord_id: chord_id.into(),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn edit_chord(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::EditChordParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    let chord_id: u64 = params.chord_id.into();
    if let Some(e) = definition_missing(app, definition_id) {
        return reject(request, e);
    }
    let mut specs = existing_specs(app, definition_id);
    let Some(slot) = specs.iter_mut().find(|s| s.id == Some(chord_id)) else {
        return reject(
            request,
            RpcError::not_found(format!(
                "no chord with id {chord_id} in section {definition_id}"
            )),
        );
    };
    if params.symbol.is_none() && params.start_beat.is_none() && params.duration_beats.is_none() {
        // Nothing to change; don't spend an undo entry on a no-op.
        return ack_task(app, request, Task::none());
    }
    if let Some(symbol) = &params.symbol {
        slot.chord = match parse_symbol(symbol) {
            Ok(c) => c,
            Err(e) => return reject(request, e),
        };
    }
    if let Some(start) = params.start_beat {
        slot.start_beat = match whole_beats(start, "start_beat") {
            Ok(b) => b,
            Err(e) => return reject(request, e),
        };
    }
    if let Some(duration) = params.duration_beats {
        slot.duration_beats = match whole_beats(duration, "duration_beats") {
            Ok(b) => b,
            Err(e) => return reject(request, e),
        };
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ReplaceSectionChords {
            definition_id,
            chords: specs,
        }),
    );
    ack_or_compose_error(app, request, task)
}

fn delete_chord(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::DeleteChordParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    let chord_id: u64 = params.chord_id.into();
    if let Some(e) = definition_missing(app, definition_id) {
        return reject(request, e);
    }
    let mut specs = existing_specs(app, definition_id);
    let len_before = specs.len();
    specs.retain(|s| s.id != Some(chord_id));
    if specs.len() == len_before {
        return reject(
            request,
            RpcError::not_found(format!(
                "no chord with id {chord_id} in section {definition_id}"
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ReplaceSectionChords {
            definition_id,
            chords: specs,
        }),
    );
    ack_or_compose_error(app, request, task)
}

fn apply_progression(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::ApplyProgressionParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let definition_id: u64 = params.section_id.into();
    if let Some(e) = definition_missing(app, definition_id) {
        return reject(request, e);
    }
    let chords = match resolve_progression(&params) {
        Ok(c) => c,
        Err(e) => return reject(request, e),
    };

    let time_sig_num = crate::update::compose::section_meter(app, definition_id).numerator;
    let beats_per_chord = match params.beats_per_chord {
        Some(b) => match whole_beats(b, "beats_per_chord") {
            Ok(0) | Err(_) => {
                return reject(
                    request,
                    RpcError::invalid_params(format!(
                        "beats_per_chord must be a positive whole number of beats; got {b}"
                    )),
                )
            }
            Ok(b) => b,
        },
        // Doc #265 default: one bar per chord.
        None => u32::from(time_sig_num),
    };
    let section_beats = app
        .compose
        .find_definition(definition_id)
        .map(|d| d.length_bars * u32::from(time_sig_num))
        .unwrap_or(0);
    let needed = chords.len() as u32 * beats_per_chord;
    if needed > section_beats {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "progression needs {needed} beats ({} chords x {beats_per_chord} beats) but the \
                 section is {section_beats} beats long",
                chords.len()
            )),
        );
    }

    let specs: Vec<SectionChordSpec> = chords
        .into_iter()
        .enumerate()
        .map(|(i, chord)| SectionChordSpec {
            id: None,
            start_beat: i as u32 * beats_per_chord,
            duration_beats: beats_per_chord,
            chord,
        })
        .collect();
    let task = super::run_via_update(
        app,
        Message::Compose(ComposeMessage::ReplaceSectionChords {
            definition_id,
            chords: specs,
        }),
    );
    if let Some(error) = take_compose_error(app) {
        return reject(request, RpcError::invalid_params(error));
    }
    let result = proto::ApplyProgressionResult {
        chord_ids: chord_ids(app, definition_id)
            .into_iter()
            .map(ChordId::from)
            .collect(),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

// ---------------------------------------------------------------------------
// Progression sources
// ---------------------------------------------------------------------------

/// Named progressions, as roman-numeral degree sequences rendered
/// diatonically in the request's key. Kept deliberately small and
/// spelled out — the `numerals` param covers everything else.
const PRESETS: &[(&str, &[&str])] = &[
    ("pop", &["I", "V", "vi", "IV"]),
    ("axis", &["I", "V", "vi", "IV"]),
    ("50s", &["I", "vi", "IV", "V"]),
    ("doo-wop", &["I", "vi", "IV", "V"]),
    ("pachelbel", &["I", "V", "vi", "iii", "IV", "I", "IV", "V"]),
    ("andalusian", &["i", "VII", "VI", "V"]),
    ("ii-V-I", &["ii", "V", "I"]),
    (
        "12-bar-blues",
        &["I", "I", "I", "I", "IV", "IV", "I", "I", "V", "IV", "I", "I"],
    ),
];

/// Resolve the request's chord source — explicit `symbols`, `key` +
/// `numerals`, or `key` + `preset` (exactly one).
fn resolve_progression(params: &proto::ApplyProgressionParams) -> Result<Vec<Chord>, RpcError> {
    let sources = usize::from(params.symbols.is_some())
        + usize::from(params.numerals.is_some())
        + usize::from(params.preset.is_some());
    if sources != 1 {
        return Err(RpcError::invalid_params(
            "provide exactly one chord source: symbols, numerals (with key), or preset (with key)",
        ));
    }
    if let Some(symbols) = &params.symbols {
        if symbols.is_empty() {
            return Err(RpcError::invalid_params("symbols must not be empty"));
        }
        return symbols.iter().map(|s| parse_symbol(s)).collect();
    }

    let Some(key) = &params.key else {
        return Err(RpcError::invalid_params(
            "key is required when using numerals or preset",
        ));
    };
    let scale = parse_key_scale(key)?;
    let sevenths = params.sevenths.unwrap_or(false);
    let numerals: Vec<&str> = match &params.numerals {
        Some(numerals) => {
            if numerals.is_empty() {
                return Err(RpcError::invalid_params("numerals must not be empty"));
            }
            numerals.iter().map(String::as_str).collect()
        }
        None => {
            let preset = params.preset.as_deref().unwrap_or_default();
            let wanted = preset.trim();
            let Some((_, numerals)) = PRESETS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            else {
                let known: Vec<&str> = PRESETS.iter().map(|(name, _)| *name).collect();
                return Err(RpcError::invalid_params(format!(
                    "unknown progression preset {preset:?} (one of: {})",
                    known.join(", ")
                )));
            };
            numerals.to_vec()
        }
    };
    numerals
        .into_iter()
        .map(|n| render_numeral(scale, n, sevenths))
        .collect()
}

/// One roman numeral (or plain digit) rendered diatonically in `scale`.
/// Case is accepted but ignored — the chord quality comes from the
/// scale, matching `resonance_music_theory::diatonic_chord`.
fn render_numeral(scale: Scale, numeral: &str, sevenths: bool) -> Result<Chord, RpcError> {
    let degree = match numeral.trim().to_ascii_lowercase().as_str() {
        "i" | "1" => 1,
        "ii" | "2" => 2,
        "iii" | "3" => 3,
        "iv" | "4" => 4,
        "v" | "5" => 5,
        "vi" | "6" => 6,
        "vii" | "7" => 7,
        _ => {
            return Err(RpcError::invalid_params(format!(
                "invalid roman numeral {numeral:?} (expected i..vii; quality is diatonic to the key)"
            )))
        }
    };
    Ok(diatonic_chord(scale, degree, sevenths))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The section's current chords as keep-the-id replacement specs.
fn existing_specs(app: &Resonance, definition_id: u64) -> Vec<SectionChordSpec> {
    app.compose
        .find_definition(definition_id)
        .map(|d| d.chords.iter().map(spec_of).collect())
        .unwrap_or_default()
}

fn spec_of(state: &ChordState) -> SectionChordSpec {
    SectionChordSpec {
        id: Some(state.id),
        start_beat: state.start_beat,
        duration_beats: state.duration_beats,
        chord: state.chord,
    }
}

/// The section's chord ids in grid order.
fn chord_ids(app: &Resonance, definition_id: u64) -> Vec<u64> {
    app.compose
        .find_definition(definition_id)
        .map(|d| d.chords.iter().map(|c| c.id).collect())
        .unwrap_or_default()
}

fn parse_symbol(symbol: &str) -> Result<Chord, RpcError> {
    parse_chord(symbol)
        .map_err(|e| RpcError::invalid_params(format!("invalid chord symbol {symbol:?}: {e}")))
}

/// Both beat coordinates of an add: start (>= 0) and duration (>= 1).
fn beats(start: f64, duration: f64) -> Result<(u32, u32), RpcError> {
    let start = whole_beats(start, "start_beat")?;
    let duration = whole_beats(duration, "duration_beats")?;
    if duration == 0 {
        return Err(RpcError::invalid_params(
            "duration_beats must be at least 1",
        ));
    }
    Ok((start, duration))
}

/// A wire beat value mapped onto the app's whole-beat chord grid.
fn whole_beats(value: f64, what: &str) -> Result<u32, RpcError> {
    if value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value <= f64::from(u32::MAX) {
        Ok(value as u32)
    } else {
        Err(RpcError::invalid_params(format!(
            "{what} must be a whole non-negative number of beats (the chord grid is whole \
             beats in protocol v1); got {value}"
        )))
    }
}
