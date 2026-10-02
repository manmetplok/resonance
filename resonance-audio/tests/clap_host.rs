//! `clap_host` test group: the CLAP host (`clap_host/`) and plugin lifecycle: load, rescan, bypass, params, latency.
//!
//! One binary per group instead of one per file (code review ARCH-03).
//! Each of these files used to be its own integration-test target; they
//! are still ordinary test files, only the target boundary moved. Add a
//! new test as a module here (or in another group), never as a new
//! top-level `tests/*.rs` file — `tools/arch-invariants` enforces that.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! the `tests/clap_host/` subdirectory.

#[path = "clap_host/clap_all_notes_off.rs"]
mod clap_all_notes_off;
#[path = "clap_host/clap_bundle_path.rs"]
mod clap_bundle_path;
#[path = "clap_host/plugin_preset_state.rs"]
mod plugin_preset_state;
#[path = "clap_host/preset_discovery.rs"]
mod preset_discovery;
#[path = "clap_host/clap_factory_presets.rs"]
mod clap_factory_presets;
#[path = "clap_host/clap_ffi_hardening.rs"]
mod clap_ffi_hardening;
#[path = "clap_host/clap_latency_tracking.rs"]
mod clap_latency_tracking;
#[path = "clap_host/clap_note_event_order.rs"]
mod clap_note_event_order;
#[path = "clap_host/color_plugin_loads.rs"]
mod color_plugin_loads;
#[path = "clap_host/clap_thread_roles.rs"]
mod clap_thread_roles;
#[path = "clap_host/clap_param_flush.rs"]
mod clap_param_flush;
#[path = "clap_host/clap_param_flags.rs"]
mod clap_param_flags;
#[path = "clap_host/clap_param_meta.rs"]
mod clap_param_meta;
#[path = "clap_host/param_from_text.rs"]
mod param_from_text;
#[path = "clap_host/clap_plugin_drop_order.rs"]
mod clap_plugin_drop_order;
#[path = "clap_host/plugin_binaries.rs"]
mod plugin_binaries;
#[path = "clap_host/plugin_bypass.rs"]
mod plugin_bypass;
#[path = "clap_host/plugin_editor_state.rs"]
mod plugin_editor_state;
#[path = "clap_host/plugin_id_duplicate_rejected.rs"]
mod plugin_id_duplicate_rejected;
#[path = "clap_host/plugin_live_values.rs"]
mod plugin_live_values;
#[path = "clap_host/plugin_load_failure.rs"]
mod plugin_load_failure;
#[path = "clap_host/plugin_output_scrub.rs"]
mod plugin_output_scrub;
#[path = "clap_host/plugin_rescan.rs"]
mod plugin_rescan;
#[path = "clap_host/probe_chain.rs"]
mod probe_chain;
#[path = "clap_host/depth_reverb.rs"]
mod depth_reverb;
#[path = "clap_host/drums_kit_info.rs"]
mod drums_kit_info;
#[path = "clap_host/sub_track_plugin_removal.rs"]
mod sub_track_plugin_removal;
