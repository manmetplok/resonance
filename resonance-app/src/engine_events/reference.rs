//! App-side folding of reference-track (A/B) engine events into
//! [`crate::reference::ReferenceState`]. The engine is authoritative for
//! measured loudness and waveform peaks (the app allocates reference and
//! marker ids); these handlers reconcile the optimistic GUI mirror with
//! what the engine reports.

use resonance_audio::types::{
    ABSource, ReferenceAnalysisStage, ReferenceId, SamplePos,
};
use resonance_metering::MeterSnapshot;

use crate::reference::{AbMeters, ReferenceEntry, ReferenceMarkerState, ReferenceStatus};
use crate::Resonance;

/// Whether an event for an unlisted `id` should register it. A load the
/// app sent is listed from the start, so an unlisted id is either the
/// stale echo of one it has dropped — the analysis worker of a removed,
/// undone or superseded reference runs on and still reports (FU-A5b) —
/// or one the app never issued, which is taken as it comes.
fn registers(r: &mut Resonance, id: ReferenceId) -> bool {
    if r.reference.is_stale(id) {
        return false;
    }
    r.reference.saw_engine_id(id);
    true
}

pub(super) fn analysis_progress(r: &mut Resonance, id: ReferenceId, stage: ReferenceAnalysisStage) {
    if let Some(entry) = r.reference.entry_mut(id) {
        entry.status = ReferenceStatus::Analyzing(stage);
        return;
    }
    // An id the app never issued: register a provisional entry so the
    // view can show the "analysing…" stage before `ReferenceLoaded`.
    if registers(r, id) {
        r.reference.entries.push(ReferenceEntry::analyzing(
            id,
            String::new(),
            String::new(),
            stage,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn loaded(
    r: &mut Resonance,
    id: ReferenceId,
    name: String,
    path: String,
    integrated_lufs: f32,
    waveform_peaks: Vec<(f32, f32)>,
    length_samples: u64,
) {
    // `reference.load` lists a pooled asset under the asset's name, not
    // its `asset_N` file's; any other load takes the engine's.
    let name = r.control.reference_names.remove(&id).unwrap_or(name);
    if let Some(entry) = r.reference.entry_mut(id) {
        entry.name = name;
        entry.path = path;
        entry.integrated_lufs = integrated_lufs;
        entry.waveform_peaks = waveform_peaks;
        entry.length_samples = length_samples;
        entry.status = ReferenceStatus::Loaded;
    } else {
        // Not listed: register the finished reference directly, unless it
        // is a stale echo.
        if !registers(r, id) {
            return;
        }
        r.reference.entries.push(ReferenceEntry {
            id,
            name,
            path,
            status: ReferenceStatus::Loaded,
            integrated_lufs,
            waveform_peaks,
            markers: Vec::new(),
            position_samples: 0,
            length_samples,
        });
    }
}

pub(super) fn load_failed(r: &mut Resonance, path: String, reason: String) {
    // The failure carries no id: mark the oldest still-analysing entry of
    // that path as failed, so it stops showing a spinner, and surface the
    // reason as a dismissable notice.
    if let Some(entry) = r
        .reference
        .entries
        .iter_mut()
        .find(|e| e.path == path && matches!(e.status, ReferenceStatus::Analyzing(_)))
    {
        entry.status = ReferenceStatus::Error(reason.clone());
    }
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
    r.reference.monitor.ab_source = source;
}

pub(super) fn loudness_match_changed(r: &mut Resonance, enabled: bool, offset_db: f32) {
    r.reference.loudness_match = enabled;
    r.reference.monitor.offset_db = offset_db;
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
        // Idempotent: the app allocated the id and listed the marker
        // when it sent `AddRefMarker`; this is the echo.
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
    r.reference.monitor.loop_to_mix = enabled;
}

pub(super) fn ab_meter_snapshot(
    r: &mut Resonance,
    mix: MeterSnapshot,
    reference: Option<MeterSnapshot>,
) {
    r.reference.monitor.ab_meter = Some(AbMeters { mix, reference });
}
