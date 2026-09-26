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
//!
//! Shape copied from [`export_stems`][super::stem_export::export_stems]:
//! every target is rendered over ONE shared range on a worker thread, and
//! the targets run sequentially because they share the live plugin
//! instances and cannot render concurrently.
//!
//! ## Two whole-range figures that are computed here, not taken from a meter
//!
//! `crest_db` and `correlation` are computed over the whole measured range
//! by [`measure_rendered_buffer`] rather than read from
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
use indexmap::IndexMap;
use parking_lot::RwLock;

use resonance_metering::lufs::block_accumulator::BLOCK_HOP_SECS;
use resonance_metering::offline::{
    band_shares, clipped_samples, mono_penalty_db, sample_peak_db, sample_peak_linear, BandShares,
    FLOOR_DBFS,
};
use resonance_metering::{LraMeter, LufsMeter, MeterSnapshot, TruePeakMeter};

use crate::clap_host::PluginMap;
use crate::types::*;

use super::super::SharedState;
use super::stem::{render_stem, stem_project_range};
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
pub fn measure_mix(
    measure_id: u64,
    targets: Vec<StemSource>,
    range: Option<(SamplePos, SamplePos)>,
    source: MeasureSource,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<PluginMap>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
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
    let Some(_offline) = OfflineRenderGuard::try_acquire_exclusive(shared) else {
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
        stem_project_range(clips, midi_clips, tempo_map, sample_rate)
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
            tracks,
            busses,
            master,
            clips,
            midi_clips,
            plugins,
            tempo_map,
            sample_rate,
        ) {
            Ok(samples) => results.push(measure_rendered_buffer(
                target,
                start,
                end,
                &samples,
                sample_rate,
            )),
            Err(message) => {
                fail(message);
                return;
            }
        }
    }

    let _ = event_tx.send(AudioEvent::MixMeasured {
        measure_id,
        results,
    });
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
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) {
    std::thread::Builder::new()
        .name("measure-mix".into())
        .spawn(move || {
            // Panic supervision: a panicking render must still emit the
            // path's terminal error event (see `crate::supervise`). The
            // exclusive `OfflineRenderGuard` is taken inside
            // `measure_mix` and drops during the unwind, so the panic
            // path releases the renderer like the error path does.
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "measure-mix",
                || {
                    measure_mix(
                        measure_id,
                        targets,
                        range,
                        source,
                        &shared,
                        &tracks,
                        &busses,
                        &master,
                        &clips,
                        &midi_clips,
                        &plugins,
                        &tempo_map,
                        sample_rate,
                        &event_tx,
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
pub fn measure_rendered_buffer(
    target: StemSource,
    range_start: SamplePos,
    range_end: SamplePos,
    interleaved: &[f32],
    sample_rate: u32,
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

    MixMeasurement {
        target,
        source: MeasureSource::Render,
        range_start,
        range_end,
        frames: frames as u64,
        lufs_integrated: lufs.integrated_lufs(),
        lufs_short_term_max: short_term_max,
        lufs_momentary_max: momentary_max,
        lra_lu: lra.lra_lu(),
        true_peak_dbtp: true_peak.peak_dbtp(),
        sample_peak_db: sample_peak_db(&left, &right),
        crest_db: range_crest_db(&left, &right),
        clipped_samples: clipped_samples(&left, &right),
        correlation: range_correlation(&left, &right),
        mono_penalty_db: mono_penalty_db(rate, &left, &right),
        bands: band_shares(rate, &left, &right),
    }
}

/// Peak-to-RMS ratio over the WHOLE buffer, dB — the crest factor of the
/// measured range, not of a sliding window. `0.0` for silence.
///
/// Peak is `max(|L|, |R|)`, RMS is over both channels, matching
/// [`CrestMeter`][resonance_metering::CrestMeter]'s definition but with
/// the range as the window. See the module docs for why the meter itself
/// is not used here.
fn range_crest_db(left: &[f32], right: &[f32]) -> f32 {
    let n = left.len().min(right.len());
    if n == 0 {
        return 0.0;
    }
    let peak = sample_peak_linear(left, right);
    let mut sum_sq = 0.0f64;
    for i in 0..n {
        let s = left[i].abs().max(right[i].abs()) as f64;
        sum_sq += s * s;
    }
    let rms = (sum_sq / n as f64).sqrt();
    if peak <= 0.0 || rms <= 1e-20 {
        return 0.0;
    }
    20.0 * (peak as f64 / rms).log10() as f32
}

/// Pearson correlation of L against R over the WHOLE buffer, clamped to
/// `[-1, 1]`. `0.0` for a silent or single-sided buffer — the same
/// neutral value [`CorrelationMeter`][resonance_metering::CorrelationMeter]
/// reports when it has nothing to say.
fn range_correlation(left: &[f32], right: &[f32]) -> f32 {
    let n = left.len().min(right.len());
    let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let l = left[i] as f64;
        let r = right[i] as f64;
        ll += l * l;
        rr += r * r;
        lr += l * r;
    }
    let denom_sq = ll * rr;
    if denom_sq <= 1e-20 {
        return 0.0;
    }
    (lr / denom_sq.sqrt()).clamp(-1.0, 1.0) as f32
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
    }
}
