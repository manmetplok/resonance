//! `meter.*` control handlers (ba doc #273, todos #1219 / #1220): put
//! the app's own BS.1770 numbers on the wire instead of making a client
//! bounce a WAV and shell out to an analyser.
//!
//! # Shape
//!
//! A measurement renders the requested slice offline, so it runs as a
//! JOB exactly like `render.mixdown` — the handler validates, registers
//! a [`JobToken::Measure`], fires [`AudioCommand::MeasureMix`] and
//! returns `{job_id}` immediately. The engine answers with exactly one
//! terminal event, `MixMeasured` or `MixMeasureError`, which
//! [`mix_measured`] / [`mix_measure_error`] turn into the job's result.
//! Both events echo the `measure_id` the command carried, and the token
//! passed is the JOB ID itself (ba todo #1243) — so a result names its
//! own job and correlation is a lookup, not an inference. An event whose
//! job is already terminal or gone resolves nothing.
//!
//! `meter.stems` is the same job over a longer `targets` vector: ONE
//! engine pass measuring the master and every top-level track over one
//! shared range. It is a pure enumeration here — never N commands, and
//! never a mute-and-bounce, which is what let a whole drum kit bleed
//! into every other stem in the field and produce a plausible,
//! completely wrong balance table. Note what "one pass" does and does
//! not claim: one command, one range, one guard, one `MixMeasured`. The
//! engine still loops `render_stem` once per target inside it
//! (`engine/bounce/measure.rs`), so a pass costs N full-length renders
//! and scales with track count — which is why the MCP tool warns that a
//! large project can outlast its wait and have to be polled.
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
//! * **One OFFLINE measurement at a time.** This mirrors the engine's
//!   own rule — two offline measurements would contend for the single
//!   offline renderer, and `OfflineRenderGuard` refuses the second — and
//!   answering `busy` synchronously beats handing back a job id that is
//!   only going to fail. It applies to `source: "render"` only. Until ba
//!   todo #1243 it had to apply to EVERY source, live included, because
//!   the correlation was "the newest live measure job" and a second open
//!   job would have made it name the wrong one; that reason is gone, and
//!   a live read now runs happily alongside a render measurement.
//! * `source: "live"` reads the engine's streaming master meter and
//!   needs no render, so it is exempt from every *render* guard — but it
//!   only exists for the master mix, and asking for a track on that path
//!   is refused up front rather than turned into a job that fails.
//! * `source: "render"` shares the offline renderer with bounce, freeze
//!   and export. A measurement must never disturb a render that is
//!   producing a file, so it is refused `busy` while one is in flight
//!   (the mutation gate covers bounce-in-place and freeze; the WAV
//!   bounce flag is checked here) and while the transport is rolling.
//!
//! # Known coupling: there is still no `job.cancel`
//!
//! A Measure job is only ever ended by an engine event, and the offline
//! guard refuses every later `source: "render"` call until it ends.
//! Every engine path emits exactly one terminal event (`MixMeasured` or
//! `MixMeasureError`, on every branch of `bounce::measure`), so the job
//! always resolves — but if one ever failed to, render measurements
//! would stay wedged behind "a measurement is already in progress" for
//! the life of the process. Live reads no longer wedge with it.
//!
//! What ba todo #1243 changed is that bounding this with a wall-clock
//! timeout is now SAFE. Reaping a job on a deadline used to be
//! unacceptable because a late engine event would then complete whatever
//! job was open next — one caller receiving another's numbers, the exact
//! defect the #1219 review caught. With `measure_id` on both events a
//! late event names a job that is already terminal and
//! [`crate::control_jobs::JobBoard::live_measure`] drops it. The timeout
//! itself is deliberately NOT added here: picking a deadline that cannot
//! reap a legitimately slow `meter.stems` on a large project (N
//! full-length renders) is a judgement call of its own, and it belongs
//! with `job.cancel` rather than bolted onto this handler.

use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_audio::types::{
    AudioCommand, MeasureSource as EngineSource, MixMeasurement, SamplePos, StemSource,
};
use resonance_control::methods::meter::{
    self as proto, Bands, MeasureParams, MeasureResult, MeasureSource, MeasureTarget, StemsParams,
    StemsResult, TrackMeasurement,
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
        proto::STEMS => stems(app, conn, request),
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
// meter.stems
// ---------------------------------------------------------------------------

/// Measure the master and every track in ONE engine pass.
///
/// This is a pure control-layer enumeration: it builds one `targets`
/// vector and issues one [`AudioCommand::MeasureMix`]. It is emphatically
/// NOT N measurements — that is what makes a balance pass cost one call
/// instead of one full-length bounce per track, and it is why every
/// entry shares a range and is therefore comparable.
///
/// Sub-tracks are deliberately NOT enumerated. A sub-track is one output
/// port of its parent's instrument and carries no material of its own;
/// `stem_filter` already folds it into the parent, and asking the engine
/// for it on its own would render silence (the parent's instrument never
/// runs). So the kit is measured once, on the parent, which reports the
/// folded ids in `includes_track_ids`.
fn stems(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: StemsParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };

    // Stems always render: there is one live tap and it only covers the
    // master, so it could never answer this question.
    if let Some(error) = source_guard(app, EngineSource::Render, StemSource::Master) {
        return (super::failure(request, error), Task::none());
    }
    let range = match resolve_range(app, params.range) {
        Ok(range) => range,
        Err(e) => return (super::failure(request, e), Task::none()),
    };

    let mut targets = vec![StemSource::Master];
    targets.extend(stem_track_ids(app).map(StemSource::Track));
    if params.include_busses {
        targets.extend(app.sorted_busses().iter().map(|b| StemSource::Bus(b.id)));
    }

    (
        start(
            app,
            conn,
            request,
            proto::STEMS,
            targets,
            range,
            EngineSource::Render,
        ),
        Task::none(),
    )
}

/// The tracks that get an entry of their own, in mixer order: the
/// top-level ones. A sub-track is measured as part of its parent (see
/// [`stems`]), so listing it here would both double-count the kit and
/// ask the engine for a stem that renders silent.
fn stem_track_ids(app: &Resonance) -> impl Iterator<Item = u64> + '_ {
    app.sorted_tracks()
        .iter()
        .filter(|t| t.sub_track.is_none())
        .map(|t| t.id)
}

/// The sub-tracks folded into `parent`'s measurement, in mixer order —
/// exactly the set `stem_filter` adds for `StemSource::Track(parent)`.
fn folded_sub_tracks(app: &Resonance, parent: u64) -> Vec<resonance_control::ids::TrackId> {
    app.sorted_tracks()
        .iter()
        .filter(|t| t.sub_track.is_some_and(|link| link.parent_track_id == parent))
        .map(|t| t.id.into())
        .collect()
}

/// Build the `meter.stems` payload from one pass's results.
///
/// The engine echoes each result's `target`, so entries are attributed
/// by identity rather than by trusting request order.
fn stems_result(app: &Resonance, results: &[MixMeasurement]) -> Option<StemsResult> {
    let master = results
        .iter()
        .find(|m| m.target == StemSource::Master)
        .map(|m| measure_result(*m, app.sample_rate))?;

    let tracks = results
        .iter()
        .filter_map(|m| {
            let (id, includes) = match m.target {
                StemSource::Master => return None,
                StemSource::Track(id) => (id, folded_sub_tracks(app, id)),
                StemSource::Bus(id) => (id, Vec::new()),
            };
            Some(TrackMeasurement {
                track_id: id.into(),
                name: entry_name(app, m.target, id),
                includes_track_ids: includes,
                measurement: measure_result(*m, app.sample_rate),
            })
        })
        .collect();

    Some(StemsResult { master, tracks })
}

/// The measured entry's display name. A track deleted between the
/// request and the engine's answer still gets an honest label rather
/// than an empty string.
fn entry_name(app: &Resonance, target: StemSource, id: u64) -> String {
    let found = match target {
        StemSource::Bus(_) => app
            .sorted_busses()
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.name.clone()),
        _ => app
            .sorted_tracks()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone()),
    };
    found.unwrap_or_else(|| format!("(removed {id})"))
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
        JobToken::Measure {
            method,
            offline: source == EngineSource::Render,
        },
        Some(conn),
    );
    // The job id IS the correlation token (ba todo #1243). The engine
    // treats it as opaque and echoes it on whichever terminal event the
    // command produces, so the result names its own job instead of being
    // matched to "the newest measurement still open".
    let measure_id = u64::from(started.job_id);
    if app
        .engine
        .send(AudioCommand::MeasureMix {
            measure_id,
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
/// Called from `engine_events::dispatch`. `measure_id` is the job id the
/// command carried out (ba todo #1243), so this is a direct lookup: a
/// result completes the request that asked for it, and an event whose
/// job is gone — already terminal, evicted, or a measurement the GUI
/// started rather than the control API — completes nothing at all rather
/// than handing one caller another caller's numbers.
pub(crate) fn mix_measured(
    app: &mut Resonance,
    measure_id: u64,
    results: Vec<MixMeasurement>,
) {
    let Some(JobToken::Measure { method, .. }) = app.control.jobs.live_measure(measure_id) else {
        return;
    };
    // One engine pass backs both methods; the method that asked decides
    // which shape its results are read into.
    let payload = match method {
        // `meter.measure` asks for exactly one target, so exactly one
        // measurement comes back.
        proto::MEASURE => results
            .first()
            .map(|m| measure_result(*m, app.sample_rate))
            .and_then(|r| serde_json::to_value(r).ok()),
        proto::STEMS => stems_result(app, &results).and_then(|r| serde_json::to_value(r).ok()),
        _ => return,
    };
    // An empty or master-less result set means the engine changed under
    // us; fail loudly rather than report a default that reads real.
    match payload {
        Some(payload) => app.control.jobs.complete(measure_id, payload),
        None => app
            .control
            .jobs
            .fail(measure_id, "the engine returned no usable measurement"),
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
pub(crate) fn mix_measure_error(app: &mut Resonance, measure_id: u64, message: String) {
    match app.control.jobs.live_measure(measure_id) {
        Some(_) => app.control.jobs.fail(measure_id, message),
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
        // `ABMeterTap` — the only thing that ever writes
        // `shared.mix_meter` — runs no crest and no correlation meter;
        // both fall out of `MeterSnapshot::default()` as 0.0. Passing
        // those through would be the worst kind of fabrication: 0.0 is
        // *inside* each field's plausible range, so a reader cannot tell
        // it from a measurement, and it happens to spell "maximally
        // squashed" and "perfectly wide".
        crest_db: (!live).then_some(m.crest_db),
        clipped_samples: (!live).then_some(m.clipped_samples),
        correlation: (!live).then_some(m.correlation),
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
        // Nor does the concurrency guard below: reading the streaming tap
        // contends with nothing, and since ba todo #1243 both terminal
        // events carry the job id, so a live read that overlaps a render
        // measurement gets its OWN result. Until then this had to be
        // refused too, purely so `mix_measured` could identify its job by
        // "the newest live measure" — a guard that existed to prop up the
        // correlation, not to protect anything.
        return None;
    }

    // Two offline measurements would contend for the one offline
    // renderer, which is a real conflict rather than a correlation
    // artefact: the engine's `OfflineRenderGuard` refuses the second
    // outright. A synchronous `busy` says so before a job is registered,
    // instead of handing the caller a job id that is only going to fail.
    if app.control.jobs.has_live_offline_measure() {
        return Some(RpcError::busy("a measurement is already in progress"));
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
    None
}
