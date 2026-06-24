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
mod state;

pub(crate) use state::MidiHardwareState;

pub(crate) use clips::{
    handle_add_instrument_track, handle_add_midi_note, handle_add_vocal_track,
    handle_apply_groove_to_clip, handle_create_midi_clip, handle_delete_midi_clip,
    handle_extract_groove_from_clip, handle_humanize_midi_notes, handle_load_midi_clip_direct,
    handle_move_midi_clip, handle_move_midi_note, handle_quantize_midi_notes,
    handle_remove_midi_note, handle_resize_midi_note, handle_set_midi_note_velocity,
    handle_trim_midi_clip,
};

/// Test surface for the bulk MIDI-edit handlers (quantize / humanize /
/// groove). Exposed under `__test_support` (via `lib.rs`) so the
/// engine tests in `tests/midi_bulk_edits.rs` can drive each code path —
/// missing-clip no-op, atomic apply, selection-respecting, and the bulk
/// `MidiNotesEdited` / `GrooveExtracted` event emission — without bringing
/// up the engine thread.
pub use clips::{
    apply_groove_to_clip_in_place, extract_groove_from_clip_in_place,
    humanize_midi_notes_in_place, quantize_midi_notes_in_place,
};

/// Test surface for the MIDI clip move/trim handlers. Exposed under
/// `__test_support` (via `lib.rs`) so the regression test in
/// `tests/midi_clip_handlers.rs` can drive both code paths — missing-clip
/// no-op and happy-path — without bringing up the engine thread.
pub use clips::{move_midi_clip_in_place, trim_midi_clip_in_place};
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
    capture_loop_record_midi_pass, close_open_recordings, flush_live_note_stash,
    handle_live_control_event,
    handle_live_midi_event, handle_send_note_off, handle_send_note_on,
};
/// Test surface for the live-note contention path. Exposed under
/// `__test_support` (via `lib.rs`) so the regression test in
/// `tests/live_note_retry_order.rs` can verify that a contended-then-
/// retried NoteOn never lands after its NoteOff, without a live CLAP
/// plugin.
pub use live::deliver_or_stash;
/// Test surface for the live-input arrival → intra-block sample offset
/// conversion. Exposed under `__test_support` (via `lib.rs`) so the
/// test in `tests/live_arrival_offset.rs` can drive the pure function
/// without the engine thread.
pub use live::live_arrival_sample_offset;
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
