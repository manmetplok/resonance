//! App-side folding of reference-track (A/B) engine events into
//! [`crate::reference::ReferenceState`]. The engine is authoritative for
//! ids, measured loudness, waveform peaks, and marker ids; these handlers
//! reconcile the optimistic GUI mirror with what the engine reports.

use resonance_audio::types::{
    ABSource, ReferenceAnalysisStage, ReferenceId, SamplePos,
};
use resonance_metering::MeterSnapshot;

use crate::reference::{AbMeters, ReferenceEntry, ReferenceMarkerState, ReferenceStatus};
use crate::Resonance;

/// Recover the queued name/path for a not-yet-registered reference id.
/// Loads are processed in dispatch order, so the oldest pending path
/// belongs to the first new id we hear about.
fn take_pending(r: &mut Resonance) -> Option<String> {
    r.reference.pending_loads.pop_front()
}

pub(super) fn analysis_progress(r: &mut Resonance, id: ReferenceId, stage: ReferenceAnalysisStage) {
    if let Some(entry) = r.reference.entry_mut(id) {
        entry.status = ReferenceStatus::Analyzing(stage);
        return;
    }
    // First we've heard of this id — register a provisional entry so the
    // view can show the "analysing…" stage before `ReferenceLoaded`.
    let path = take_pending(r).unwrap_or_default();
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_default();
    r.reference
        .entries
        .push(ReferenceEntry::analyzing(id, name, path, stage));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn loaded(
    r: &mut Resonance,
    id: ReferenceId,
    name: String,
    path: String,
    integrated_lufs: f32,
    waveform_peaks: Vec<(f32, f32)>,
) {
    if let Some(entry) = r.reference.entry_mut(id) {
        entry.name = name;
        entry.path = path;
        entry.integrated_lufs = integrated_lufs;
        entry.waveform_peaks = waveform_peaks;
        entry.status = ReferenceStatus::Loaded;
    } else {
        // No provisional entry (no analysis-progress was seen) — register
        // the finished reference directly. Drain the pending path it used.
        let _ = take_pending(r);
        r.reference.entries.push(ReferenceEntry {
            id,
            name,
            path,
            status: ReferenceStatus::Loaded,
            integrated_lufs,
            waveform_peaks,
            markers: Vec::new(),
            position_samples: 0,
        });
    }
}

pub(super) fn load_failed(r: &mut Resonance, path: String, reason: String) {
    // A failed load never allocated an id, so drop the matching pending
    // path (oldest, FIFO) and surface the reason as a dismissable notice.
    let _ = take_pending(r);
    r.reference.last_error = Some(format!("{path}: {reason}"));
}

pub(super) fn removed(r: &mut Resonance, id: ReferenceId) {
    if let Some(idx) = r.reference.index_of(id) {
        r.reference.entries.remove(idx);
    }
    if r.reference.active_id == Some(id) {
        r.reference.active_id = None;
    }
}

pub(super) fn active_changed(r: &mut Resonance, id: ReferenceId) {
    r.reference.active_id = Some(id);
}

pub(super) fn ab_source_changed(r: &mut Resonance, source: ABSource) {
    r.reference.ab_source = source;
}

pub(super) fn loudness_match_changed(r: &mut Resonance, enabled: bool, offset_db: f32) {
    r.reference.loudness_match = enabled;
    r.reference.offset_db = offset_db;
}

pub(super) fn trim_changed(r: &mut Resonance, db: f32) {
    r.reference.trim_db = db;
}

pub(super) fn marker_added(
    r: &mut Resonance,
    ref_id: ReferenceId,
    marker_id: u32,
    position_samples: SamplePos,
    label: String,
) {
    if let Some(entry) = r.reference.entry_mut(ref_id) {
        // Idempotent: the engine is authoritative for the id.
        if !entry.markers.iter().any(|mk| mk.id == marker_id) {
            entry.markers.push(ReferenceMarkerState {
                id: marker_id,
                position_samples,
                label,
            });
        }
    }
}

pub(super) fn marker_removed(r: &mut Resonance, ref_id: ReferenceId, marker_id: u32) {
    if let Some(entry) = r.reference.entry_mut(ref_id) {
        entry.markers.retain(|mk| mk.id != marker_id);
    }
}

pub(super) fn position_changed(r: &mut Resonance, ref_id: ReferenceId, position_samples: SamplePos) {
    if let Some(entry) = r.reference.entry_mut(ref_id) {
        entry.position_samples = position_samples;
    }
}

pub(super) fn loop_to_mix_changed(r: &mut Resonance, enabled: bool) {
    r.reference.loop_to_mix = enabled;
}

pub(super) fn ab_meter_snapshot(
    r: &mut Resonance,
    mix: MeterSnapshot,
    reference: Option<MeterSnapshot>,
) {
    r.reference.ab_meter = Some(AbMeters { mix, reference });
}
