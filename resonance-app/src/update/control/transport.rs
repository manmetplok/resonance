//! `transport.*` control methods (ba doc #265, todo #1150).
//!
//! Every mutation synthesizes the existing [`TransportMessage`] (or
//! [`ChordTrackMessage`] for the global key) and routes it through the
//! FULL `update()` path via [`super::run_via_update`], so remote edits
//! hit the same gates and undo classification as the GUI: play / seek /
//! stop are transient (no undo entry), tempo / time-signature / loop /
//! key edits are undoable and revert with Cmd-Z. Params are validated
//! *before* dispatch so a rejection reports precisely instead of
//! silently no-oping. Every reply is a [`TransportResult`] echoing the
//! post-call transport plus the revision counter.

use crate::message::{ChordTrackMessage, Message, TransportMessage};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::transport::{
    self, LoopSetParams, SeekParams, SetKeyParams, SetTempoParams, SetTimeSignatureParams,
    TransportResult,
};
use resonance_control::{PositionSpec, Request, Response, RpcError};
use resonance_music_theory::{Mode, PitchClass, Scale};

/// BPM range the transport accepts (mirrors `CommitBpm`'s clamp — the
/// control path rejects instead of clamping so the client learns why).
const BPM_RANGE: std::ops::RangeInclusive<f64> = 20.0..=300.0;

/// Handle a `transport.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        transport::PLAY => simple(app, request, TransportMessage::Play),
        transport::STOP => simple(app, request, TransportMessage::Stop),
        transport::PAUSE => simple(app, request, TransportMessage::Pause),
        transport::LOOP_TOGGLE => simple(app, request, TransportMessage::ToggleLoop),
        transport::SEEK => seek(app, request),
        transport::LOOP_SET => loop_set(app, request),
        transport::SET_TEMPO => set_tempo(app, request),
        transport::SET_TIME_SIGNATURE => set_time_signature(app, request),
        transport::SET_KEY => set_key(app, request),
        _ => return None,
    };
    Some(out)
}

/// The `TransportResult` echo built from post-dispatch state.
fn echo(app: &Resonance, request: &Request) -> Response {
    let result = TransportResult {
        state: super::song::transport_state(app),
        playhead: super::song::song_position(app, app.transport.playhead),
        looping: app.transport.loop_enabled,
        revision: app.revision(),
    };
    super::success(request, &result)
}

fn reject(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (super::failure(request, error), Task::none())
}

/// A no-params method that maps 1:1 onto one transport message.
fn simple(
    app: &mut Resonance,
    request: &Request,
    message: TransportMessage,
) -> (Response, Task<Message>) {
    let task = super::run_via_update(app, Message::Transport(message));
    (echo(app, request), task)
}

fn seek(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SeekParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let sample = match resolve_position(app, &params.position) {
        Ok(s) => s,
        Err(e) => return reject(request, e),
    };
    simple(app, request, TransportMessage::SeekToSample(sample))
}

fn loop_set(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: LoopSetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let start = match resolve_position(app, &params.start) {
        Ok(s) => s,
        Err(e) => return reject(request, e),
    };
    let end = match resolve_position(app, &params.end) {
        Ok(s) => s,
        Err(e) => return reject(request, e),
    };
    if start >= end {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "loop start must be before end (got start={start}, end={end} samples)"
            )),
        );
    }
    simple(
        app,
        request,
        TransportMessage::SetLoopRange {
            loop_in: start,
            loop_out: end,
            enabled: params.enabled,
        },
    )
}

fn set_tempo(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetTempoParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if !params.bpm.is_finite() || !BPM_RANGE.contains(&params.bpm) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "bpm {} out of range {:?}",
                params.bpm, BPM_RANGE
            )),
        );
    }
    // Route the GUI's own two-step text path so the commit (and its
    // undo entry) is byte-for-byte the same edit the tempo field makes.
    let text = super::run_via_update(
        app,
        Message::Transport(TransportMessage::SetBpmText(params.bpm.to_string())),
    );
    let commit = super::run_via_update(app, Message::Transport(TransportMessage::CommitBpm));
    (echo(app, request), Task::batch([text, commit]))
}

fn set_time_signature(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetTimeSignatureParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if params.numerator == 0 || params.numerator > 32 {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "time-signature numerator {} out of range 1..=32",
                params.numerator
            )),
        );
    }
    if !matches!(params.denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "time-signature denominator {} must be a power of two (1..=32)",
                params.denominator
            )),
        );
    }
    simple(
        app,
        request,
        TransportMessage::SetTimeSignature {
            numerator: params.numerator,
            denominator: params.denominator,
        },
    )
}

fn set_key(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetKeyParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let tonic = match parse_tonic(&params.tonic) {
        Some(t) => t,
        None => {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "unknown tonic {:?} (expected a pitch name like \"A\" or \"F#\")",
                    params.tonic
                )),
            )
        }
    };
    let mode = match parse_mode(&params.scale) {
        Some(m) => m,
        None => {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "unknown scale {:?} (expected e.g. \"major\", \"minor\", \"dorian\")",
                    params.scale
                )),
            )
        }
    };
    // The app's global key is the chord track's song key (the transport
    // label source), so `transport.set_key` IS supported and routes the
    // existing undoable message.
    let task = super::run_via_update(
        app,
        Message::ChordTrack(ChordTrackMessage::SetSongKey {
            scale: Scale::new(tonic, mode),
        }),
    );
    (echo(app, request), task)
}

// ---------------------------------------------------------------------------
// Param resolution
// ---------------------------------------------------------------------------

/// Resolve a client [`PositionSpec`] (1-based musical bar/beat, or an
/// absolute sample) to a sample position via the tempo map.
///
/// Shared with `meter.*` (todo #1219), which windows a measurement with
/// the same vocabulary — one definition of "bar 5" for the whole control
/// surface.
pub(super) fn resolve_position(app: &Resonance, spec: &PositionSpec) -> Result<u64, RpcError> {
    if let Some(sample) = spec.sample {
        if spec.bar.is_some() || spec.beat.is_some() {
            return Err(RpcError::invalid_params(
                "give either a musical position (bar [+ beat]) or a sample, not both",
            ));
        }
        return Ok(sample);
    }
    let Some(bar) = spec.bar else {
        return Err(RpcError::invalid_params(
            "position needs a bar (with optional beat) or a sample",
        ));
    };
    if bar < 1 {
        return Err(RpcError::invalid_params("bar is 1-based"));
    }
    let beat = spec.beat.unwrap_or(1.0);
    if beat < 1.0 {
        return Err(RpcError::invalid_params("beat is 1-based"));
    }
    let bar_sample = app.tempo_map.bar_to_sample(bar - 1); // wire 1-based -> app 0-based
    let ticks = ((beat - 1.0) * resonance_audio::types::TICKS_PER_QUARTER_NOTE as f64).round()
        as u64;
    Ok(app
        .tempo_map
        .tick_to_abs_sample(bar_sample, ticks, app.sample_rate))
}

/// Parse a pitch name (`"A"`, `"F#"`, and the flat spellings) into a
/// [`PitchClass`].
fn parse_tonic(name: &str) -> Option<PitchClass> {
    let name = name.trim();
    // Flat spellings map onto the enharmonic sharp pitch classes.
    let normalized = match name.to_ascii_uppercase().as_str() {
        "DB" => "C#",
        "EB" => "D#",
        "GB" => "F#",
        "AB" => "G#",
        "BB" => "A#",
        _ => name,
    };
    (0..12u8)
        .map(PitchClass::from_semitone)
        .find(|pc| pc.as_str().eq_ignore_ascii_case(normalized))
}

/// Parse a lowercase scale name into a [`Mode`] (the wire strings are
/// exactly [`Mode::as_str`]).
fn parse_mode(name: &str) -> Option<Mode> {
    [
        Mode::Chromatic,
        Mode::Major,
        Mode::Minor,
        Mode::Dorian,
        Mode::Phrygian,
        Mode::Lydian,
        Mode::Mixolydian,
        Mode::Locrian,
        Mode::HarmonicMinor,
        Mode::MelodicMinor,
    ]
    .into_iter()
    .find(|m| m.as_str().eq_ignore_ascii_case(name.trim()))
}
