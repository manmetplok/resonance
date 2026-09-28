//! `bounce` test group: offline renderers (`engine/bounce`): bounce, export, freeze, stems, normalization.
//!
//! One binary per group instead of one per file (code review ARCH-03).
//! Each of these files used to be its own integration-test target; they
//! are still ordinary test files, only the target boundary moved. Add a
//! new test as a module here (or in another group), never as a new
//! top-level `tests/*.rs` file — `tools/arch-invariants` enforces that.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! the `tests/bounce/` subdirectory.

#[path = "bounce/automation_render.rs"]
mod automation_render;
#[path = "bounce/bounce_external_offsets.rs"]
mod bounce_external_offsets;
#[path = "bounce/bounce_midi_events.rs"]
mod bounce_midi_events;
#[path = "bounce/bounce_plugin_lock.rs"]
mod bounce_plugin_lock;
#[path = "bounce/bounce_render_range_tempo.rs"]
mod bounce_render_range_tempo;
#[path = "bounce/bounce_tail_and_master_latency.rs"]
mod bounce_tail_and_master_latency;
#[path = "bounce/bounce_transport_guard.rs"]
mod bounce_transport_guard;
#[path = "bounce/export_encoders.rs"]
mod export_encoders;
#[path = "bounce/export_normalize.rs"]
mod export_normalize;
#[path = "bounce/export_settings.rs"]
mod export_settings;
#[path = "bounce/freeze_cache_read.rs"]
mod freeze_cache_read;
#[path = "bounce/freeze_render_core.rs"]
mod freeze_render_core;
#[path = "bounce/midi_export_project.rs"]
mod midi_export_project;
#[path = "bounce/offline_render_fidelity.rs"]
mod offline_render_fidelity;
#[path = "bounce/reference_export_exclusion.rs"]
mod reference_export_exclusion;
#[path = "bounce/stem_export.rs"]
mod stem_export;
#[path = "bounce/vocal_tuning_bounce.rs"]
mod vocal_tuning_bounce;
