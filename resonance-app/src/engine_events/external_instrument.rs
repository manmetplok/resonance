//! App-side handlers mirroring external-instrument engine events into the
//! GUI `external_instruments` map (architecture doc #169, epic #39).
//!
//! The engine is the source of truth for the stored config and for device
//! status: it echoes `ExternalInstrumentChanged` after every accepted config
//! op and reports `…MidiOutOffline` / `…ReturnInputOffline` when an endpoint
//! is unreachable. These handlers keep the GUI mirror in step. The engine
//! never emits an explicit "online" event — offline flags are cleared
//! optimistically app-side when the route changes or is re-checked (see
//! `crate::update::external_instrument`).

use resonance_audio::types::TrackId;
use resonance_common::ExternalInstrument;

use crate::state::ExternalInstrumentState;
use crate::Resonance;

/// Mirror a stored / changed config. Inserts the track into the map if it
/// wasn't external yet (the engine accepted it as one), preserving any live
/// offline flags for a track that was already external.
pub(super) fn changed(r: &mut Resonance, config: ExternalInstrument) {
    r.devices.external_instruments
        .entry(config.track_id)
        .or_insert_with(|| ExternalInstrumentState::new(config.track_id))
        .apply_config(&config);
}

/// The track left external-instrument mode — drop its mirror.
pub(super) fn cleared(r: &mut Resonance, track_id: TrackId) {
    r.devices.external_instruments.remove(&track_id);
}

/// The track's MIDI output device went offline. Set the flag if the track is
/// external; an event for an unknown track is a stale race and ignored.
pub(super) fn midi_out_offline(r: &mut Resonance, track_id: TrackId) {
    if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
        state.midi_out_offline = true;
    }
}

/// The track's audio-return input device went offline.
pub(super) fn return_input_offline(r: &mut Resonance, track_id: TrackId) {
    if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
        state.return_input_offline = true;
    }
}

/// Auto-detect ("ping") measured a round-trip latency for the track: store the
/// engine's already-applied offset as the track's displayed/applied offset so
/// the inspector's latency readout reflects the measurement (doc #169, #204).
///
/// The engine emits this *after* applying the offset (it is the floored
/// `max(manual_offset, measured)`), so mirroring `latency_samples` here keeps
/// the GUI in lock-step without a second round-trip. An event for an unknown
/// track is a stale race and ignored. The engine also echoes an
/// `ExternalInstrumentChanged` carrying the same offset; making this handler
/// authoritative means the displayed offset is correct regardless of the order
/// the two events arrive in.
///
/// A successful measurement also resolves the in-flight auto-detect: the
/// `latency_detect_in_progress` guard is cleared and any stale failure reason
/// from a previous attempt is dropped (todo #1068).
pub(super) fn latency_measured(r: &mut Resonance, track_id: TrackId, latency_samples: i64) {
    if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
        state.latency_offset_samples = latency_samples;
        state.latency_detect_in_progress = false;
        state.latency_detect_error = None;
    }
}

/// Auto-detect could not measure a round-trip (MIDI out offline, no/silent
/// return, or nothing came back within the listen window). The stored offset
/// stands unchanged, but the in-flight guard is cleared and `reason` is stored
/// so the inspector can surface why the ping failed rather than leaving the
/// user waiting on a hung detect (todo #1068). An event for an unknown track
/// is a stale race and ignored.
pub(super) fn latency_detect_failed(r: &mut Resonance, track_id: TrackId, reason: String) {
    if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
        state.latency_detect_in_progress = false;
        state.latency_detect_error = Some(reason);
    }
}

/// The engine confirmed it stored a device-param map for the track
/// (`AudioCommand::SetTrackDeviceParams` → `TrackDeviceParamsApplied`, epic
/// #40, doc #201 §4). Mirror the applied param ids onto the GUI state so the
/// app has an authoritative echo of what the engine actually holds — used to
/// confirm the dispatch and to reconstruct the mirror after a project-load
/// command replay. The selected `device_id` stays the app's source of truth;
/// this only records the engine-side applied set (empty means cleared). An
/// event for a track that isn't external is a stale race and ignored.
pub(super) fn device_params_applied(
    r: &mut Resonance,
    track_id: TrackId,
    param_ids: Vec<String>,
) {
    if let Some(state) = r.devices.external_instruments.get_mut(&track_id) {
        state.applied_param_ids = param_ids;
    }
}
