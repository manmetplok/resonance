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
