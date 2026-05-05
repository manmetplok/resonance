//! Engine-thread MIDI handlers, split by concern:
//!
//! - [`clips`]: instrument-track creation, MIDI clip CRUD, per-note edits.
//! - [`live`]: live note input from the GUI / hardware, recording into clips.
//! - [`hardware`]: hardware MIDI input/output device enumeration and assignment.
//! - [`outbound`]: timeline → hardware MIDI output scheduling.
//! - [`clock`]: MIDI clock master/slave (Start, Stop, Continue, SPP, 24 PPQN).
//!
//! The engine control thread (`engine::thread`) calls into these directly;
//! `mod.rs` only re-exports the per-handler entry points so callers don't
//! have to know which submodule a handler lives in.

mod clips;
mod clock;
mod hardware;
mod live;
mod outbound;

pub(crate) use clips::{
    handle_add_instrument_track, handle_add_midi_note, handle_create_midi_clip,
    handle_delete_midi_clip, handle_load_midi_clip_direct, handle_move_midi_clip,
    handle_move_midi_note, handle_remove_midi_note, handle_resize_midi_note,
    handle_set_midi_note_velocity, handle_trim_midi_clip,
};
pub(crate) use clock::{
    clock_send_continue, clock_send_song_position, clock_send_start, clock_send_stop,
    handle_midi_clock_event, handle_set_midi_clock_input, handle_set_midi_clock_output,
    poll_midi_clock_send,
};
pub(crate) use hardware::{
    handle_list_midi_inputs, handle_list_midi_outputs, handle_set_track_midi_input,
    handle_set_track_midi_output,
};
pub(crate) use live::{
    close_open_recordings, handle_live_midi_event, handle_send_note_off, handle_send_note_on,
};
pub use outbound::{outbound_step_start, OutboundStep};
pub(crate) use outbound::poll_timeline_to_midi_output;

use crate::types::TempoMap;

/// Convert an absolute sample position to an absolute tick using the
/// engine's shared tempo map. Thin wrapper over
/// [`TempoMap::sample_to_abs_tick`]. Used by the live recording and
/// outbound paths so a single helper signature is shared between them.
pub(super) fn sample_to_abs_tick(map: &TempoMap, sample_pos: u64, sample_rate: u32) -> u64 {
    map.sample_to_abs_tick(sample_pos, sample_rate)
}
