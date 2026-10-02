//! `plugins` test group.
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

#[path = "plugins/add_external_instrument_track.rs"]
mod add_external_instrument_track;
#[path = "plugins/add_track_menu_external_instrument.rs"]
mod add_track_menu_external_instrument;
#[path = "plugins/audition_transport_snapshot.rs"]
mod audition_transport_snapshot;
#[path = "plugins/external_instrument_detect_latency.rs"]
mod external_instrument_detect_latency;
#[path = "plugins/external_instrument_detect_latency_button.rs"]
mod external_instrument_detect_latency_button;
#[path = "plugins/external_instrument_device_persistence.rs"]
mod external_instrument_device_persistence;
#[path = "plugins/external_instrument_device_preset.rs"]
mod external_instrument_device_preset;
#[path = "plugins/external_instrument_patch_by_name.rs"]
mod external_instrument_patch_by_name;
#[path = "plugins/external_instrument_patch_picker_snapshot.rs"]
mod external_instrument_patch_picker_snapshot;
#[path = "plugins/external_instrument_persistence.rs"]
mod external_instrument_persistence;
#[path = "plugins/external_instrument_state.rs"]
mod external_instrument_state;
#[path = "plugins/drums_output_mode_subtracks.rs"]
mod drums_output_mode_subtracks;
#[path = "plugins/freeze_banner_render.rs"]
mod freeze_banner_render;
#[path = "plugins/freeze_event_mirror.rs"]
mod freeze_event_mirror;
#[path = "plugins/freeze_handlers.rs"]
mod freeze_handlers;
#[path = "plugins/freeze_persist.rs"]
mod freeze_persist;
#[path = "plugins/freeze_progress_modal.rs"]
mod freeze_progress_modal;
#[path = "plugins/freeze_readonly.rs"]
mod freeze_readonly;
#[path = "plugins/freeze_stale_on_content.rs"]
mod freeze_stale_on_content;
#[path = "plugins/frozen_track_render.rs"]
mod frozen_track_render;
#[path = "plugins/missing_plugin_slot.rs"]
mod missing_plugin_slot;
#[path = "plugins/missing_plugin_state_preserved.rs"]
mod missing_plugin_state_preserved;
#[path = "plugins/plugin_edited_by_plugin.rs"]
mod plugin_edited_by_plugin;
#[path = "plugins/plugin_live_values.rs"]
mod plugin_live_values;
#[path = "plugins/plugin_param_undo.rs"]
mod plugin_param_undo;
#[path = "plugins/plugin_state_excluded_params.rs"]
mod plugin_state_excluded_params;
#[path = "plugins/settings_plugin_rescan_button.rs"]
mod settings_plugin_rescan_button;
#[path = "plugins/track_freeze_menu.rs"]
mod track_freeze_menu;
#[path = "plugins/track_preset_save_prompt.rs"]
mod track_preset_save_prompt;
