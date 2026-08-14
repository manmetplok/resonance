//! Live hardware-MIDI pickup on the audio thread.
//!
//! Runs first in every callback — before any branch — so a note played on
//! a controller reaches its instrument within one quantum whichever branch
//! renders the block (doc #260 finding #16).

use crate::midi_hardware::LiveMidiEvent;
use crate::types::*;

use super::midi_stash::MidiStash;

/// The instrument a live note on `track_id` plays: the first plugin of
/// a MIDI-accepting track. Mirrors the engine thread's
/// `handle_send_note_on` resolution; `None` for audio tracks or an
/// empty chain (the event is still forwarded for MIDI-thru/recording).
pub fn live_instrument_for(
    tracks: &indexmap::IndexMap<TrackId, Track>,
    track_id: TrackId,
) -> Option<PluginInstanceId> {
    let track = tracks.get(&track_id)?;
    if !track.track_type.accepts_midi() {
        return None;
    }
    track.plugins().first().copied()
}

/// Drain live hardware-MIDI events on the audio thread (doc #260
/// finding #16): queue each note straight into its instrument with a
/// real intra-block sample offset — so it renders in the *next* audio
/// block (~1 quantum) instead of waiting for the ~16 ms engine-thread
/// cadence that used to dominate live latency (and clamped every
/// offset to 0 at small quanta) — then forward the event to the engine
/// thread for recording + MIDI-thru bookkeeping.
///
/// Lock discipline: when the tracks/plugins read locks are contended
/// (a UI edit in flight) the events simply stay in the channel for the
/// next callback ~1 quantum later — never dropped, never delivered
/// twice, and plugin/bookkeeping ordering never splits across threads.
/// A contended *instrument mutex* parks the note in the mixer's
/// [`MidiStash`], exactly like timeline MIDI.
#[allow(clippy::too_many_arguments)]
pub(super) fn pickup_live_midi(
    live_midi_rx: &crossbeam_channel::Receiver<LiveMidiEvent>,
    live_midi_fwd: &crossbeam_channel::Sender<LiveMidiEvent>,
    tracks: &parking_lot::RwLock<indexmap::IndexMap<TrackId, Track>>,
    plugins: &parking_lot::RwLock<
        indexmap::IndexMap<PluginInstanceId, parking_lot::Mutex<crate::clap_host::SyncClapInstance>>,
    >,
    midi_stash: &mut MidiStash,
    sample_rate: u32,
    frames: usize,
) {
    if live_midi_rx.is_empty() {
        return;
    }
    let (Some(tracks_guard), Some(plugins_guard)) = (tracks.try_read(), plugins.try_read()) else {
        // Contended: leave the events queued; the next callback (one
        // quantum away) picks them up — still far inside the old
        // engine-cadence latency budget.
        return;
    };
    let now = std::time::Instant::now();
    for ev in live_midi_rx.try_iter() {
        let (track_id, is_note_on, note, velocity, arrival) = match &ev {
            LiveMidiEvent::InboundNoteOn {
                track_id,
                note,
                velocity,
                arrival,
            } => (*track_id, true, *note, *velocity, *arrival),
            LiveMidiEvent::InboundNoteOff {
                track_id,
                note,
                arrival,
            } => (*track_id, false, *note, 0.0, *arrival),
        };
        if let Some(inst_id) = live_instrument_for(&tracks_guard, track_id) {
            if let Some(mutex) = plugins_guard.get(&inst_id) {
                let offset = crate::engine::midi::live_arrival_sample_offset(
                    arrival,
                    now,
                    sample_rate,
                    frames,
                );
                crate::engine::midi::deliver_or_stash(
                    midi_stash,
                    inst_id,
                    mutex,
                    PendingNoteEvent {
                        is_note_on,
                        note,
                        velocity,
                        sample_offset: offset,
                    },
                );
            }
        }
        // Bookkeeping (record-into-clip, MIDI thru) stays on the engine
        // thread; a full forward channel just drops the bookkeeping,
        // never the audible note.
        let _ = live_midi_fwd.try_send(ev);
    }
}
