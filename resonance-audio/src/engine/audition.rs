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

use thiserror::Error;

use super::thread::HandlerCtx;
use super::SharedState;
use crate::types::{AudioEvent, EngineError, EngineErrorKind};

/// Minimum interval between throttled `AuditionPosition` events, matching the
/// ~60 Hz cadence of the main `PlayheadMoved` reporting.
const POSITION_REPORT_INTERVAL: Duration = Duration::from_millis(16);

/// Failure decoding an audition preview file. Message text matches the
/// historical literal / propagated-decoder strings.
#[derive(Debug, Error)]
pub enum AuditionError {
    #[error("audition path is not valid UTF-8")]
    InvalidPath,
    #[error(transparent)]
    Decode(#[from] resonance_common::WavDecodeError),
}

impl From<AuditionError> for EngineError {
    fn from(e: AuditionError) -> Self {
        let kind = match &e {
            AuditionError::InvalidPath => EngineErrorKind::Unsupported,
            AuditionError::Decode(_) => EngineErrorKind::Io,
        };
        EngineError::new(kind, e.to_string())
    }
}

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

/// Bit 0 of [`SharedState::audition_ctl`]: a preview is playing.
pub(crate) const AUDITION_PLAYING: u64 = 1;

/// The run generation in an `audition_ctl` word.
#[inline]
pub(crate) fn audition_gen(ctl: u64) -> u64 {
    ctl >> 1
}

#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
impl SharedState {
    /// Whether a preview is playing.
    pub fn audition_playing(&self) -> bool {
        self.audition_ctl.load(Ordering::Acquire) & AUDITION_PLAYING != 0
    }

    /// Whether the audio callback latched a natural finish the engine loop
    /// has not consumed yet.
    #[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
    pub fn audition_finish_pending(&self) -> bool {
        self.audition_finished.load(Ordering::Acquire) != 0
    }

    /// Begin a new run (`playing`) or stop (`!playing`): bump the
    /// generation so a callback block of the previous run can neither
    /// carry its position into this one nor latch its finish over it
    /// (code review RT-11). Release: everything stored before — source,
    /// start, ratio, flags — is visible to a callback that sees the new
    /// word. Returns the previous word.
    fn audition_new_run(&self, playing: bool) -> u64 {
        let bit = if playing { AUDITION_PLAYING } else { 0 };
        self.audition_ctl
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |c| {
                Some(((audition_gen(c) + 1) << 1) | bit)
            })
            .unwrap_or_else(|c| c)
    }
}

/// Publish `source` and seed the playback state so the audio callback starts
/// previewing it. `start_frame` is clamped to the source length. The source
/// and every option are stored before the run word flips to a new playing
/// generation with Release ordering, so an audio thread that Acquire-loads
/// it observes a fully-initialised state.
///
/// A restart while a preview plays is race-free (code review RT-11): the
/// audio callback used to store the position it loaded at the top of its
/// block back over the new start, and could latch the old preview's finish
/// over the new one. The new generation makes the callback restart from
/// `audition_start_bits` and its finish latch a compare-exchange that fails.
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
        .audition_start_bits
        .store(start.to_bits(), Ordering::Relaxed);
    // For the position report before the first block; the callback itself
    // reads `audition_start_bits` for a new run.
    shared
        .audition_pos_bits
        .store(start.to_bits(), Ordering::Relaxed);
    shared.audition_finished.store(0, Ordering::Relaxed);
    // The previous source (a whole sample's PCM) is retired, not dropped:
    // the overlay may be mid-block on it (code review MIX-04).
    super::retire::publish_opt(&shared.audition_source, Some(Arc::new(source)), &shared.retired);
    // Last, with Release: the audio callback gates on this word and only
    // then loads the source and options above.
    shared.audition_new_run(true);
}

/// Stop any in-flight preview and drop its source. Returns `true` when a
/// preview was actually playing, so the caller can decide whether to emit
/// `AuditionStopped` (a stop on an idle audition is a silent no-op).
pub fn stop_audition_in_place(shared: &SharedState) -> bool {
    // A callback that still sees the old playing word for a block reads a
    // source `ArcSwap` guard (which synchronises itself) and mixes one
    // extra block of preview — the same outcome as the stop landing a
    // block later. The new generation voids any finish it latches.
    let was_playing = shared.audition_new_run(false) & AUDITION_PLAYING != 0;
    super::retire::publish_opt(&shared.audition_source, None, &shared.retired);
    shared.audition_finished.store(0, Ordering::Relaxed);
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
pub fn load_audition_source(path: &Path, sample_rate: u32) -> Result<AuditionSource, AuditionError> {
    let path_str = path.to_str().ok_or(AuditionError::InvalidPath)?;
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
                .send(AudioEvent::Error(EngineError::io(format!("audition: {e}"))));
        }
    }
}

/// Engine-loop poll: fire `AuditionStopped` on a natural finish, keep the
/// sync-to-tempo ratio current as the project tempo moves, and emit
/// throttled `AuditionPosition` events for the scrub playhead.
pub(crate) fn poll_audition(ctx: &HandlerCtx, last_report: &mut Instant) {
    // Natural finish latched by the audio callback: emit once, drop the
    // source — but only for the run still current. A finish latched by a
    // block of a run that was restarted or stopped since is stale, and
    // acting on it would drop the new preview's source (RT-11).
    let finished = ctx.shared.audition_finished.swap(0, Ordering::Acquire);
    if finished != 0 {
        let ctl = ctx.shared.audition_ctl.load(Ordering::Acquire);
        if finished - 1 == audition_gen(ctl) && ctl & AUDITION_PLAYING == 0 {
            super::retire::publish_opt(&ctx.shared.audition_source, None, &ctx.shared.retired);
            let _ = ctx.event_tx.send(AudioEvent::AuditionStopped);
        }
    }

    // Acquire pairs with the Release in `start_audition_in_place`:
    // everything read below the gate (source, ratio inputs, position) is
    // published before the run word flips to playing.
    if !ctx.shared.audition_playing() {
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
