//! Reference-track (A/B) engine state + command handlers.
//!
//! [`ReferencePlayer`] holds the engine-thread-local state for the
//! reference A/B feature: the loaded references (each with a decoded-PCM
//! slot, playback cursor, measured loudness + offset, and comparison
//! markers), which reference is active, the monitored source, and the
//! loudness-match / trim / loop-to-mix knobs.
//!
//! The handlers here are **stubs**: they mutate the in-memory state and
//! emit the matching [`AudioEvent`] so the GUI contract is exercised
//! end-to-end, but they perform no real decode, playback, or loudness
//! measurement yet — those land in later todos. Each handler takes the
//! player + event sender directly (not the full engine `HandlerCtx`) so
//! it can be driven headlessly from integration tests, mirroring the
//! clip fade/gain handler pattern.

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::types::{ABSource, AudioEvent, ReferenceId, ReferenceMarker, SamplePos};

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
}

/// Derive a display name for a reference from its source path's file
/// stem, falling back to the full string if there is no stem.
fn name_from_path(path: &PathBuf) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// `LoadReferenceTrack`: register a reference and emit `ReferenceLoaded`.
/// Stub — no real decode/analysis, so the loaded reference reports an
/// unmeasured loudness and an empty waveform overview.
pub fn handle_load_reference_track(
    player: &mut ReferencePlayer,
    event_tx: &Sender<AudioEvent>,
    id_hint: Option<ReferenceId>,
    path: PathBuf,
) {
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
    let entry = ReferenceEntry::new(id, name.clone(), path.clone());
    let integrated_lufs = entry.integrated_lufs;
    player.entries.push(entry);
    let _ = event_tx.send(AudioEvent::ReferenceLoaded {
        id,
        name,
        path: path.to_string_lossy().into_owned(),
        integrated_lufs,
        waveform_peaks: Vec::new(),
    });
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
