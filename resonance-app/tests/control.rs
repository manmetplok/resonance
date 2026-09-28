//! `control` test group.
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

#[path = "control/control_arrangement_bars.rs"]
mod control_arrangement_bars;
#[path = "control/control_arrangement_global_events.rs"]
mod control_arrangement_global_events;
#[path = "control/control_automation.rs"]
mod control_automation;
#[path = "control/control_automation_render.rs"]
mod control_automation_render;
#[path = "control/control_automation_shape.rs"]
mod control_automation_shape;
#[path = "control/control_automation_edit.rs"]
mod control_automation_edit;
#[path = "control/control_bus.rs"]
mod control_bus;
#[path = "control/control_bus_create_commit.rs"]
mod control_bus_create_commit;
#[path = "control/control_bus_effects.rs"]
mod control_bus_effects;
#[path = "control/control_color_plugin.rs"]
mod control_color_plugin;
#[path = "control/control_chain_presets.rs"]
mod control_chain_presets;
#[path = "control/control_clip_confirm_place_guard.rs"]
mod control_clip_confirm_place_guard;
#[path = "control/control_clip_place.rs"]
mod control_clip_place;
#[path = "control/control_clip_split.rs"]
mod control_clip_split;
#[path = "control/control_edit_undo.rs"]
mod control_edit_undo;
#[path = "control/control_endpoint.rs"]
mod control_endpoint;
#[path = "control/control_external_instrument.rs"]
mod control_external_instrument;
#[path = "control/control_generate.rs"]
mod control_generate;
#[path = "control/control_generate_after_load.rs"]
mod control_generate_after_load;
#[path = "control/control_global_events.rs"]
mod control_global_events;
#[path = "control/control_import_midi.rs"]
mod control_import_midi;
#[path = "control/control_jobs.rs"]
mod control_jobs;
#[path = "control/control_lane_generator.rs"]
mod control_lane_generator;
#[path = "control/control_master.rs"]
mod control_master;
#[path = "control/control_master_params.rs"]
mod control_master_params;
#[path = "control/control_meter.rs"]
mod control_meter;
#[path = "control/control_mixer_volume_db.rs"]
mod control_mixer_volume_db;
#[path = "control/control_mutation_gate.rs"]
mod control_mutation_gate;
#[path = "control/control_mutation_gate_loading.rs"]
mod control_mutation_gate_loading;
#[path = "control/control_notes.rs"]
mod control_notes;
#[path = "control/control_notes_bulk.rs"]
mod control_notes_bulk;
#[path = "control/control_notes_confirm_caps.rs"]
mod control_notes_confirm_caps;
#[path = "control/control_notes_create_insert_race.rs"]
mod control_notes_create_insert_race;
#[path = "control/control_notes_move_clip.rs"]
mod control_notes_move_clip;
#[path = "control/control_notes_read_your_writes.rs"]
mod control_notes_read_your_writes;
#[path = "control/control_plugin_bypass.rs"]
mod control_plugin_bypass;
#[path = "control/control_plugin_param_bounds.rs"]
mod control_plugin_param_bounds;
#[path = "control/control_plugin_param_meta.rs"]
mod control_plugin_param_meta;
#[path = "control/control_plugin_presets.rs"]
mod control_plugin_presets;
#[path = "control/control_plugin_params_persist.rs"]
mod control_plugin_params_persist;
#[path = "control/control_plugins_catalog.rs"]
mod control_plugins_catalog;
#[path = "control/control_plugins_rescan.rs"]
mod control_plugins_rescan;
#[path = "control/control_position_beats.rs"]
mod control_position_beats;
#[path = "control/control_project.rs"]
mod control_project;
#[path = "control/control_render.rs"]
mod control_render;
#[path = "control/control_render_wav_geometry.rs"]
mod control_render_wav_geometry;
#[path = "control/control_reply_contract.rs"]
mod control_reply_contract;
#[path = "control/control_section_harmony.rs"]
mod control_section_harmony;
#[path = "control/control_sends.rs"]
mod control_sends;
#[path = "control/control_sidechain.rs"]
mod control_sidechain;
#[path = "control/control_socket_roundtrip.rs"]
mod control_socket_roundtrip;
#[path = "control/control_song_routing.rs"]
mod control_song_routing;
#[path = "control/control_song_summary_tempo_map.rs"]
mod control_song_summary_tempo_map;
#[path = "control/control_song_views.rs"]
mod control_song_views;
#[path = "control/control_track_add_plugin_result.rs"]
mod control_track_add_plugin_result;
#[path = "control/control_replace_effect.rs"]
mod control_replace_effect;
#[path = "control/control_track_dispatch.rs"]
mod control_track_dispatch;
#[path = "control/control_track_frozen.rs"]
mod control_track_frozen;
#[path = "control/control_track_mixer.rs"]
mod control_track_mixer;
#[path = "control/control_track_move_effect.rs"]
mod control_track_move_effect;
#[path = "control/control_track_presets.rs"]
mod control_track_presets;
#[path = "control/control_track_remove_effect.rs"]
mod control_track_remove_effect;
#[path = "control/control_transport.rs"]
mod control_transport;
#[path = "control/control_undo_contract.rs"]
mod control_undo_contract;
#[path = "control/pool_usage_staleness.rs"]
mod pool_usage_staleness;
#[path = "control/audition_scrub_fingerprint.rs"]
mod audition_scrub_fingerprint;
#[path = "control/control_view_model_shared.rs"]
mod control_view_model_shared;
#[path = "control/control_vocal.rs"]
mod control_vocal;
#[path = "control/control_vocal_generate.rs"]
mod control_vocal_generate;
#[path = "control/control_vocal_lanes.rs"]
mod control_vocal_lanes;
#[path = "control/control_vocal_render_all_lanes.rs"]
mod control_vocal_render_all_lanes;
