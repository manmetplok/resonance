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
use std::sync::Arc;

use crossbeam_channel::Sender;

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

/// `PollABMeters`: reply with an A/B meter snapshot. Stub — emits
/// default (silent) meters; real metering lands with the playback path.
pub fn handle_poll_ab_meters(player: &ReferencePlayer, event_tx: &Sender<AudioEvent>) {
    use resonance_metering::MeterSnapshot;
    let reference = player
        .active_id
        .map(|_| MeterSnapshot::default());
    let _ = event_tx.send(AudioEvent::ABMeterSnapshot {
        mix: MeterSnapshot::default(),
        reference,
    });
}
