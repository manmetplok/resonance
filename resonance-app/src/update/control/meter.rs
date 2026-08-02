//! `meter.*` control handlers (ba doc #273, todo #1219): put the app's
//! own BS.1770 numbers on the wire instead of making a client bounce a
//! WAV and shell out to an analyser.
//!
//! # Shape
//!
//! A measurement renders the requested slice offline, so it runs as a
//! JOB exactly like `render.mixdown` — the handler validates, registers
//! a [`JobToken::Measure`], fires [`AudioCommand::MeasureMix`] and
//! returns `{job_id}` immediately. The engine answers with exactly one
//! terminal event, `MixMeasured` or `MixMeasureError`, which
//! [`mix_measured`] / [`mix_measure_error`] turn into the job's result.
//! Neither event carries a correlation id, but the engine refuses a
//! second measurement while one runs, so the newest live measure job IS
//! the one that just finished.
//!
//! Nothing here mutates: no file is written, no project or transport
//! state is touched, and the reply carries no `revision`. It is
//! nonetheless dispatched BELOW the mutation gate in
//! [`super::execute`] and deliberately left out of
//! [`super::is_read_only_method`], because a measurement describes the
//! OPEN project — with nothing open the honest answer is a stable
//! `busy`, not an empty measurement that reads like a real one. That is
//! the same call `master.summary` and `edit.status` make.
//!
//! # Guards
//!
//! * `source: "live"` reads the engine's streaming master meter and
//!   needs no render, so it is exempt from every render guard — but it
//!   only exists for the master mix, and asking for a track on that path
//!   is refused up front rather than turned into a job that fails.
//! * `source: "render"` shares the offline renderer with bounce, freeze
//!   and export. A measurement must never disturb a render that is
//!   producing a file, so it is refused `busy` while one is in flight
//!   (the mutation gate covers bounce-in-place and freeze; the WAV
//!   bounce flag is checked here) and while the transport is rolling.
//! * Only one measurement at a time, mirroring the engine's own rule.

use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_audio::types::{
    AudioCommand, MeasureSource as EngineSource, MixMeasurement, SamplePos, StemSource,
};
use resonance_control::methods::meter::{
    self as proto, Bands, MeasureParams, MeasureResult, MeasureSource, MeasureTarget,
};
use resonance_control::methods::render::RangeSpec;
use resonance_control::{Request, Response, RpcError};

/// Handle a `meter.*` request, or `None` when `method` belongs to
/// another namespace. Returns a [`Task`] for signature symmetry with the
/// other below-the-gate handlers; measurement produces none.
pub(super) fn try_handle(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::MEASURE => measure(app, conn, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// meter.measure
// ---------------------------------------------------------------------------

fn measure(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    // Every field defaults, so absent params mean "the whole song's
    // master mix, rendered".
    let params: MeasureParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };

    let target = match resolve_target(app, params.target) {
        Ok(target) => target,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    let source = match params.source {
        MeasureSource::Render => EngineSource::Render,
        MeasureSource::Live => EngineSource::Live,
    };

    if let Some(error) = source_guard(app, source, target) {
        return (super::failure(request, error), Task::none());
    }

    // The live tap integrates from the start of the session and has no
    // notion of a range, so accepting one silently would be a lie.
    let range = if source == EngineSource::Live {
        if params.range.is_some_and(|r| !is_whole(&r)) {
            return (
                super::failure(
                    request,
                    RpcError::invalid_params(
                        "the live master meter has no range; omit `range`, or use \
                         source \"render\" to measure a window",
                    ),
                ),
                Task::none(),
            );
        }
        None
    } else {
        match resolve_range(app, params.range) {
            Ok(range) => range,
            Err(e) => return (super::failure(request, e), Task::none()),
        }
    };

    (
        start(app, conn, request, proto::MEASURE, vec![target], range, source),
        Task::none(),
    )
}

// ---------------------------------------------------------------------------
// Job plumbing
// ---------------------------------------------------------------------------

/// Register the job and fire the one engine command that backs it.
///
/// `targets` is a vector because the engine measures every slice over
/// ONE shared range in ONE pass — which is what keeps the numbers
/// directly comparable, and what lets `meter.stems` be an enumeration
/// rather than a second render path.
fn start(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
    method: &'static str,
    targets: Vec<StemSource>,
    range: Option<(SamplePos, SamplePos)>,
    source: EngineSource,
) -> Response {
    let started = app.start_control_job(
        method,
        &describe(&targets, source),
        JobToken::Measure { method },
        Some(conn),
    );
    if app
        .engine
        .send(AudioCommand::MeasureMix {
            targets,
            range,
            source,
        })
        .is_err()
    {
        app.control.jobs.fail(
            u64::from(started.job_id),
            "measurement did not start (engine unavailable)",
        );
    }
    super::success(request, &started)
}

fn describe(targets: &[StemSource], source: EngineSource) -> String {
    let what = match (targets.len(), targets.first()) {
        (1, Some(StemSource::Master)) => "the master mix".to_owned(),
        (1, Some(StemSource::Track(id))) => format!("track {id}"),
        (1, Some(StemSource::Bus(id))) => format!("bus {id}"),
        (n, _) => format!("{n} mix slices"),
    };
    match source {
        EngineSource::Live => format!("Measure {what} (live meter)"),
        EngineSource::Render => format!("Measure {what}"),
    }
}

/// Resolve the control job for a completed [`AudioCommand::MeasureMix`].
///
/// Called from `engine_events::dispatch`. A no-op when no measurement
/// was control-initiated — the GUI has no measurement surface today, so
/// that path is unreachable rather than merely unusual, but treating it
/// as normal keeps the hook honest if one is ever added.
pub(crate) fn mix_measured(app: &mut Resonance, results: Vec<MixMeasurement>) {
    let Some((job_id, token)) = app.control.jobs.newest_live_measure() else {
        return;
    };
    let JobToken::Measure { method } = token else {
        return;
    };
    if method != proto::MEASURE {
        return;
    }
    // `meter.measure` asks for exactly one target, so exactly one
    // measurement comes back. An empty set means the engine changed
    // under us; fail loudly rather than report a default.
    match results.first() {
        Some(measurement) => {
            let payload = serde_json::to_value(measure_result(*measurement, app.sample_rate))
                .unwrap_or(serde_json::Value::Null);
            app.control.jobs.complete(job_id, payload);
        }
        None => app
            .control
            .jobs
            .fail(job_id, "the engine returned no measurement"),
    }
}

/// Fail the in-flight measurement job with the engine's reason.
///
/// The engine's `MixMeasureError` string is written to be user-facing
/// ("Another offline render is in progress", "Stop transport before
/// measuring the mix", ...), so it is passed through verbatim rather
/// than re-worded here. [`source_guard`] already answers the cases the
/// app can see coming with a synchronous `busy`; this is what is left
/// once the engine wins a race the app could not.
pub(crate) fn mix_measure_error(app: &mut Resonance, message: String) {
    match app.control.jobs.newest_live_measure() {
        Some((job_id, _)) => app.control.jobs.fail(job_id, message),
        None => eprintln!("audio: mix measurement failed: {message}"),
    }
}

// ---------------------------------------------------------------------------
// Wire <-> engine conversion
// ---------------------------------------------------------------------------

/// Turn one engine measurement into its wire form.
///
/// The two jobs this does beyond renaming fields are what the result's
/// honesty rests on: `-inf` (silence, or a range too short to fill the
/// meter's window) becomes `null` rather than a number JSON cannot
/// represent, and every figure the live streaming tap cannot supply
/// becomes `null` rather than the placeholder zero the engine carries.
///
/// `sample_rate` is the engine's, used only to turn the measured frame
/// count into seconds.
pub(crate) fn measure_result(m: MixMeasurement, sample_rate: u32) -> MeasureResult {
    let live = m.source == EngineSource::Live;
    MeasureResult {
        target: wire_target(m.target),
        source: match m.source {
            EngineSource::Render => MeasureSource::Render,
            EngineSource::Live => MeasureSource::Live,
        },
        lufs_integrated: finite(m.lufs_integrated),
        // The live tap keeps no history, so its short-term / momentary
        // readings are the CURRENT windows, not maxima — reporting them
        // under a `_max` name would be wrong. They move to the `_now`
        // fields instead, which only ever appear on this path.
        lufs_short_max: (!live).then(|| finite(m.lufs_short_term_max)).flatten(),
        lufs_momentary_max: (!live).then(|| finite(m.lufs_momentary_max)).flatten(),
        lufs_short_term_now: live.then(|| finite(m.lufs_short_term_max)).flatten(),
        lufs_momentary_now: live.then(|| finite(m.lufs_momentary_max)).flatten(),
        lra: m.lra_lu,
        true_peak_db: m.true_peak_dbtp,
        sample_peak_db: (!live).then_some(m.sample_peak_db),
        crest_db: m.crest_db,
        clipped_samples: (!live).then_some(m.clipped_samples),
        correlation: m.correlation,
        mono_penalty_db: (!live).then_some(m.mono_penalty_db),
        bands: (!live).then_some(Bands {
            low: m.bands.low,
            mid: m.bands.mid,
            high: m.bands.high,
            air: m.bands.air,
        }),
        // The engine reports the frame count it actually fed the meters,
        // which is authoritative: a client that asked for "the whole
        // song" learns how long that turned out to be.
        measured_seconds: (!live)
            .then(|| (sample_rate > 0).then(|| m.frames as f64 / f64::from(sample_rate)))
            .flatten(),
    }
}

/// `-inf` (silence, or a range shorter than the meter's window) has no
/// JSON representation and is not a level; report it as absent.
fn finite(value: f32) -> Option<f32> {
    value.is_finite().then_some(value)
}

fn wire_target(source: StemSource) -> MeasureTarget {
    match source {
        StemSource::Master => MeasureTarget::Master,
        StemSource::Track(id) => MeasureTarget::Track(id.into()),
        StemSource::Bus(id) => MeasureTarget::Bus(id.into()),
    }
}

// ---------------------------------------------------------------------------
// Param resolution
// ---------------------------------------------------------------------------

/// Resolve a wire target against the current topology.
///
/// `{"track_id": N}` also accepts a bus id: `song.summary` reports
/// busses as track lines from the same id space, so refusing an id it
/// just handed out would be a trap. `{"bus_id": N}` is the explicit
/// spelling and refuses anything that is not a bus.
fn resolve_target(app: &Resonance, target: MeasureTarget) -> Result<StemSource, RpcError> {
    match target {
        MeasureTarget::Master => Ok(StemSource::Master),
        MeasureTarget::Track(id) => {
            let raw = u64::from(id);
            if app.sorted_tracks().iter().any(|t| t.id == raw) {
                Ok(StemSource::Track(raw))
            } else if app.sorted_busses().iter().any(|b| b.id == raw) {
                Ok(StemSource::Bus(raw))
            } else {
                Err(RpcError::not_found(format!("no track with id {raw}")))
            }
        }
        MeasureTarget::Bus(id) => {
            let raw = u64::from(id);
            if app.sorted_busses().iter().any(|b| b.id == raw) {
                Ok(StemSource::Bus(raw))
            } else {
                Err(RpcError::not_found(format!("no bus with id {raw}")))
            }
        }
    }
}

/// Resolve a wire range to engine samples, or `None` for "the whole
/// project" (which the engine derives from the clips itself).
///
/// A range reaching past the end of the song is CLAMPED rather than
/// refused: asking for eight bars of a six-bar song is a reasonable
/// thing for a client to do, and measuring the six that exist answers
/// it. A range lying entirely outside the song is a different thing —
/// there is nothing there to measure, and silently returning the
/// silence floor would look like a real reading.
fn resolve_range(
    app: &Resonance,
    range: Option<RangeSpec>,
) -> Result<Option<(SamplePos, SamplePos)>, RpcError> {
    let Some(range) = range.filter(|r| !is_whole(r)) else {
        return Ok(None);
    };

    let mut start = match range.start {
        Some(spec) if !spec.is_empty() => super::transport::resolve_position(app, &spec)?,
        _ => 0,
    };
    let song_end = super::song::song_end_sample(app);
    let mut end = match range.end {
        Some(spec) if !spec.is_empty() => super::transport::resolve_position(app, &spec)?,
        // An open-ended range runs to the end of the song. With an empty
        // project there is nothing to run to, so hand the whole thing
        // back to the engine and let it report "no audio to measure".
        _ if song_end > 0 => song_end,
        _ => return Ok(None),
    };

    if song_end > 0 {
        start = start.min(song_end);
        end = end.min(song_end);
    }
    if end <= start {
        return Err(RpcError::invalid_params(format!(
            "empty measurement range: {start}..{end} samples (the song spans \
             0..{song_end})"
        )));
    }
    Ok(Some((start, end)))
}

/// True when a range names no coordinates at all, i.e. "the whole song".
fn is_whole(range: &RangeSpec) -> bool {
    range.start.is_none_or(|p| p.is_empty()) && range.end.is_none_or(|p| p.is_empty())
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// Why this measurement cannot run right now, or `None`.
fn source_guard(
    app: &Resonance,
    source: EngineSource,
    target: StemSource,
) -> Option<RpcError> {
    if source == EngineSource::Live {
        // There is one live tap and it sits on the master mix; reporting
        // that snapshot under a track's name would be a fabrication.
        if target != StemSource::Master {
            return Some(RpcError::invalid_params(
                "the live meter only exists for the master mix; measure a track \
                 with source \"render\"",
            ));
        }
        // No render, so none of the offline guards apply — measuring the
        // live meter during playback is the whole point of this source.
        return None;
    }

    if app.transport.recording {
        return Some(RpcError::busy(
            "the transport is recording; stop it before measuring",
        ));
    }
    if app.transport.playing {
        return Some(RpcError::busy(
            "the transport is rolling; stop it before measuring, or use \
             source \"live\" to read the master meter as it plays",
        ));
    }
    // The mutation gate above already refuses a bounce-in-place or a
    // freeze; this is the WAV-bounce flag it does not cover, and it is
    // the case the field report hits (measure right after asking for a
    // mixdown). All three mean the same thing to a measurement: the
    // offline renderer is taken.
    if app.io.bouncing {
        return Some(RpcError::busy(
            "a render is in progress; measure again when it finishes",
        ));
    }
    if app.control.jobs.newest_live_measure().is_some() {
        return Some(RpcError::busy("a measurement is already in progress"));
    }
    None
}
