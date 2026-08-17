//! `performance` test group.
//!
//! One binary per group instead of one per file. Each of these files used
//! to be its own integration-test target, and every target re-monomorphizes
//! the whole app + iced generic surface — 240 of them cost 193s to relink
//! after a one-line change (ba doc #285 §3, todo #1368). They are still
//! ordinary test files; only the target boundary moved.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! a `tests/<group>/` subdirectory.

#[path = "common/mod.rs"]
mod common;

#[path = "performance/commands_registry.rs"]
mod commands_registry;
#[path = "performance/keycap_styles.rs"]
mod keycap_styles;
#[path = "performance/performance_beat_cue.rs"]
mod performance_beat_cue;
#[path = "performance/performance_center_stage.rs"]
mod performance_center_stage;
#[path = "performance/performance_center_stage_render.rs"]
mod performance_center_stage_render;
#[path = "performance/performance_chord_readout.rs"]
mod performance_chord_readout;
#[path = "performance/performance_mode_toggle.rs"]
mod performance_mode_toggle;
#[path = "performance/performance_next_lane.rs"]
mod performance_next_lane;
#[path = "performance/performance_next_lane_render.rs"]
mod performance_next_lane_render;
#[path = "performance/performance_persistence.rs"]
mod performance_persistence;
#[path = "performance/performance_scaffold.rs"]
mod performance_scaffold;
#[path = "performance/performance_section_readout.rs"]
mod performance_section_readout;
#[path = "performance/performance_state.rs"]
mod performance_state;
#[path = "performance/playback_source_inspector.rs"]
mod playback_source_inspector;
#[path = "performance/playback_source_mirror.rs"]
mod playback_source_mirror;
#[path = "performance/playback_source_persist.rs"]
mod playback_source_persist;
#[path = "performance/reference_events.rs"]
mod reference_events;
#[path = "performance/reference_handlers.rs"]
mod reference_handlers;
#[path = "performance/reference_panel_analyzing.rs"]
mod reference_panel_analyzing;
#[path = "performance/reference_panel_error.rs"]
mod reference_panel_error;
#[path = "performance/reference_panel_loudness.rs"]
mod reference_panel_loudness;
#[path = "performance/reference_panel_populated.rs"]
mod reference_panel_populated;
#[path = "performance/reference_panel_scaffold.rs"]
mod reference_panel_scaffold;
#[path = "performance/reference_persistence.rs"]
mod reference_persistence;
#[path = "performance/reference_undo.rs"]
mod reference_undo;
#[path = "performance/remote_indicator_snapshot.rs"]
mod remote_indicator_snapshot;
