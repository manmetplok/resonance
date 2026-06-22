//! Reference-track (A/B) engine state + command handlers.
//!
//! [`ReferencePlayer`] holds the engine-thread-local state for the
//! reference A/B feature: the loaded references (each with a decoded-PCM
//! slot, playback cursor, measured loudness + offset, and comparison
//! markers), which reference is active, the monitored source, and the
//! loudness-match / trim / loop-to-mix knobs.
//!
//! Loading a reference is a two-step, asynchronous flow:
//!
//! 1. [`handle_load_reference_track`] **registers** the entry on the
//!    engine thread (so the UI can show it and the user can already
//!    select it / drop markers) and spawns a short-lived worker thread.
//! 2. The worker runs [`run_reference_analysis`]: decode the file to the
//!    engine sample rate, measure its integrated loudness (BS.1770 via
//!    [`resonance_metering::LufsMeter`]), and build a downsampled waveform
//!    overview — emitting [`AudioEvent::ReferenceAnalysisProgress`] for
//!    each stage and a final [`AudioEvent::ReferenceLoaded`] (or
//!    [`AudioEvent::ReferenceLoadFailed`] on a decode error). On success
//!    it feeds the decoded PCM + measured loudness back to the engine via
//!    an internal [`AudioCommand::ReferenceAnalyzed`], which
//!    [`handle_reference_analyzed`] stores into the registered entry.
//!
//! The remaining handlers mutate the in-memory state and emit the
//! matching [`AudioEvent`]; they take the player + event sender directly
//! (not the full engine `HandlerCtx`) so they can be driven headlessly
//! from integration tests, mirroring the clip fade/gain handler pattern.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwapOption;
use crossbeam_channel::Sender;
use resonance_metering::{LraMeter, LufsMeter, MeterSnapshot, PlrMeter, TruePeakMeter};

use crate::decode::decode_file;
use crate::types::{
    ABSource, AudioCommand, AudioEvent, ReferenceAnalysisStage, ReferenceId, ReferenceMarker,
    SamplePos,
};

/// Target size of the waveform overview emitted with a loaded reference.
/// The decoded PCM is decimated into at most this many `(min, max)` peak
/// pairs regardless of track length, so a 30-second loop and a 6-minute
/// master both render as a comparably-detailed overview (unlike the
/// fixed-bucket [`crate::types::compute_waveform_peaks`], which scales the
/// pair count with duration).
pub const REFERENCE_OVERVIEW_PEAKS: usize = 1000;

/// One loaded reference track. The decoded PCM lives behind an
/// `Option<Arc<…>>` slot that the (future) decode worker fills in; until
/// then it is `None` and playback is silent.
///
/// `name`/`path`/`pcm` are populated now but only consumed once the real
/// decode + playback path lands (a later todo); `allow(dead_code)` marks
/// them as intentional model fields rather than oversight.
#[allow(dead_code)]
pub(crate) struct ReferenceEntry {
    pub id: ReferenceId,
    pub name: String,
    pub path: PathBuf,
    /// Decoded interleaved stereo PCM at the project rate, or `None`
    /// until the decode worker fills it in.
    pub pcm: Option<Arc<Vec<f32>>>,
    /// Playback cursor within this reference, in sample frames.
    pub cursor: SamplePos,
    /// Measured integrated loudness (LUFS); `NEG_INFINITY` until analysed.
    pub integrated_lufs: f32,
    /// Gain offset (dB) applied when loudness-matching this reference to
    /// the mix; `0.0` until the offset is computed.
    pub offset_db: f32,
    /// Comparison markers placed on this reference.
    pub markers: Vec<ReferenceMarker>,
    /// Monotonic per-reference marker id allocator.
    pub next_marker_id: u32,
}

impl ReferenceEntry {
    fn new(id: ReferenceId, name: String, path: PathBuf) -> Self {
        ReferenceEntry {
            id,
            name,
            path,
            pcm: None,
            cursor: 0,
            integrated_lufs: f32::NEG_INFINITY,
            offset_db: 0.0,
            markers: Vec::new(),
            next_marker_id: 1,
        }
    }
}

/// Engine-thread-local state for the reference A/B feature.
pub struct ReferencePlayer {
    pub(crate) entries: Vec<ReferenceEntry>,
    pub(crate) active_id: Option<ReferenceId>,
    pub(crate) ab_source: ABSource,
    pub(crate) loudness_match: bool,
    pub(crate) ref_trim_db: f32,
    pub(crate) loop_to_mix: bool,
    /// Monotonic [`ReferenceId`] allocator.
    next_ref_id: u32,
}

impl Default for ReferencePlayer {
    fn default() -> Self {
        ReferencePlayer {
            entries: Vec::new(),
            active_id: None,
            ab_source: ABSource::Mix,
            loudness_match: false,
            ref_trim_db: 0.0,
            loop_to_mix: false,
            next_ref_id: 1,
        }
    }
}

impl ReferencePlayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop every loaded reference and reset the A/B controls to their
    /// defaults, including the id allocator. Used by `ClearAll` (project
    /// close / load) so a freshly loaded project's references are
    /// re-registered from id 1, matching the app-side restore. The caller
    /// must `publish` afterwards so the audio-thread monitor stops reading
    /// the dropped reference's PCM.
    pub fn clear(&mut self) {
        *self = ReferencePlayer::new();
    }

    fn entry_mut(&mut self, id: ReferenceId) -> Option<&mut ReferenceEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    fn entry(&self, id: ReferenceId) -> Option<&ReferenceEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Measured integrated loudness (LUFS) stored on a reference entry,
    /// or `None` if no such entry exists. Test accessor for the analysis
    /// fill path ([`handle_reference_analyzed`]); the value is otherwise
    /// only consumed internally (loudness matching).
    #[doc(hidden)]
    pub fn entry_integrated_lufs(&self, id: ReferenceId) -> Option<f32> {
        self.entry(id).map(|e| e.integrated_lufs)
    }

    /// Whether a reference entry has had its decoded PCM filled in yet.
    /// Test accessor for the analysis fill path.
    #[doc(hidden)]
    pub fn entry_has_pcm(&self, id: ReferenceId) -> Option<bool> {
        self.entry(id).map(|e| e.pcm.is_some())
    }

    /// Publish the current A/B state into the audio-thread
    /// [`ReferenceMonitor`]. Called from the engine control thread after
    /// a reference command mutates this player, so the cpal callback sees
    /// the new selection / gain / PCM on its next block.
    ///
    /// `reset_cursor` re-syncs the monitor's live cursor to the active
    /// entry's stored cursor — used on (re)activation, explicit seeks,
    /// and decode completion. Control-only changes (source toggle, trim,
    /// loudness, loop) pass `false` so a free-running reference isn't
    /// yanked back to the start mid-audition.
    pub fn publish(&self, monitor: &ReferenceMonitor, reset_cursor: bool) {
        let active = self.active_id.and_then(|id| self.entry(id));
        monitor
            .source_is_reference
            .store(self.ab_source == ABSource::Reference, Ordering::Relaxed);
        monitor.loop_to_mix.store(self.loop_to_mix, Ordering::Relaxed);
        monitor.pcm.store(active.and_then(|e| e.pcm.clone()));
        // gain = (loudness_match ? active offset : 0) + manual trim, dB->linear.
        let offset_db = if self.loudness_match {
            active.map(|e| e.offset_db).unwrap_or(0.0)
        } else {
            0.0
        };
        let gain = 10f32.powf((offset_db + self.ref_trim_db) / 20.0);
        monitor.gain_bits.store(gain.to_bits(), Ordering::Relaxed);
        if reset_cursor {
            monitor
                .cursor
                .store(active.map(|e| e.cursor).unwrap_or(0), Ordering::Relaxed);
        }
    }
}

/// Wait-free snapshot of the reference A/B monitor, published by the
/// engine control thread ([`ReferencePlayer::publish`]) and read
/// lock-free by the audio callback ([`ReferenceMonitor::render`]) to
/// replace the post-master monitor output with the active reference's
/// PCM. It lives in [`crate::engine::SharedState`] so the cpal callback
/// can reach it without locking the control-thread-owned
/// [`ReferencePlayer`].
///
/// The offline / realtime bounce render paths deliberately never read
/// this (see [`crate::engine::bounce`] and `bounce_realtime`), so every
/// export is the processed mix regardless of the live A/B selection.
pub struct ReferenceMonitor {
    /// `true` when the monitored source is the reference (else the mix).
    source_is_reference: AtomicBool,
    /// The active reference's decoded interleaved-stereo PCM, or `None`
    /// when nothing is active or it is still decoding.
    pcm: ArcSwapOption<Vec<f32>>,
    /// Free-running playback cursor in sample frames. The audio thread
    /// advances it each block; control-thread seeks / (re)activation
    /// overwrite it via [`ReferencePlayer::publish`].
    cursor: AtomicU64,
    /// Combined linear monitor gain (loudness-match offset + manual
    /// trim), bit-punned f32.
    gain_bits: AtomicU32,
    /// When `true`, the reference cursor follows the mix transport each
    /// block instead of free-running.
    loop_to_mix: AtomicBool,
}

impl Default for ReferenceMonitor {
    fn default() -> Self {
        ReferenceMonitor {
            source_is_reference: AtomicBool::new(false),
            pcm: ArcSwapOption::empty(),
            cursor: AtomicU64::new(0),
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
            loop_to_mix: AtomicBool::new(false),
        }
    }
}

impl ReferenceMonitor {
    /// Replace `data` (interleaved, `channels`-wide, `frames` long) with
    /// the active reference's PCM scaled by the monitor gain, advancing
    /// the playback cursor. Returns `true` when the reference was engaged
    /// and the buffer was replaced; `false` (buffer left untouched) when
    /// the monitored source is the mix or no decoded reference is active,
    /// so the caller keeps the processed mix.
    ///
    /// `playhead` is the transport position in sample frames. In
    /// loop-to-mix mode the reference is read from there each block so it
    /// stays locked to the song; otherwise it free-runs from its own
    /// cursor and wraps at the end of the reference.
    pub fn render(&self, data: &mut [f32], channels: usize, frames: usize, playhead: u64) -> bool {
        if !self.source_is_reference.load(Ordering::Relaxed) {
            return false;
        }
        let Some(pcm) = self.pcm.load_full() else {
            return false;
        };
        let total = (pcm.len() / 2) as u64;
        if total == 0 {
            return false;
        }
        let gain = f32::from_bits(self.gain_bits.load(Ordering::Relaxed));
        let loop_to_mix = self.loop_to_mix.load(Ordering::Relaxed);
        let start = if loop_to_mix {
            playhead % total
        } else {
            self.cursor.load(Ordering::Relaxed) % total
        };
        for f in 0..frames {
            let src = ((start + f as u64) % total) as usize * 2;
            let l = (pcm[src] * gain).clamp(-1.0, 1.0);
            let r = (pcm[src + 1] * gain).clamp(-1.0, 1.0);
            let base = f * channels;
            data[base] = l;
            if channels > 1 {
                data[base + 1] = r;
            }
            // Reference PCM is stereo; silence any further channels.
            for c in 2..channels {
                data[base + c] = 0.0;
            }
        }
        if !loop_to_mix {
            self.cursor
                .store((start + frames as u64) % total, Ordering::Relaxed);
        }
        true
    }

    /// Current free-run cursor (sample frames). Test accessor.
    #[doc(hidden)]
    pub fn cursor_for_test(&self) -> u64 {
        self.cursor.load(Ordering::Relaxed)
    }

    /// Whether the monitored source is currently the reference. Test
    /// accessor for the publish path.
    #[doc(hidden)]
    pub fn is_reference_for_test(&self) -> bool {
        self.source_is_reference.load(Ordering::Relaxed)
    }
}

/// How often (seconds) a short-term mean-square is pushed into the LRA
/// tracker, matching the mastering plugin's 1 Hz cadence.
const LRA_TICK_SECONDS: f32 = 1.0;

/// Streaming meter tap for one A/B signal — the processed mix or the
/// active reference. Owns the BS.1770-4 / EBU R128 measurement DSP
/// (`ref_lufs_meter` / `ref_true_peak` / `ref_lra`) and turns the audio it
/// is fed into a `Copy` [`MeterSnapshot`], so the mix tap and the
/// reference tap are the same type driven from two different signals.
///
/// Trimmed to the readouts the A/B panel compares — integrated /
/// short-term / momentary LUFS, true-peak, loudness range, and the derived
/// PLR/PSR. The two fields it doesn't measure (`correlation`, `crest_db`)
/// stay at their [`MeterSnapshot::default`] values.
///
/// Audio-thread-owned and single-threaded: [`feed_interleaved`] mutates
/// the meters in place (no locks, no allocation after [`reserve`]); the
/// control thread never touches it. Published snapshots reach the control
/// thread lock-free through an
/// [`AtomicMeterSnapshot`](resonance_metering::AtomicMeterSnapshot).
pub struct ABMeterTap {
    lufs: LufsMeter,
    true_peak: TruePeakMeter,
    lra: LraMeter,
    /// De-interleave scratch, grown to the largest block seen so the
    /// audio-thread feed path never allocates.
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    samples_since_lra: usize,
    lra_tick_samples: usize,
}

impl ABMeterTap {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            lufs: LufsMeter::new(sample_rate),
            true_peak: TruePeakMeter::new(),
            lra: LraMeter::new(),
            scratch_l: Vec::new(),
            scratch_r: Vec::new(),
            samples_since_lra: 0,
            lra_tick_samples: (LRA_TICK_SECONDS * sample_rate).max(1.0) as usize,
        }
    }

    /// Pre-size the de-interleave scratch so [`feed_interleaved`] is
    /// allocation-free for blocks up to `frames` long. Called once from the
    /// engine when the audio buffer size is known (and pre-faulted there).
    pub fn reserve(&mut self, frames: usize) {
        if self.scratch_l.len() < frames {
            self.scratch_l.resize(frames, 0.0);
            self.scratch_r.resize(frames, 0.0);
        }
    }

    /// Clear all accumulated loudness/peak/range state. Used when the
    /// metered signal restarts (e.g. a reference re-activates from its top)
    /// so a fresh integrated reading matches an offline analysis of the
    /// played material.
    pub fn reset(&mut self) {
        self.lufs.reset();
        self.true_peak.reset();
        self.lra.reset();
        self.samples_since_lra = 0;
    }

    /// Feed one interleaved block (`channels`-wide, `frames` long) to every
    /// meter. Mono input is duplicated across L/R so the BS.1770 stereo sum
    /// sees a proper pair. De-interleaves into owned scratch (grown only on
    /// the rare oversize block) then ticks the LRA tracker at ~1 Hz.
    pub fn feed_interleaved(&mut self, data: &[f32], channels: usize, frames: usize) {
        if frames == 0 || channels == 0 {
            return;
        }
        let frames = frames.min(data.len() / channels);
        if frames == 0 {
            return;
        }
        self.reserve(frames);
        let l = &mut self.scratch_l[..frames];
        let r = &mut self.scratch_r[..frames];
        if channels >= 2 {
            for f in 0..frames {
                let base = f * channels;
                l[f] = data[base];
                r[f] = data[base + 1];
            }
        } else {
            for f in 0..frames {
                let s = data[f * channels];
                l[f] = s;
                r[f] = s;
            }
        }
        self.lufs.push_stereo(l, r);
        self.true_peak.push_stereo(l, r);
        self.tick_lra(frames);
    }

    /// Push a 3 s short-term mean-square into the LRA tracker every
    /// `lra_tick_samples`, recovering the mean-square by inverting the
    /// short-term LUFS formula (same approach as the mastering plugin).
    fn tick_lra(&mut self, frames: usize) {
        self.samples_since_lra += frames;
        while self.samples_since_lra >= self.lra_tick_samples {
            self.samples_since_lra -= self.lra_tick_samples;
            let st = self.lufs.short_term_lufs();
            if st.is_finite() {
                let ms = 10.0_f64.powf((st as f64 + 0.691) / 10.0);
                self.lra.push_short_term_mean_square(ms);
            }
        }
    }

    /// Build a [`MeterSnapshot`] from the current meter state. Cheap (reads
    /// only), so the audio thread can publish one per block.
    pub fn snapshot(&self) -> MeterSnapshot {
        let (tp_l, tp_r) = self.true_peak.per_channel_dbtp();
        let tp_max = self.true_peak.peak_dbtp();
        let integrated = self.lufs.integrated_lufs();
        let short_term = self.lufs.short_term_lufs();
        let plr = PlrMeter::compute(tp_max, tp_max, integrated, short_term);
        MeterSnapshot {
            momentary_lufs: self.lufs.momentary_lufs(),
            short_term_lufs: short_term,
            integrated_lufs: integrated,
            true_peak_left_dbtp: tp_l,
            true_peak_right_dbtp: tp_r,
            true_peak_max_dbtp: tp_max,
            plr_db: plr.plr_db,
            psr_db: plr.psr_db,
            lra_lu: self.lra.lra_lu(),
            ..MeterSnapshot::default()
        }
    }
}

/// Audio-thread-owned pair of [`ABMeterTap`]s — one for the processed mix,
/// one for the active reference — held in the cpal callback's scratch and
/// passed into [`crate::mixer::mix_audio`] by `&mut`. Each block the mixer
/// feeds whichever signal it rendered and publishes that tap's snapshot
/// into [`crate::engine::SharedState`] for the control thread to poll.
pub struct ABMeters {
    pub mix: ABMeterTap,
    pub reference: ABMeterTap,
}

impl ABMeters {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            mix: ABMeterTap::new(sample_rate),
            reference: ABMeterTap::new(sample_rate),
        }
    }

    /// Pre-size both taps' de-interleave scratch for blocks up to `frames`
    /// long so neither allocates inside the realtime callback.
    pub fn reserve(&mut self, frames: usize) {
        self.mix.reserve(frames);
        self.reference.reserve(frames);
    }
}

/// Derive a display name for a reference from its source path's file
/// stem, falling back to the full string if there is no stem.
fn name_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Register a reference entry on the engine thread and return its id,
/// honouring an `id_hint` (e.g. on project load) or allocating a fresh
/// monotonic [`ReferenceId`]. The entry starts unanalysed (no PCM,
/// `NEG_INFINITY` loudness); the analysis worker fills it in later via
/// [`handle_reference_analyzed`]. Pure (no I/O, no events) so the entry
/// exists the instant the load command is dispatched.
pub fn register_reference(
    player: &mut ReferencePlayer,
    id_hint: Option<ReferenceId>,
    path: PathBuf,
) -> ReferenceId {
    let id = match id_hint {
        Some(id) => {
            // Honour the hinted id and bump the allocator past it so
            // freshly-loaded references never collide with it.
            player.next_ref_id = player.next_ref_id.max(id.0 + 1);
            id
        }
        None => {
            let id = ReferenceId(player.next_ref_id);
            player.next_ref_id += 1;
            id
        }
    };
    let name = name_from_path(&path);
    player.entries.push(ReferenceEntry::new(id, name, path));
    id
}

/// Decode + analyse a reference file end-to-end, driving the full event
/// lifecycle through `emit_event` and reporting the decoded result back
/// to the engine through `emit_cmd`. Pure over its two sinks (no threads,
/// no engine state) so it runs headlessly in tests against a temp file:
///
/// 1. `ReferenceAnalysisProgress { Decoding }` → decode + resample to
///    `sample_rate` (any workspace symphonia format).
/// 2. `ReferenceAnalysisProgress { MeasuringLufs }` → integrated LUFS via
///    [`resonance_metering::LufsMeter::analyze_offline`].
/// 3. `ReferenceAnalysisProgress { BuildingPeaks }` → ~[`REFERENCE_OVERVIEW_PEAKS`]
///    `(min, max)` overview pairs.
/// 4. `ReferenceAnalysisProgress { ComputingOffset }` → the loudness-match
///    offset is computed against the live mix loudness when the user
///    enables matching (a later todo), so this stage only marks progress
///    here; the entry's offset stays `0` until then.
/// 5. `ReferenceAnalyzed { id, pcm, integrated_lufs }` back to the engine
///    (fills the registered entry) **and** `ReferenceLoaded` to the GUI.
///
/// A decode failure emits `ReferenceLoadFailed { path, reason }` and stops
/// — the registered entry is left in place for the app to surface/remove.
pub fn run_reference_analysis(
    id: ReferenceId,
    path: &Path,
    sample_rate: u32,
    emit_event: impl Fn(AudioEvent),
    emit_cmd: impl Fn(AudioCommand),
) {
    let path_str = path.to_string_lossy().into_owned();

    emit_event(AudioEvent::ReferenceAnalysisProgress {
        id,
        stage: ReferenceAnalysisStage::Decoding,
    });
    let (pcm, name) = match decode_file(&path_str, sample_rate) {
        Ok(decoded) => decoded,
        Err(reason) => {
            emit_event(AudioEvent::ReferenceLoadFailed {
                path: path_str,
                reason,
            });
            return;
        }
    };

    emit_event(AudioEvent::ReferenceAnalysisProgress {
        id,
        stage: ReferenceAnalysisStage::MeasuringLufs,
    });
    let integrated_lufs = measure_integrated_lufs(&pcm, sample_rate);

    emit_event(AudioEvent::ReferenceAnalysisProgress {
        id,
        stage: ReferenceAnalysisStage::BuildingPeaks,
    });
    let waveform_peaks = reference_overview_peaks(&pcm);

    // The loudness-match offset depends on the *mix's* current loudness,
    // which is only available once the A/B metering tap exists (a later
    // todo). Report the stage for the UI checklist, but leave the entry's
    // offset at its `0` default until matching is actually enabled.
    emit_event(AudioEvent::ReferenceAnalysisProgress {
        id,
        stage: ReferenceAnalysisStage::ComputingOffset,
    });

    let pcm = Arc::new(pcm);
    emit_cmd(AudioCommand::ReferenceAnalyzed {
        id,
        pcm: Arc::clone(&pcm),
        integrated_lufs,
    });
    emit_event(AudioEvent::ReferenceLoaded {
        id,
        name,
        path: path_str,
        integrated_lufs,
        waveform_peaks,
    });
}

/// Measure the integrated loudness (LUFS) of stereo-interleaved PCM by
/// splitting it into left/right channels and running a one-shot
/// [`resonance_metering::LufsMeter`]. `decode_file` always yields stereo
/// interleaved, so the split is a straight even/odd deinterleave.
fn measure_integrated_lufs(interleaved: &[f32], sample_rate: u32) -> f32 {
    let frames = interleaved.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for f in 0..frames {
        left.push(interleaved[f * 2]);
        right.push(interleaved[f * 2 + 1]);
    }
    resonance_metering::LufsMeter::analyze_offline(sample_rate as f32, &left, &right).integrated
}

/// Decimate stereo-interleaved PCM into at most [`REFERENCE_OVERVIEW_PEAKS`]
/// `(min, max)` pairs over the mono mix `(L + R) / 2`, for the panel's
/// waveform overview. Bucket size scales with duration so the pair count
/// stays bounded regardless of track length; an empty buffer yields no
/// peaks.
fn reference_overview_peaks(interleaved: &[f32]) -> Vec<(f32, f32)> {
    let total_frames = interleaved.len() / 2;
    if total_frames == 0 {
        return Vec::new();
    }
    let bucket = total_frames.div_ceil(REFERENCE_OVERVIEW_PEAKS).max(1);
    let mut peaks = Vec::with_capacity(total_frames.div_ceil(bucket));
    for start in (0..total_frames).step_by(bucket) {
        let end = (start + bucket).min(total_frames);
        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;
        for f in start..end {
            let mono = (interleaved[f * 2] + interleaved[f * 2 + 1]) * 0.5;
            if mono < min_val {
                min_val = mono;
            }
            if mono > max_val {
                max_val = mono;
            }
        }
        peaks.push((min_val, max_val));
    }
    peaks
}

/// `LoadReferenceTrack`: register the reference and kick off its decode +
/// loudness analysis on a short-lived worker thread (mirroring the
/// import-to-pool path). The worker emits the analysis-progress +
/// loaded/failed events and reports the decoded PCM back via
/// `AudioCommand::ReferenceAnalyzed`. If the worker thread can't be
/// spawned the load fails up front with `ReferenceLoadFailed`.
pub fn handle_load_reference_track(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    cmd_tx: &Sender<AudioCommand>,
    sample_rate: u32,
    id_hint: Option<ReferenceId>,
    path: PathBuf,
) {
    let id = register_reference(player, id_hint, path.clone());

    let path_str = path.to_string_lossy().into_owned();
    let worker_event_tx = event_tx.clone();
    let cmd_tx = cmd_tx.clone();
    let spawn = std::thread::Builder::new()
        .name("resonance-ref-analyze".into())
        .spawn(move || {
            run_reference_analysis(
                id,
                &path,
                sample_rate,
                |ev| {
                    let _ = worker_event_tx.send(ev);
                },
                |cmd| {
                    let _ = cmd_tx.send(cmd);
                },
            );
        });
    if let Err(e) = spawn {
        let _ = event_tx.send(AudioEvent::ReferenceLoadFailed {
            path: path_str,
            reason: format!("Failed to spawn reference-analysis thread: {e}"),
        });
    }
}

/// `ReferenceAnalyzed` (engine-internal): store the decoded PCM and
/// measured loudness from the analysis worker into the registered entry.
/// A no-op if the entry was removed while it was still decoding.
pub fn handle_reference_analyzed(
    player: &mut ReferencePlayer,
    id: ReferenceId,
    pcm: Arc<Vec<f32>>,
    integrated_lufs: f32,
) {
    if let Some(entry) = player.entry_mut(id) {
        entry.pcm = Some(pcm);
        entry.integrated_lufs = integrated_lufs;
    }
}

/// `RemoveReferenceTrack`: drop a reference and its decoded PCM. Clears
/// the active selection if it pointed at the removed reference.
pub fn handle_remove_reference_track(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    id: ReferenceId,
) {
    let before = player.entries.len();
    player.entries.retain(|e| e.id != id);
    if player.entries.len() == before {
        // Unknown id — nothing removed, no event.
        return;
    }
    if player.active_id == Some(id) {
        player.active_id = None;
    }
    let _ = event_tx.send(AudioEvent::ReferenceRemoved { id });
}

/// `SetActiveReference`: choose which reference the A/B monitor auditions.
pub fn handle_set_active_reference(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    id: ReferenceId,
) {
    if player.entry(id).is_none() {
        return;
    }
    player.active_id = Some(id);
    let _ = event_tx.send(AudioEvent::ActiveReferenceChanged { id });
}

/// `SetABSource`: switch the monitored signal between mix and reference.
pub fn handle_set_ab_source(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    source: ABSource,
) {
    player.ab_source = source;
    let _ = event_tx.send(AudioEvent::ABSourceChanged { source });
}

/// `SetRefLoudnessMatch`: toggle loudness matching. Reports the offset
/// the active reference would apply (its measured `offset_db`, or `0.0`
/// when nothing is active).
pub fn handle_set_ref_loudness_match(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    enabled: bool,
) {
    player.loudness_match = enabled;
    let offset_db = player
        .active_id
        .and_then(|id| player.entry(id))
        .map(|e| e.offset_db)
        .unwrap_or(0.0);
    let _ = event_tx.send(AudioEvent::RefLoudnessMatchChanged { enabled, offset_db });
}

/// `SetRefTrim`: set the manual reference level trim (dB).
pub fn handle_set_ref_trim(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    db: f32,
) {
    player.ref_trim_db = db;
    let _ = event_tx.send(AudioEvent::RefTrimChanged { db });
}

/// `AddRefMarker`: place a comparison marker on a reference.
pub fn handle_add_ref_marker(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    ref_id: ReferenceId,
    position_samples: SamplePos,
    label: String,
) {
    let Some(entry) = player.entry_mut(ref_id) else {
        return;
    };
    let marker_id = entry.next_marker_id;
    entry.next_marker_id += 1;
    entry.markers.push(ReferenceMarker {
        id: marker_id,
        position_samples,
        label: label.clone(),
    });
    let _ = event_tx.send(AudioEvent::RefMarkerAdded {
        ref_id,
        marker_id,
        position_samples,
        label,
    });
}

/// `RemoveRefMarker`: remove a comparison marker from a reference.
pub fn handle_remove_ref_marker(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    ref_id: ReferenceId,
    marker_id: u32,
) {
    let Some(entry) = player.entry_mut(ref_id) else {
        return;
    };
    let before = entry.markers.len();
    entry.markers.retain(|m| m.id != marker_id);
    if entry.markers.len() == before {
        return;
    }
    let _ = event_tx.send(AudioEvent::RefMarkerRemoved { ref_id, marker_id });
}

/// `SetRefPosition`: seek a reference's own playback cursor.
pub fn handle_set_ref_position(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    ref_id: ReferenceId,
    position_samples: SamplePos,
) {
    let Some(entry) = player.entry_mut(ref_id) else {
        return;
    };
    entry.cursor = position_samples;
    let _ = event_tx.send(AudioEvent::RefPositionChanged {
        ref_id,
        position_samples,
    });
}

/// `SetRefLoopToMix`: toggle whether references follow the mix transport.
pub fn handle_set_ref_loop_to_mix(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    enabled: bool,
) {
    player.loop_to_mix = enabled;
    let _ = event_tx.send(AudioEvent::RefLoopToMixChanged { enabled });
}

/// `PollABMeters`: reply with the latest A/B meter snapshot. `mix` is the
/// processed-mix tap published by the audio callback (always present); the
/// reference snapshot is only meaningful when a reference is loaded, so it
/// is forwarded as `Some(reference_snapshot)` when one is active and `None`
/// otherwise. The audio thread feeds the reference tap from the reference
/// PCM *after* its loudness-match/trim gain is applied, so the reference
/// snapshot — and hence the panel's Delta against the mix — reflects the
/// level the user actually hears.
///
/// Both snapshots are loaded lock-free from
/// [`crate::engine::SharedState`] by the caller and passed in by value,
/// keeping this handler pure so it can be driven headlessly from tests.
pub fn handle_poll_ab_meters(
    player: &ReferencePlayer,
    mix: MeterSnapshot,
    reference_snapshot: MeterSnapshot,
    event_tx: &Sender<AudioEvent>,
) {
    let reference = player.active_id.map(|_| reference_snapshot);
    let _ = event_tx.send(AudioEvent::ABMeterSnapshot { mix, reference });
}
