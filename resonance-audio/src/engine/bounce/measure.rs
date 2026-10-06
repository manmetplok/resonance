//! Offline mix measurement (ba todo #1218, doc #273).
//!
//! Drives [`AudioCommand::MeasureMix`]: answer loudness questions about a
//! slice of the mix in milliseconds instead of a bounce-to-disk plus an
//! external analysis pass. Nothing is written to disk and no project or
//! transport state is touched — this is a read-only query.
//!
//! It is deliberately thin. Both halves already exist:
//!
//! * the render is [`render_stem`][super::stem::render_stem] — the same
//!   in-RAM renderer the stem exporter uses, whose `stem_filter` folds a
//!   track's sub-tracks into that track and a bus's member tracks into the
//!   bus. That attribution is the whole point: measuring a multi-output
//!   instrument (`com.resonance.drums` fans out to seven ports) measures
//!   the kit, not port 0, and cannot suffer the cross-track bleed that
//!   mute-and-bounce measurement does.
//! * the numbers are `resonance-metering`'s BS.1770-4 meters plus its
//!   `offline` whole-buffer primitives (band shares, mono penalty, sample
//!   peak, clip count).
//! * the opt-in details a [`DetailSet`] asks for (warmth-width-depth.md
//!   §7.1) are `resonance_metering::detail`, read off ONE shared spectral
//!   analysis of the same rendered buffer, and only when asked.
//!
//! Shape copied from [`export_stems`][super::stem_export::export_stems]:
//! every target is rendered over ONE shared range on a worker thread, and
//! the targets run sequentially because they share the live plugin
//! instances and cannot render concurrently.
//!
//! ## Two whole-range figures that are computed here, not taken from a meter
//!
//! `crest_db` and `correlation` are computed over the whole measured range
//! by [`measure_rendered_buffer`] (through
//! `resonance_metering::offline::{range_crest_db, range_correlation}`)
//! rather than read from
//! [`CrestMeter`][resonance_metering::CrestMeter] /
//! [`CorrelationMeter`][resonance_metering::CorrelationMeter]. Those two
//! meters are 100 ms *sliding-window* readouts built for a live display:
//! their terminal value describes only the last 100 ms of whatever was
//! pushed, which for a four-minute range is the fade-out, not the mix.
//! A range report wants the range's own peak-to-RMS and L/R correlation,
//! which are the textbook definitions and six lines of arithmetic each —
//! no new DSP, just the right window. Every other figure comes straight
//! from the crate.
//!
//! [`AudioCommand::MeasureMix`]: crate::types::AudioCommand::MeasureMix

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;

use resonance_metering::detail::analyze_detail;
use resonance_metering::detail::depth::{self, SendTerm};
use resonance_metering::detail::spectrum::spectrum_detail_from;
use resonance_metering::detail::stereo::stereo_detail_from;
use resonance_metering::lufs::block_accumulator::BLOCK_HOP_SECS;
use resonance_metering::offline::{
    band_shares, clipped_samples, mono_penalty_db, range_correlation, range_crest_db,
    sample_peak_db, BandShares, FLOOR_DBFS,
};
use resonance_metering::{LraMeter, LufsMeter, MeterSnapshot, PlrMeter, TruePeakMeter};

use crate::types::*;

use super::super::SharedState;
use super::stem::{render_dry_track_stem, render_return_stem, render_stem, stem_project_range};
use super::{OfflineRenderGuard, MEASURE_BUSY_MSG};

/// Measure `targets` over one shared range and emit exactly one terminal
/// event on `event_tx`: [`AudioEvent::MixMeasured`] with one measurement
/// per target in request order, or [`AudioEvent::MixMeasureError`].
///
/// `measure_id` is the caller's opaque correlation token and is echoed on
/// whichever of those two events this call emits, on EVERY branch —
/// including the early rejections that never reach a render (ba todo
/// #1243). Nothing here interprets it.
///
/// Synchronous core, called on the worker thread spawned by
/// [`measure_mix_spawn`]. Pulled out (like
/// [`export_stems`][super::stem_export::export_stems]) so integration
/// tests can drive it directly and assert the emitted event.
///
/// Measurement is **fail-fast**: if one target fails to render, the whole
/// command reports `MixMeasureError` and no numbers. A partial result set
/// would invite comparing figures that did not all come from the same
/// pass, which is exactly the error class this command exists to remove.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
pub fn measure_mix(
    measure_id: u64,
    targets: Vec<StemSource>,
    range: Option<(SamplePos, SamplePos)>,
    source: MeasureSource,
    shared: &Arc<SharedState>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: &crate::engine::AutomationSnapshot,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    measure_mix_detailed(
        measure_id,
        targets,
        range,
        source,
        DetailSet::default(),
        shared,
        tempo_map,
        automation,
        sample_rate,
        event_tx,
    );
}

/// [`measure_mix`] with opt-in details (warmth-width-depth.md §7.1):
/// every rendered target also carries the [`MeasurementDetail`] that
/// `detail` asks for. The live path ignores `detail`.
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
pub fn measure_mix_detailed(
    measure_id: u64,
    targets: Vec<StemSource>,
    range: Option<(SamplePos, SamplePos)>,
    source: MeasureSource,
    detail: DetailSet,
    shared: &Arc<SharedState>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: &crate::engine::AutomationSnapshot,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    measure_mix_holding(
        measure_id,
        targets,
        range,
        source,
        detail,
        shared,
        tempo_map,
        automation,
        sample_rate,
        event_tx,
        None,
    );
}

/// [`measure_mix`] with the offline renderer optionally already held:
/// [`measure_mix_spawn`] takes it on the engine thread (FU-F1b) so a Play
/// can't land between the spawn and the worker's transport check.
#[allow(clippy::too_many_arguments)]
fn measure_mix_holding(
    measure_id: u64,
    targets: Vec<StemSource>,
    range: Option<(SamplePos, SamplePos)>,
    source: MeasureSource,
    detail: DetailSet,
    shared: &Arc<SharedState>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: &crate::engine::AutomationSnapshot,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
    held: Option<OfflineRenderGuard>,
) {
    let fail = |message: String| {
        let _ = event_tx.send(AudioEvent::MixMeasureError {
            measure_id,
            message,
        });
    };

    if targets.is_empty() {
        fail("No targets to measure".into());
        return;
    }

    if source == MeasureSource::Live {
        // The live tap only exists for the master mix, and there is only
        // one of it — reporting the same snapshot under a track's name
        // would be a lie, so refuse instead of silently rendering.
        if targets.len() != 1 || targets[0] != StemSource::Master {
            fail("Live measurement is only available for the master mix".into());
            return;
        }
        let _ = event_tx.send(AudioEvent::MixMeasured {
            measure_id,
            results: vec![from_live_snapshot(shared.mix_meter.load())],
        });
        return;
    }

    // -- Offline render path. --
    // A measurement must never disturb a render that is producing a file,
    // and two renders would corrupt each other's plugin state, so this is
    // the one offline path that refuses rather than queues.
    let Some(_offline) = held.or_else(|| OfflineRenderGuard::try_acquire_exclusive(shared)) else {
        fail(MEASURE_BUSY_MSG.into());
        return;
    };
    // Same guard as every other offline renderer: rendering while the
    // transport rolls interleaves process()/reset() on the shared plugin
    // instances with live playback and corrupts both.
    if shared.playing.load(Ordering::Relaxed) {
        fail("Stop transport before measuring the mix".into());
        return;
    }
    // The default range is the one the export renders — the project
    // extent plus the shared FX tail (code review ENG-07) — so the
    // reported loudness is the exported file's.
    let Some((start, end)) = range.or_else(|| {
        let graph = shared.graph.load();
        stem_project_range(&graph.clips, &graph.midi_clips, tempo_map, sample_rate)
            .map(|(s, e)| (s, e + super::super::bounce_common::offline_tail_frames(sample_rate)))
    }) else {
        fail("No audio to measure".into());
        return;
    };
    if end <= start {
        fail("Empty measurement range".into());
        return;
    }

    let mut results = Vec::with_capacity(targets.len());
    for target in targets {
        match render_stem(
            target,
            start,
            end,
            shared,
            tempo_map,
            automation,
            sample_rate,
        ) {
            Ok(samples) => results.push(measure_rendered_buffer_detailed(
                target,
                start,
                end,
                &samples,
                sample_rate,
                detail,
            )),
            Err(e) => {
                fail(e.to_string());
                return;
            }
        }
    }

    if detail.depth {
        if let Err(message) = fill_depth(
            &mut results,
            (start, end),
            shared,
            tempo_map,
            automation,
            sample_rate,
        ) {
            fail(message);
            return;
        }
    }

    let _ = event_tx.send(AudioEvent::MixMeasured {
        measure_id,
        results,
    });
}

/// Fill in every track target's DRR estimate (warmth-width-depth.md
/// §7.6, decision D5): no per-source renders, one render per RETURN.
///
/// For each return bus a measured track sends to, [`render_return_stem`]
/// renders what the return makes of its feeders' sends alone, and its
/// gain is `E_out / E_in`, with `E_in` the sum over its feeders of their
/// DRY energy ([`render_dry_track_stem`]) times the send gain squared
/// (pre-fader sends divide the source's fader back out). A feeder may be a
/// sub-track (a kit tap): its sends render like any track's, and the
/// return render keeps its parent and siblings out. Cost: one render
/// per return plus one per feeder, on top of the pass. That assumes the feeders are uncorrelated,
/// which is exact for one feeder and close for a mix. A track's DRR then
/// follows from its own sends and those gains ([`depth::drr_db_estimate`]).
///
/// Everything rendered honours automation; the send levels and a
/// pre-fader source's fader are their current static values.
fn fill_depth(
    results: &mut [MixMeasurement],
    (start, end): (SamplePos, SamplePos),
    shared: &Arc<SharedState>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: &crate::engine::AutomationSnapshot,
    sample_rate: u32,
) -> Result<(), String> {
    let sends = shared.aux_sends.load();
    let tracks = shared.tracks();
    let enabled_from = |id: TrackId| {
        sends
            .iter()
            .filter(move |s| s.enabled && s.source == SendSource::Track(id))
            .copied()
            .collect::<Vec<_>>()
    };
    let buffer_energy = |interleaved: &[f32]| {
        let (l, r): (Vec<f32>, Vec<f32>) =
            interleaved.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
        depth::mean_square(&l, &r)
    };

    // Each feeder's DRY energy, rendered once. Its ordinary stem will not
    // do: that carries the feeder's own wet path back from the returns.
    let mut energy: std::collections::HashMap<TrackId, f64> = Default::default();

    let mut returns: Vec<BusId> = results
        .iter()
        .filter_map(|m| match m.target {
            StemSource::Track(id) => Some(id),
            _ => None,
        })
        .flat_map(|id| enabled_from(id).into_iter().map(|s| s.dest))
        .collect();
    returns.sort_unstable();
    returns.dedup();

    let mut gain_db: std::collections::HashMap<BusId, Option<f64>> = Default::default();
    for bus in returns {
        let rendered =
            render_return_stem(bus, start, end, shared, tempo_map, automation, sample_rate)
                .map_err(|e| e.to_string())?;
        let Some((output, feeders)) = rendered else {
            gain_db.insert(bus, None);
            continue;
        };
        let e_out = buffer_energy(&output);
        let mut e_in = 0.0f64;
        for feeder in feeders {
            let e_feeder = match energy.get(&feeder) {
                Some(&e) => e,
                None => {
                    let stem = render_dry_track_stem(
                        feeder,
                        start,
                        end,
                        shared,
                        tempo_map,
                        automation,
                        sample_rate,
                    )
                    .map_err(|e| e.to_string())?;
                    let e = buffer_energy(&stem);
                    energy.insert(feeder, e);
                    e
                }
            };
            // The faders between the pre-fader tap and the dry stem: the
            // track's own, and for a sub-track (a kit tap) its parent's
            // group trim, which rides on the tap's route too.
            let fader_sq = tracks.get(&feeder).map_or(1.0, |t| {
                let parent = t
                    .sub_track_of
                    .and_then(|(parent, _)| tracks.get(&parent))
                    .map_or(1.0, |p| f64::from(p.volume()));
                (f64::from(t.volume()) * parent).powi(2)
            });
            for send in enabled_from(feeder).into_iter().filter(|s| s.dest == bus) {
                let level = 10f64.powf(f64::from(send.level_db) / 20.0);
                let tapped = if send.pre_fader {
                    if fader_sq > 0.0 {
                        e_feeder / fader_sq
                    } else {
                        0.0
                    }
                } else {
                    e_feeder
                };
                e_in += tapped * level * level;
            }
        }
        let gain = (e_in > 0.0 && e_out > 0.0).then(|| 10.0 * (e_out / e_in).log10());
        gain_db.insert(bus, gain);
    }

    for m in results.iter_mut() {
        let StemSource::Track(id) = m.target else {
            continue;
        };
        let Some(d) = m.detail.depth.as_mut() else {
            continue;
        };
        let own = enabled_from(id);
        let fader_db = tracks.get(&id).map_or(0.0, |t| {
            let v = f64::from(t.volume());
            if v > 0.0 {
                20.0 * v.log10()
            } else {
                -120.0
            }
        });
        d.dry_only = own.is_empty();
        d.sends = own
            .iter()
            .map(|s| DepthSend {
                bus_id: s.dest,
                send_level_db: s.level_db,
                pre_fader: s.pre_fader,
                return_gain_db: gain_db.get(&s.dest).copied().flatten().map(|g| g as f32),
            })
            .collect();
        let terms: Vec<SendTerm> = d
            .sends
            .iter()
            .filter_map(|s| {
                s.return_gain_db.map(|g| SendTerm {
                    send_level_db: f64::from(s.send_level_db),
                    return_gain_db: f64::from(g),
                    pre_fader: s.pre_fader,
                })
            })
            .collect();
        d.drr_db_estimate = depth::drr_db_estimate(fader_db, &terms).map(|v| v as f32);
    }
    Ok(())
}

/// Spawn [`measure_mix`] on a dedicated worker thread so the engine
/// dispatch loop stays responsive while the targets render; same rationale
/// as [`export_stems_spawn`][super::stem_export::export_stems_spawn].
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure_mix_spawn(
    measure_id: u64,
    targets: Vec<StemSource>,
    range: Option<(SamplePos, SamplePos)>,
    source: MeasureSource,
    detail: DetailSet,
    shared: Arc<SharedState>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: Arc<crate::engine::AutomationSnapshot>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) {
    measure_mix_spawn_after(
        measure_id,
        targets,
        range,
        source,
        detail,
        shared,
        tempo_map,
        automation,
        sample_rate,
        event_tx,
        || {},
    );
}

/// [`measure_mix_spawn`], with the worker running `before` first — a
/// test's way to park the worker before its render (it used to hold the
/// clip list's write lock for that; the list has no lock since ARCH-02
/// B-5).
#[allow(clippy::too_many_arguments)]
pub(crate) fn measure_mix_spawn_after(
    measure_id: u64,
    targets: Vec<StemSource>,
    range: Option<(SamplePos, SamplePos)>,
    source: MeasureSource,
    detail: DetailSet,
    shared: Arc<SharedState>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: Arc<crate::engine::AutomationSnapshot>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
    before: impl FnOnce() + Send + 'static,
) {
    // Take the renderer here, on the engine thread, like the file-writing
    // spawn paths (FU-F1b): the transport handlers are then ordered
    // against it, so a Play either precedes it (and the worker's
    // "stop the transport" check refuses) or is refused itself.
    let held = if source == MeasureSource::Render && !targets.is_empty() {
        match OfflineRenderGuard::try_acquire_exclusive(&shared) {
            Some(guard) => Some(guard),
            None => {
                let _ = event_tx.send(AudioEvent::MixMeasureError {
                    measure_id,
                    message: MEASURE_BUSY_MSG.into(),
                });
                return;
            }
        }
    } else {
        None
    };
    std::thread::Builder::new()
        .name("measure-mix".into())
        .spawn(move || {
            before();
            // Panic supervision: a panicking render must still emit the
            // path's terminal error event (see `crate::supervise`). The
            // exclusive `OfflineRenderGuard` moves into
            // `measure_mix_holding` and drops during the unwind, so the
            // panic path releases the renderer like the error path does.
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "measure-mix",
                || {
                    measure_mix_holding(
                        measure_id,
                        targets,
                        range,
                        source,
                        detail,
                        &shared,
                        &tempo_map,
                        &automation,
                        sample_rate,
                        &event_tx,
                        held,
                    );
                },
                |message| {
                    let _ = panic_tx.send(AudioEvent::MixMeasureError {
                        measure_id,
                        message,
                    });
                },
            );
        })
        .expect("spawn measure-mix thread");
}

/// Measure one already-rendered interleaved-stereo buffer.
///
/// Pure: buffer in, numbers out, no engine state read. Exposed so tests
/// (and any future caller with PCM in hand) can assert the measurement
/// itself without driving a render.
///
/// The buffer is fed to the BS.1770 meters in 100 ms hops — the LUFS
/// gating-block hop — so the momentary (400 ms) and short-term (3 s)
/// maxima can be sampled as they slide across the range, and the
/// short-term mean squares can be pushed into the EBU R128 loudness-range
/// tracker at the 10 Hz the spec expects. A single whole-buffer push would
/// only ever expose the windows sitting at the very end of the range.
#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
pub fn measure_rendered_buffer(
    target: StemSource,
    range_start: SamplePos,
    range_end: SamplePos,
    interleaved: &[f32],
    sample_rate: u32,
) -> MixMeasurement {
    measure_rendered_buffer_detailed(
        target,
        range_start,
        range_end,
        interleaved,
        sample_rate,
        DetailSet::default(),
    )
}

/// [`measure_rendered_buffer`] plus the opt-in details `detail` asks for.
///
/// Every detail is read off one shared spectral analysis
/// ([`analyze_detail`]), run only when some detail needs it, so a plain
/// measurement pays nothing for their existence.
pub fn measure_rendered_buffer_detailed(
    target: StemSource,
    range_start: SamplePos,
    range_end: SamplePos,
    interleaved: &[f32],
    sample_rate: u32,
    detail: DetailSet,
) -> MixMeasurement {
    let frames = interleaved.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for f in interleaved.chunks_exact(2) {
        left.push(f[0]);
        right.push(f[1]);
    }

    let rate = sample_rate as f32;
    let mut lufs = LufsMeter::new(rate);
    let mut lra = LraMeter::new();
    let mut true_peak = TruePeakMeter::new();

    let hop = ((BLOCK_HOP_SECS * rate) as usize).max(1);
    let mut momentary_max = f32::NEG_INFINITY;
    let mut short_term_max = f32::NEG_INFINITY;
    let mut pos = 0usize;
    while pos < frames {
        let stop = (pos + hop).min(frames);
        lufs.push_stereo(&left[pos..stop], &right[pos..stop]);
        true_peak.push_stereo(&left[pos..stop], &right[pos..stop]);

        let momentary = lufs.momentary_lufs();
        if momentary.is_finite() && momentary > momentary_max {
            momentary_max = momentary;
        }
        let short_term = lufs.short_term_lufs();
        if short_term.is_finite() {
            if short_term > short_term_max {
                short_term_max = short_term;
            }
            // Recover the 3 s mean square by inverting the short-term
            // LUFS formula, the same way the live A/B tap feeds LRA.
            lra.push_short_term_mean_square(10.0_f64.powf((short_term as f64 + 0.691) / 10.0));
        }
        pos = stop;
    }

    let lufs_integrated = lufs.integrated_lufs();
    let true_peak_dbtp = true_peak.peak_dbtp();
    let dynamics = detail
        .dynamics
        .then(|| PlrMeter::range(true_peak_dbtp, lufs_integrated, short_term_max));
    let decay = detail
        .decay
        .then(|| resonance_metering::decay::program_decay(&left, &right, rate));
    MixMeasurement {
        target,
        source: MeasureSource::Render,
        range_start,
        range_end,
        frames: frames as u64,
        lufs_integrated,
        lufs_short_term_max: short_term_max,
        lufs_momentary_max: momentary_max,
        lra_lu: lra.lra_lu(),
        true_peak_dbtp,
        sample_peak_db: sample_peak_db(&left, &right),
        crest_db: range_crest_db(&left, &right),
        clipped_samples: clipped_samples(&left, &right),
        correlation: range_correlation(&left, &right),
        mono_penalty_db: mono_penalty_db(rate, &left, &right),
        bands: band_shares(rate, &left, &right),
        detail: MeasurementDetail {
            dynamics,
            decay,
            ..spectral_detail(detail, rate, &left, &right)
        },
    }
}

/// The spectral details of one rendered buffer — everything read off the
/// shared analysis, which runs only when one of them is asked for.
fn spectral_detail(detail: DetailSet, rate: f32, left: &[f32], right: &[f32]) -> MeasurementDetail {
    // The assistant reads its own LTAS (mono, 1/6-octave), not the shared
    // stereo analysis below.
    let assist_ltas = detail
        .assist
        .then(|| resonance_metering::spectrum::offline::sixth_octave_ltas(rate, left, right));
    if !(detail.spectrum || detail.stereo || detail.depth) {
        return MeasurementDetail {
            assist_ltas,
            ..MeasurementDetail::default()
        };
    }
    let spec = analyze_detail(rate, left, right);
    MeasurementDetail {
        spectrum: detail.spectrum.then(|| spectrum_detail_from(&spec)),
        stereo: detail.stereo.then(|| stereo_detail_from(&spec, left, right)),
        dynamics: None,
        decay: None,
        // The DRR half needs the other targets and the returns, so
        // `fill_depth` completes it once the whole pass has rendered.
        depth: detail.depth.then(|| DepthDetail {
            hf_tilt_db: depth::hf_tilt_db(&spec),
            drr_db_estimate: None,
            dry_only: false,
            sends: Vec::new(),
        }),
        assist_ltas,
    }
}

/// Where a [`AudioCommand::MeasureAudio`] worker gets its audio.
pub(crate) enum DecodedInput {
    /// Read this file the way a clip placed from it is read.
    File(std::path::PathBuf),
    /// Already-decoded interleaved stereo at the engine rate (a loaded
    /// reference).
    Pcm(Arc<Vec<f32>>),
    /// Nothing to measure; the message says why.
    Unavailable(String),
}

/// Measure already-decoded interleaved stereo — an audio file or a
/// reference track — exactly as [`measure_rendered_buffer_detailed`]
/// measures a render, over `0..frames`, with source
/// [`MeasureSource::Decoded`] and [`StemSource::Master`] as a placeholder
/// target.
pub fn measure_decoded(interleaved: &[f32], sample_rate: u32, detail: DetailSet) -> MixMeasurement {
    let frames = (interleaved.len() / 2) as SamplePos;
    MixMeasurement {
        source: MeasureSource::Decoded,
        ..measure_rendered_buffer_detailed(
            StemSource::Master,
            0,
            frames,
            interleaved,
            sample_rate,
            detail,
        )
    }
}

/// Read `path` the way [`AudioCommand::LoadClipFromWav`] does
/// ([`ClipSource::open_wav_at_rate`]) and measure it: a pooled asset
/// measures sample-for-sample like a clip placed from it.
pub fn measure_audio_file(
    path: &std::path::Path,
    sample_rate: u32,
    detail: DetailSet,
) -> Result<MixMeasurement, EngineError> {
    let source = ClipSource::open_wav_at_rate(path, sample_rate)
        .map_err(|e| EngineError::io(format!("cannot read {}: {e}", path.display())))?;
    if source.frame_count() == 0 {
        return Err(EngineError::io(format!("{} holds no audio", path.display())));
    }
    Ok(measure_decoded(source.as_frames(), sample_rate, detail))
}

/// Run a [`AudioCommand::MeasureAudio`] on a worker thread and emit its
/// one terminal event. Renders nothing, so it takes no offline-render
/// guard.
pub(crate) fn measure_audio_spawn(
    measure_id: u64,
    input: DecodedInput,
    detail: DetailSet,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) {
    let spawned = std::thread::Builder::new()
        .name("measure-audio".into())
        .spawn({
            let event_tx = event_tx.clone();
            move || {
                let panic_tx = event_tx.clone();
                crate::supervise::run_supervised(
                    "measure-audio",
                    || {
                        let result = match input {
                            DecodedInput::File(path) => {
                                measure_audio_file(&path, sample_rate, detail)
                                    .map_err(|e| e.to_string())
                            }
                            DecodedInput::Pcm(pcm) if pcm.len() >= 2 => {
                                Ok(measure_decoded(&pcm, sample_rate, detail))
                            }
                            DecodedInput::Pcm(_) => Err("the reference holds no audio".into()),
                            DecodedInput::Unavailable(message) => Err(message),
                        };
                        let _ = event_tx.send(match result {
                            Ok(m) => AudioEvent::MixMeasured {
                                measure_id,
                                results: vec![m],
                            },
                            Err(message) => AudioEvent::MixMeasureError {
                                measure_id,
                                message,
                            },
                        });
                    },
                    |message| {
                        let _ = panic_tx.send(AudioEvent::MixMeasureError {
                            measure_id,
                            message,
                        });
                    },
                );
            }
        });
    if let Err(e) = spawned {
        let _ = event_tx.send(AudioEvent::MixMeasureError {
            measure_id,
            message: format!("could not start the measurement: {e}"),
        });
    }
}

/// Build the master measurement from the engine's live meter snapshot —
/// "what just played", with no render.
///
/// The live tap is a streaming meter, so the whole-buffer figures simply
/// do not exist on this path; they carry the placeholders documented on
/// [`MixMeasurement`], and `source` is `Live` so a consumer can tell.
fn from_live_snapshot(snapshot: MeterSnapshot) -> MixMeasurement {
    MixMeasurement {
        target: StemSource::Master,
        source: MeasureSource::Live,
        range_start: 0,
        range_end: 0,
        frames: 0,
        lufs_integrated: snapshot.integrated_lufs,
        // Instantaneous, not maxima — the live tap keeps no history.
        lufs_short_term_max: snapshot.short_term_lufs,
        lufs_momentary_max: snapshot.momentary_lufs,
        lra_lu: snapshot.lra_lu,
        true_peak_dbtp: snapshot.true_peak_max_dbtp,
        sample_peak_db: FLOOR_DBFS,
        crest_db: snapshot.crest_db,
        clipped_samples: 0,
        correlation: snapshot.correlation,
        mono_penalty_db: 0.0,
        bands: BandShares::SILENT,
        detail: MeasurementDetail::default(),
    }
}
