//! Audition preview playback (doc #175).
//!
//! Audition lets the engine preview an arbitrary audio file — a pool asset
//! or an un-imported file straight off the filesystem — *without* touching
//! the arrangement, transport, or undo history. It is deliberately transient:
//! never serialized, never an [`AudioClip`], and it does not move the main
//! playhead.
//!
//! The decoded preview audio plus its playback state live in
//! [`SharedState`](super::SharedState) — already shared between the engine
//! control thread and the cpal audio callback — so no extra channel or `Arc`
//! plumbing is needed:
//!
//! - The engine thread decodes the file off the audio thread, publishes the
//!   samples via the wait-free `audition_source` [`ArcSwapOption`], and seeds
//!   the playback flags. It also recomputes the sync-to-tempo ratio when the
//!   project tempo moves, throttles `AuditionPosition` events for the scrub
//!   playhead, and emits `AuditionStopped` when the audio thread reports a
//!   natural finish.
//! - The audio callback ([`crate::mixer`]) reads the published source and
//!   the atomic playback state each block and mixes the preview into the
//!   output buffer, advancing its own audition playhead. On a non-looping
//!   run that reaches the end it latches `audition_finished` so the engine
//!   thread can fire `AuditionStopped` exactly once.
//!
//! **Sync-to-tempo is varispeed (resampling), not pitch-preserving.** The
//! workspace has no time-stretch DSP, so when `sync_to_tempo` is on the
//! preview is resampled so its loop length snaps to a whole number of beats
//! at the project BPM — which shifts pitch with the speed change. This is a
//! pragmatic preview behaviour; a true pitch-preserving stretch is out of
//! scope for the audition path.
//!
//! [`ArcSwapOption`]: arc_swap::ArcSwapOption

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::thread::HandlerCtx;
use super::SharedState;
use crate::types::AudioEvent;

/// Minimum interval between throttled `AuditionPosition` events, matching the
/// ~60 Hz cadence of the main `PlayheadMoved` reporting.
const POSITION_REPORT_INTERVAL: Duration = Duration::from_millis(16);

/// A decoded audition preview source: stereo-interleaved f32 samples at the
/// engine sample rate. Published behind an `ArcSwapOption` so the audio
/// callback can pick it up wait-free.
#[derive(Debug, Clone)]
pub struct AuditionSource {
    /// Stereo-interleaved f32 samples (`[l, r]` per frame) at `sample_rate`.
    pub samples: Vec<f32>,
    /// Number of stereo frames in `samples`.
    pub frame_count: u64,
    /// Sample rate of `samples` — always the engine rate, since
    /// [`load_audition_source`] resamples on decode. Retained so the
    /// sync-to-tempo ratio can be recomputed when the project BPM moves.
    pub sample_rate: u32,
}

impl AuditionSource {
    /// Build a source from already-decoded engine-rate stereo samples.
    pub fn from_samples(samples: Vec<f32>, sample_rate: u32) -> Self {
        let frame_count = (samples.len() / 2) as u64;
        Self {
            samples,
            frame_count,
            sample_rate,
        }
    }
}

/// Playback-rate ratio (source frames advanced per output frame) for a
/// preview. `1.0` plays at natural speed.
///
/// With `sync_to_tempo` on, the source's natural duration is snapped to the
/// nearest whole number of beats at `bpm` and the ratio scales playback so
/// the loop fills exactly that many beats — i.e. a varispeed tempo-lock.
/// Returns `1.0` unchanged when sync is off or any input is degenerate.
pub fn compute_sync_ratio(natural_frames: u64, sample_rate: u32, bpm: f64, sync: bool) -> f32 {
    if !sync || natural_frames == 0 || sample_rate == 0 || bpm <= 0.0 {
        return 1.0;
    }
    let dur_secs = natural_frames as f64 / sample_rate as f64;
    let beats_natural = dur_secs * bpm / 60.0;
    let target_beats = beats_natural.round().max(1.0);
    (beats_natural / target_beats) as f32
}

/// Publish `source` and seed the playback state so the audio callback starts
/// previewing it. `start_frame` is clamped to the source length. The source
/// and every flag are stored before `audition_playing` flips true, so the
/// audio thread only ever observes a fully-initialised state.
pub fn start_audition_in_place(
    shared: &SharedState,
    source: AuditionSource,
    start_frame: u64,
    bpm: f64,
    loop_enabled: bool,
    sync_to_tempo: bool,
) {
    let ratio = compute_sync_ratio(source.frame_count, source.sample_rate, bpm, sync_to_tempo);
    let start = start_frame.min(source.frame_count) as f64;
    shared.audition_loop.store(loop_enabled, Ordering::Relaxed);
    shared.audition_sync.store(sync_to_tempo, Ordering::Relaxed);
    shared
        .audition_ratio_bits
        .store(ratio.to_bits(), Ordering::Relaxed);
    shared
        .audition_pos_bits
        .store(start.to_bits(), Ordering::Relaxed);
    shared.audition_finished.store(false, Ordering::Relaxed);
    shared.audition_source.store(Some(Arc::new(source)));
    // Flip playing last: the audio callback checks this first and only
    // then loads the source + flags above.
    shared.audition_playing.store(true, Ordering::Relaxed);
}

/// Stop any in-flight preview and drop its source. Returns `true` when a
/// preview was actually playing, so the caller can decide whether to emit
/// `AuditionStopped` (a stop on an idle audition is a silent no-op).
pub fn stop_audition_in_place(shared: &SharedState) -> bool {
    let was_playing = shared.audition_playing.swap(false, Ordering::Relaxed);
    shared.audition_source.store(None);
    shared.audition_finished.store(false, Ordering::Relaxed);
    was_playing
}

/// Update the loop / sync-to-tempo options for the current (or next) preview
/// and recompute the playback ratio against the loaded source, if any. The
/// options persist across `AuditionFile` commands so they can be set before
/// or after the file is chosen.
pub fn set_audition_options_in_place(
    shared: &SharedState,
    bpm: f64,
    loop_enabled: bool,
    sync_to_tempo: bool,
) {
    shared.audition_loop.store(loop_enabled, Ordering::Relaxed);
    shared.audition_sync.store(sync_to_tempo, Ordering::Relaxed);
    let guard = shared.audition_source.load();
    if let Some(src) = guard.as_ref() {
        let ratio = compute_sync_ratio(src.frame_count, src.sample_rate, bpm, sync_to_tempo);
        shared
            .audition_ratio_bits
            .store(ratio.to_bits(), Ordering::Relaxed);
    }
}

/// Decode an audio file (any format the workspace `symphonia` features
/// enable) to engine-rate stereo and wrap it as an [`AuditionSource`].
pub fn load_audition_source(path: &Path, sample_rate: u32) -> Result<AuditionSource, String> {
    let path_str = path
        .to_str()
        .ok_or_else(|| "audition path is not valid UTF-8".to_string())?;
    let (samples, _name) = crate::decode::decode_file(path_str, sample_rate)?;
    Ok(AuditionSource::from_samples(samples, sample_rate))
}

/// `AudioCommand::AuditionFile` handler: decode `path` on the engine thread
/// (off the audio callback), then start previewing it from `start_frame`
/// using the currently-set loop / sync options. A decode failure surfaces as
/// `AudioEvent::Error` and leaves any existing preview untouched.
pub(crate) fn handle_audition_file(ctx: &HandlerCtx, path: std::path::PathBuf, start_frame: u64) {
    match load_audition_source(&path, ctx.sample_rate) {
        Ok(source) => {
            let bpm = ctx.tempo_map.load().bpm as f64;
            let loop_enabled = ctx.shared.audition_loop.load(Ordering::Relaxed);
            let sync = ctx.shared.audition_sync.load(Ordering::Relaxed);
            start_audition_in_place(ctx.shared, source, start_frame, bpm, loop_enabled, sync);
        }
        Err(e) => {
            let _ = ctx
                .event_tx
                .send(AudioEvent::Error(format!("audition: {e}")));
        }
    }
}

/// Engine-loop poll: fire `AuditionStopped` on a natural finish, keep the
/// sync-to-tempo ratio current as the project tempo moves, and emit
/// throttled `AuditionPosition` events for the scrub playhead.
pub(crate) fn poll_audition(ctx: &HandlerCtx, last_report: &mut Instant) {
    // Natural finish latched by the audio callback: emit once, drop the
    // source. Runs even though `audition_playing` is already false.
    if ctx.shared.audition_finished.swap(false, Ordering::Relaxed) {
        ctx.shared.audition_source.store(None);
        let _ = ctx.event_tx.send(AudioEvent::AuditionStopped);
    }

    if !ctx.shared.audition_playing.load(Ordering::Relaxed) {
        return;
    }

    // Track the project tempo while previewing a tempo-synced loop.
    if ctx.shared.audition_sync.load(Ordering::Relaxed) {
        let guard = ctx.shared.audition_source.load();
        if let Some(src) = guard.as_ref() {
            let bpm = ctx.tempo_map.load().bpm as f64;
            let ratio = compute_sync_ratio(src.frame_count, src.sample_rate, bpm, true);
            ctx.shared
                .audition_ratio_bits
                .store(ratio.to_bits(), Ordering::Relaxed);
        }
    }

    if last_report.elapsed() >= POSITION_REPORT_INTERVAL {
        *last_report = Instant::now();
        let pos = f64::from_bits(ctx.shared.audition_pos_bits.load(Ordering::Relaxed));
        let frame = pos.max(0.0) as u64;
        let _ = ctx.event_tx.send(AudioEvent::AuditionPosition { frame });
    }
}
