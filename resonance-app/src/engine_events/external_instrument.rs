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
    r.external_instruments
        .entry(config.track_id)
        .or_insert_with(|| ExternalInstrumentState::new(config.track_id))
        .apply_config(&config);
}

/// The track left external-instrument mode — drop its mirror.
pub(super) fn cleared(r: &mut Resonance, track_id: TrackId) {
    r.external_instruments.remove(&track_id);
}

/// The track's MIDI output device went offline. Set the flag if the track is
/// external; an event for an unknown track is a stale race and ignored.
pub(super) fn midi_out_offline(r: &mut Resonance, track_id: TrackId) {
    if let Some(state) = r.external_instruments.get_mut(&track_id) {
        state.midi_out_offline = true;
    }
}

/// The track's audio-return input device went offline.
pub(super) fn return_input_offline(r: &mut Resonance, track_id: TrackId) {
    if let Some(state) = r.external_instruments.get_mut(&track_id) {
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
pub(super) fn latency_measured(r: &mut Resonance, track_id: TrackId, latency_samples: i64) {
    if let Some(state) = r.external_instruments.get_mut(&track_id) {
        state.latency_offset_samples = latency_samples;
    }
}

/// Auto-detect could not measure a round-trip (MIDI out offline, no/silent
/// return, or nothing came back within the listen window). Nothing in the
/// mirror changes — the existing offset stands — but reporting it here (rather
/// than silently dropping the event) is the join point for any future
/// inspector "auto-detect failed" surfacing. Kept as an explicit no-op so the
/// dispatch match stays exhaustive and the intent is documented.
pub(super) fn latency_detect_failed(_r: &mut Resonance, _track_id: TrackId) {}
