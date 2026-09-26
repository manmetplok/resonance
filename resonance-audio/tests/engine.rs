//! `engine` test group: engine command handlers (`engine/`), driven headlessly against plain state or `EngineHandlerHarness`.
//!
//! One binary per group instead of one per file (code review ARCH-03).
//! Each of these files used to be its own integration-test target; they
//! are still ordinary test files, only the target boundary moved. Add a
//! new test as a module here (or in another group), never as a new
//! top-level `tests/*.rs` file — `tools/arch-invariants` enforces that.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! the `tests/engine/` subdirectory.

#[path = "engine/automation_handlers.rs"]
mod automation_handlers;
#[path = "engine/aux_send_cycle.rs"]
mod aux_send_cycle;
#[path = "engine/aux_send_id_duplicate_rejected.rs"]
mod aux_send_id_duplicate_rejected;
#[path = "engine/bus_id_duplicate_rejected.rs"]
mod bus_id_duplicate_rejected;
#[path = "engine/bus_plugin_move.rs"]
mod bus_plugin_move;
#[path = "engine/clip_fade_gain_handlers.rs"]
mod clip_fade_gain_handlers;
#[path = "engine/clip_warp_handlers.rs"]
mod clip_warp_handlers;
#[path = "engine/deferred_clip_commands.rs"]
mod deferred_clip_commands;
#[path = "engine/derived_clip_id_partition.rs"]
mod derived_clip_id_partition;
#[path = "engine/device_params_handler.rs"]
mod device_params_handler;
#[path = "engine/engine_error_kind.rs"]
mod engine_error_kind;
#[path = "engine/external_instrument_handlers.rs"]
mod external_instrument_handlers;
#[path = "engine/external_instrument_ping.rs"]
mod external_instrument_ping;
#[path = "engine/external_recorded_playback.rs"]
mod external_recorded_playback;
#[path = "engine/freeze_command_plumbing.rs"]
mod freeze_command_plumbing;
#[path = "engine/loop_record_takes.rs"]
mod loop_record_takes;
#[path = "engine/master_plugin_move.rs"]
mod master_plugin_move;
#[path = "engine/midi_bulk_edits.rs"]
mod midi_bulk_edits;
#[path = "engine/midi_clip_handlers.rs"]
mod midi_clip_handlers;
#[path = "engine/midi_map_command_plumbing.rs"]
mod midi_map_command_plumbing;
#[path = "engine/offline_render_gate.rs"]
mod offline_render_gate;
#[path = "engine/persist_clip_wavs.rs"]
mod persist_clip_wavs;
#[path = "engine/playback_source_handler.rs"]
mod playback_source_handler;
#[path = "engine/playhead_seek_race.rs"]
mod playhead_seek_race;
#[path = "engine/reference_handlers.rs"]
mod reference_handlers;
#[path = "engine/take_removal.rs"]
mod take_removal;
#[path = "engine/tempo_handlers.rs"]
mod tempo_handlers;
#[path = "engine/track_id_duplicate_rejected.rs"]
mod track_id_duplicate_rejected;
#[path = "engine/track_plugin_chain.rs"]
mod track_plugin_chain;
#[path = "engine/track_plugin_move.rs"]
mod track_plugin_move;
