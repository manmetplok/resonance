//! `midi_hw` test group: hardware MIDI in/out, MIDI clock and SMF (`midi_hardware`, `midi_io`, `midi_clock`).
//!
//! One binary per group instead of one per file (code review ARCH-03).
//! Each of these files used to be its own integration-test target; they
//! are still ordinary test files, only the target boundary moved. Add a
//! new test as a module here (or in another group), never as a new
//! top-level `tests/*.rs` file — `tools/arch-invariants` enforces that.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! the `tests/midi_hw/` subdirectory.

#[path = "midi_hw/control_surface_parse.rs"]
mod control_surface_parse;
#[path = "midi_hw/device_param_automation.rs"]
mod device_param_automation;
#[path = "midi_hw/live_arrival_offset.rs"]
mod live_arrival_offset;
#[path = "midi_hw/live_note_retry_order.rs"]
mod live_note_retry_order;
#[path = "midi_hw/midi_clock_parse.rs"]
mod midi_clock_parse;
#[path = "midi_hw/midi_hardware_emit.rs"]
mod midi_hardware_emit;
#[path = "midi_hw/midi_hardware_parse.rs"]
mod midi_hardware_parse;
#[path = "midi_hw/live_record_stop.rs"]
mod live_record_stop;
#[path = "midi_hw/midi_io.rs"]
mod midi_io;
#[path = "midi_hw/midi_program_change.rs"]
mod midi_program_change;
#[path = "midi_hw/outbound_note_pairing.rs"]
mod outbound_note_pairing;
#[path = "midi_hw/outbound_step_start.rs"]
mod outbound_step_start;
#[path = "midi_hw/smf_import.rs"]
mod smf_import;
