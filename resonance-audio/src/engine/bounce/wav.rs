//! Offline export driver: render the whole project and feed the mix to a
//! pluggable encoder sink (doc #196).
//!
//! Includes master FX + master volume + hard-clip so the file plays back
//! identically outside the app. The render loop is format-agnostic — it
//! produces interleaved stereo `f32` frames and hands them to the active
//! [`EncoderSink`](super::encoder::EncoderSink), optionally through an
//! export-time [`ResampleStage`](super::resample::ResampleStage). The
//! default 32-bit-float WAV path is byte-for-byte the legacy `BounceToWav`
//! output.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::super::SharedState;
use super::encoder::{build_sink, EncoderError, EncoderSink};
use super::limiter::TruePeakLimiter;
use super::normalize::{target_gain_db, LoudnessMeasure};
use super::render::{
    build_latency_comp, render_chunk, reset_plugins, ChunkCtx, ChunkScratch, BOUNCE_CHUNK,
};
use super::resample::ResampleStage;

/// Which engine-event family an export run reports through. The legacy
/// `BounceToWav` shim keeps emitting `Bounce*` events (`Bounce` variant);
/// `ExportAudio` emits the generalized `Export*` events (`Export` variant)
/// carrying the encoder phase, error kind and encoded byte size.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ExportReporter {
    Bounce,
    Export,
}

impl ExportReporter {
    fn error(self, tx: &Sender<AudioEvent>, kind: ExportErrorKind, message: String) {
        let _ = match self {
            ExportReporter::Bounce => tx.send(AudioEvent::BounceError(message)),
            ExportReporter::Export => tx.send(AudioEvent::ExportError { kind, message }),
        };
    }

    fn complete(
        self,
        tx: &Sender<AudioEvent>,
        path: String,
        achieved_lufs: Option<f32>,
        achieved_dbtp: f32,
        bytes: u64,
    ) {
        let _ = match self {
            ExportReporter::Bounce => tx.send(AudioEvent::BounceComplete { path }),
            ExportReporter::Export => tx.send(AudioEvent::ExportComplete {
                path,
                achieved_lufs,
                achieved_dbtp,
                bytes,
            }),
        };
    }

    fn progress(self, tx: &Sender<AudioEvent>, phase: ExportPhase, fraction: f32) {
        // The legacy full-mix bounce emitted no progress events; only the
        // generalized export path reports them (the WAV bounce UI keys off
        // `io.bouncing`, not a fraction).
        if let ExportReporter::Export = self {
            let _ = tx.send(AudioEvent::ExportProgress { phase, fraction });
        }
    }
}

impl From<&EncoderError> for ExportErrorKind {
    fn from(e: &EncoderError) -> Self {
        match e {
            EncoderError::Unavailable(_) => ExportErrorKind::EncoderUnavailable,
            EncoderError::Io(_) => ExportErrorKind::Io,
        }
    }
}

/// libopus only operates at 48 kHz (and a few sub-rates); the export
/// pipeline always resamples to 48 kHz before the Opus sink.
const OPUS_SAMPLE_RATE: u32 = 48_000;

/// Resolve the encoded file's output sample rate: the format's requested
/// rate, falling back to the engine rate. MP3 keeps the engine rate; Opus
/// is pinned to 48 kHz so the shared resampler converts the mix before the
/// sink. Formats without a rate field otherwise keep the engine rate (they
/// error in `build_sink` before this matters).
fn output_sample_rate(format: &ExportFormat, engine_sr: u32) -> u32 {
    match *format {
        ExportFormat::Wav { sample_rate, .. } | ExportFormat::Flac { sample_rate, .. } => {
            sample_rate.unwrap_or(engine_sr)
        }
        ExportFormat::Mp3 { .. } => engine_sr,
        ExportFormat::Opus { .. } => OPUS_SAMPLE_RATE,
    }
}

/// Test surface: run the encoder-sink pipeline (export resampler + sink,
/// exactly as [`run_export`]'s tail) over a pre-rendered interleaved-stereo
/// `f32` buffer at `engine_sr`. Lets integration tests round-trip every
/// format through real encoders without booting the engine thread. Returns
/// the encoded byte size, or a user-facing error string.
#[doc(hidden)]
pub fn encode_buffer_for_test(
    format: &ExportFormat,
    metadata: &ExportMetadata,
    engine_sr: u32,
    frames: &[f32],
    path: &std::path::Path,
) -> Result<u64, String> {
    let out_sr = output_sample_rate(format, engine_sr);
    let mut resampler = if out_sr != engine_sr {
        Some(ResampleStage::new(engine_sr, out_sr).map_err(|e| e.message().to_string())?)
    } else {
        None
    };
    let mut sink = build_sink(format, out_sr, path).map_err(|e| e.message().to_string())?;
    for chunk in frames.chunks(BOUNCE_CHUNK * 2) {
        match resampler.as_mut() {
            Some(rs) => rs.process(chunk, sink.as_mut()),
            None => sink.write_frames(chunk),
        }
        .map_err(|e| e.message().to_string())?;
    }
    if let Some(mut rs) = resampler.take() {
        rs.flush(sink.as_mut()).map_err(|e| e.message().to_string())?;
    }
    sink.finalize(metadata).map_err(|e| e.message().to_string())
}

/// Test surface: run the two-pass loudness-normalization pipeline (measure
/// → gain trim + brick-wall limit → resample → sink → re-measure, exactly
/// as [`run_export`]'s normalized path) over a pre-rendered interleaved-
/// stereo `f32` buffer at `engine_sr`. Both passes see the same buffer (no
/// engine), so the analyze pass measures it and the apply pass re-renders
/// it through the gain + limiter. Returns `(bytes, achieved_lufs,
/// achieved_dbtp)` or a user-facing error string.
#[doc(hidden)]
pub fn normalize_buffer_for_test(
    format: &ExportFormat,
    normalize: &NormalizeSpec,
    metadata: &ExportMetadata,
    engine_sr: u32,
    frames: &[f32],
    path: &std::path::Path,
) -> Result<(u64, Option<f32>, f32), String> {
    // Pass 1: analyze the rendered mix.
    let mut measure = LoudnessMeasure::new(engine_sr);
    for chunk in frames.chunks(BOUNCE_CHUNK * 2) {
        measure.push(chunk);
    }
    let gain_db = target_gain_db(normalize, &measure.finish());

    // Pass 2: gain + limit, encode, re-measure the limited output.
    let out_sr = output_sample_rate(format, engine_sr);
    let mut resampler = if out_sr != engine_sr {
        Some(ResampleStage::new(engine_sr, out_sr).map_err(|e| e.message().to_string())?)
    } else {
        None
    };
    let mut sink = build_sink(format, out_sr, path).map_err(|e| e.message().to_string())?;
    let mut limiter = TruePeakLimiter::new(engine_sr as f32, gain_db, normalize.ceiling_dbtp);
    let mut remeasure = LoudnessMeasure::new(engine_sr);
    let mut stage = Vec::new();

    let mut feed = |stage: &[f32], sink: &mut dyn EncoderSink| match resampler.as_mut() {
        Some(rs) => rs.process(stage, sink),
        None => sink.write_frames(stage),
    };
    for chunk in frames.chunks(BOUNCE_CHUNK * 2) {
        stage.clear();
        limiter.process(chunk, &mut stage);
        remeasure.push(&stage);
        feed(&stage, sink.as_mut()).map_err(|e| e.message().to_string())?;
    }
    stage.clear();
    limiter.flush(&mut stage);
    remeasure.push(&stage);
    feed(&stage, sink.as_mut()).map_err(|e| e.message().to_string())?;
    drop(feed);

    if let Some(mut rs) = resampler.take() {
        rs.flush(sink.as_mut()).map_err(|e| e.message().to_string())?;
    }
    let bytes = sink.finalize(metadata).map_err(|e| e.message().to_string())?;
    let measured = remeasure.finish();
    let lufs = measured.integrated_lufs.is_finite().then_some(measured.integrated_lufs);
    Ok((bytes, lufs, measured.true_peak_dbtp))
}

/// Outcome of one chunked render pass driven by [`render_range`].
enum RenderOutcome {
    /// The whole range rendered and every chunk was consumed.
    Completed,
    /// `AudioCommand::CancelBounce` flipped the cancel flag mid-pass.
    Cancelled,
    /// The per-chunk consumer (encoder sink) failed.
    WriteError(EncoderError),
}

/// Render `[render_start, render_stop)` in `BOUNCE_CHUNK` blocks, dropping
/// the leading `comp_latency` frames of plugin-delay compensation, and
/// hand each trimmed interleaved-stereo chunk to `on_chunk`. Honours the
/// cooperative cancel flag and emits `phase` progress. Shared by the
/// single-pass export, the normalization analyze pass and the apply pass.
#[allow(clippy::too_many_arguments)]
fn render_range(
    ctx: &ChunkCtx<'_>,
    scratch: &mut ChunkScratch,
    shared: &Arc<SharedState>,
    render_start: u64,
    render_stop: u64,
    comp_latency: u64,
    reporter: ExportReporter,
    event_tx: &Sender<AudioEvent>,
    phase: ExportPhase,
    mut on_chunk: impl FnMut(&[f32]) -> Result<(), EncoderError>,
) -> RenderOutcome {
    let total_frames = (render_stop - render_start).max(1) as f32;
    let mut skip_frames = comp_latency as usize;
    let mut pos = render_start;
    let everything = |_: TrackId| true;
    while pos < render_stop {
        // Cooperative cancel — checked once per chunk so the UI's modal
        // Cancel button releases the export promptly (chunks are ~tens of
        // ms each).
        if shared.bounce_cancel.load(Ordering::Relaxed) {
            return RenderOutcome::Cancelled;
        }
        let frames = ((render_stop - pos) as usize).min(BOUNCE_CHUNK);
        render_chunk(ctx, scratch, pos, frames, &everything, true, true);

        let drop_now = skip_frames.min(frames);
        skip_frames -= drop_now;
        let out = &scratch.mix_buf[drop_now * 2..frames * 2];
        if let Err(e) = on_chunk(out) {
            return RenderOutcome::WriteError(e);
        }

        pos += frames as u64;
        reporter.progress(event_tx, phase, (pos - render_start) as f32 / total_frames);
    }
    RenderOutcome::Completed
}

/// Drop the partial output file and report a cancel. Shared by every
/// export pass so a cancel never leaves a half-rendered file behind.
fn cancel_cleanup(
    sink: Box<dyn EncoderSink>,
    path: &str,
    shared: &Arc<SharedState>,
    reporter: ExportReporter,
    event_tx: &Sender<AudioEvent>,
) {
    drop(sink);
    let _ = std::fs::remove_file(path);
    shared.bounce_cancel.store(false, Ordering::Relaxed);
    reporter.error(event_tx, ExportErrorKind::Cancelled, "Bounce cancelled".into());
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_export(
    path: String,
    settings: &ExportSettings,
    reporter: ExportReporter,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // Same guard as the realtime bounce path: the offline renderer
    // shares plugin instances with the live mixer, so rendering while
    // the transport rolls would interleave process() calls (and the
    // reset below) with live playback, corrupting both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        reporter.error(
            event_tx,
            ExportErrorKind::TransportRunning,
            "Stop transport before bouncing".into(),
        );
        return;
    }

    // Compute project range from audio clips + MIDI clips.
    let (render_start, render_end) = {
        let clips_guard = clips.read();
        let midi_guard = midi_clips.read();
        let tm = tempo_map.load();

        if clips_guard.is_empty() && midi_guard.is_empty() {
            reporter.error(event_tx, ExportErrorKind::NoAudio, "No clips to bounce".into());
            return;
        }
        let audio_start = clips_guard.iter().map(|c| c.start_sample).min();
        let audio_end = clips_guard.iter().map(|c| c.end_sample()).max();
        let midi_start = midi_guard.iter().map(|c| c.start_sample).min();
        // Tempo-aware end to match the renderer's tick_to_abs_sample
        // note scheduling under tempo changes.
        let midi_end = midi_guard
            .iter()
            .map(|c| tm.tick_to_abs_sample(c.start_sample, c.visible_duration_ticks(), sample_rate))
            .max();

        let start = audio_start.into_iter().chain(midi_start).min().unwrap_or(0);
        let end = audio_end.into_iter().chain(midi_end).max().unwrap_or(0);
        (start, end)
    };

    if render_end <= render_start {
        reporter.error(event_tx, ExportErrorKind::NoAudio, "No audio to bounce".into());
        return;
    }

    // Build the export-time resampler first (allocates nothing on disk) so
    // that if it fails we haven't created an output file to clean up.
    let out_sr = output_sample_rate(&settings.format, sample_rate);
    let mut resampler = if out_sr != sample_rate {
        match ResampleStage::new(sample_rate, out_sr) {
            Ok(r) => Some(r),
            Err(e) => {
                reporter.error(event_tx, (&e).into(), e.message().to_string());
                return;
            }
        }
    } else {
        None
    };

    // Build the encoder sink. Unavailable encoders (e.g. MP3/Opus) error
    // here *before any file is written*, so the app can offer the WAV/FLAC
    // fallback without a partial file lingering on disk.
    let mut sink = match build_sink(&settings.format, out_sr, std::path::Path::new(&path)) {
        Ok(s) => s,
        Err(e) => {
            reporter.error(event_tx, (&e).into(), e.message().to_string());
            return;
        }
    };

    let bounce_tm = (**tempo_map.load()).clone();
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let latency_comp = build_latency_comp(tracks, busses, plugins);
    // Render `max_latency` extra frames and drop the same number from
    // the front: plugin-delay compensation shifts every track by the
    // pipeline latency, so trimming it re-aligns the file with the
    // timeline (and the extra tail catches the delayed final samples).
    let comp_latency = latency_comp.max_latency();
    let render_stop = render_end + comp_latency;
    let ctx = ChunkCtx {
        shared,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map: &bounce_tm,
        sample_rate,
        master_vol,
        latency_comp: &latency_comp,
    };
    let mut scratch = ChunkScratch::new();

    // Loudness figures reported in `ExportComplete`, populated only when
    // normalization runs (`None` / `0.0` otherwise so the non-normalized
    // path keeps its old event payload).
    let normalize = settings.normalize;
    let (achieved_lufs, achieved_dbtp) = if normalize.enabled {
        // PASS 1 — analyze: render the whole range into the loudness
        // meters without writing anything. `reset_plugins` before the pass
        // makes the render deterministic so pass 2 reproduces it exactly.
        reset_plugins(plugins);
        let mut measure = LoudnessMeasure::new(sample_rate);
        match render_range(
            &ctx,
            &mut scratch,
            shared,
            render_start,
            render_stop,
            comp_latency,
            reporter,
            event_tx,
            ExportPhase::Analyze,
            |mix| {
                measure.push(mix);
                Ok(())
            },
        ) {
            RenderOutcome::Cancelled => {
                cancel_cleanup(sink, &path, shared, reporter, event_tx);
                return;
            }
            RenderOutcome::WriteError(e) => {
                reporter.error(event_tx, (&e).into(), e.message().to_string());
                return;
            }
            RenderOutcome::Completed => {}
        }
        let gain_db = target_gain_db(&normalize, &measure.finish());

        // PASS 2 — apply: re-render, trim by `gain_db`, brick-wall limit
        // true peaks to the ceiling, feed the sink, and re-measure the
        // limited output for the achieved loudness report.
        reset_plugins(plugins);
        let mut limiter = TruePeakLimiter::new(sample_rate as f32, gain_db, normalize.ceiling_dbtp);
        let mut remeasure = LoudnessMeasure::new(sample_rate);
        let mut stage: Vec<f32> = Vec::new();
        let outcome = render_range(
            &ctx,
            &mut scratch,
            shared,
            render_start,
            render_stop,
            comp_latency,
            reporter,
            event_tx,
            ExportPhase::Render,
            |mix| {
                stage.clear();
                limiter.process(mix, &mut stage);
                remeasure.push(&stage);
                match resampler.as_mut() {
                    Some(rs) => rs.process(&stage, sink.as_mut()),
                    None => sink.write_frames(&stage),
                }
            },
        );
        match outcome {
            RenderOutcome::Cancelled => {
                cancel_cleanup(sink, &path, shared, reporter, event_tx);
                return;
            }
            RenderOutcome::WriteError(e) => {
                reporter.error(event_tx, (&e).into(), e.message().to_string());
                return;
            }
            RenderOutcome::Completed => {}
        }
        // Drain the limiter's lookahead tail (the project's final frames).
        stage.clear();
        limiter.flush(&mut stage);
        remeasure.push(&stage);
        let res = match resampler.as_mut() {
            Some(rs) => rs.process(&stage, sink.as_mut()),
            None => sink.write_frames(&stage),
        };
        if let Err(e) = res {
            reporter.error(event_tx, (&e).into(), e.message().to_string());
            return;
        }

        let measured = remeasure.finish();
        let lufs = measured.integrated_lufs.is_finite().then_some(measured.integrated_lufs);
        (lufs, measured.true_peak_dbtp)
    } else {
        // Single pass — byte-for-byte the pre-normalization export.
        reset_plugins(plugins);
        let outcome = render_range(
            &ctx,
            &mut scratch,
            shared,
            render_start,
            render_stop,
            comp_latency,
            reporter,
            event_tx,
            ExportPhase::Render,
            |mix| match resampler.as_mut() {
                Some(rs) => rs.process(mix, sink.as_mut()),
                None => sink.write_frames(mix),
            },
        );
        match outcome {
            RenderOutcome::Cancelled => {
                cancel_cleanup(sink, &path, shared, reporter, event_tx);
                return;
            }
            RenderOutcome::WriteError(e) => {
                reporter.error(event_tx, (&e).into(), e.message().to_string());
                return;
            }
            RenderOutcome::Completed => {}
        }
        (None, 0.0)
    };

    // Flush any frames still buffered in the resampler, then finalize.
    if let Some(mut rs) = resampler.take() {
        if let Err(e) = rs.flush(sink.as_mut()) {
            reporter.error(event_tx, (&e).into(), e.message().to_string());
            return;
        }
    }
    reporter.progress(event_tx, ExportPhase::Encode, 1.0);
    match sink.finalize(&settings.metadata) {
        Ok(bytes) => reporter.complete(event_tx, path, achieved_lufs, achieved_dbtp, bytes),
        Err(e) => reporter.error(event_tx, (&e).into(), e.message().to_string()),
    }
}
