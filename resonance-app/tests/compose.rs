//! `compose` test group.
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

#[path = "compose/builtin_templates.rs"]
mod builtin_templates;
#[path = "compose/chord_box_layout.rs"]
mod chord_box_layout;
#[path = "compose/chord_generator_modes.rs"]
mod chord_generator_modes;
#[path = "compose/chord_schema_inspector.rs"]
mod chord_schema_inspector;
#[path = "compose/chord_track.rs"]
mod chord_track;
#[path = "compose/chord_track_messages.rs"]
mod chord_track_messages;
#[path = "compose/compose_arrangement_strip.rs"]
mod compose_arrangement_strip;
#[path = "compose/compose_arrangement_strip_labels.rs"]
mod compose_arrangement_strip_labels;
#[path = "compose/compose_bar_sample_tempo_map.rs"]
mod compose_bar_sample_tempo_map;
#[path = "compose/compose_derived_clip_ids.rs"]
mod compose_derived_clip_ids;
#[path = "compose/compose_drum_arrangement.rs"]
mod compose_drum_arrangement;
#[path = "compose/compose_drum_arrangement_edit.rs"]
mod compose_drum_arrangement_edit;
#[path = "compose/compose_drum_arrangement_inspector.rs"]
mod compose_drum_arrangement_inspector;
#[path = "compose/compose_drum_arrangement_persistence.rs"]
mod compose_drum_arrangement_persistence;
#[path = "compose/compose_drum_arrangement_ribbon.rs"]
mod compose_drum_arrangement_ribbon;
#[path = "compose/compose_drum_arrangement_ribbon_snapshot.rs"]
mod compose_drum_arrangement_ribbon_snapshot;
#[path = "compose/compose_drum_grid_bar_spans.rs"]
mod compose_drum_grid_bar_spans;
#[path = "compose/compose_drum_grid_chained_snapshot.rs"]
mod compose_drum_grid_chained_snapshot;
#[path = "compose/compose_drum_materialize_arrangement.rs"]
mod compose_drum_materialize_arrangement;
#[path = "compose/compose_pinned_chords.rs"]
mod compose_pinned_chords;
#[path = "compose/compose_rail_collapse.rs"]
mod compose_rail_collapse;
#[path = "compose/compose_track_count.rs"]
mod compose_track_count;
#[path = "compose/compose_vocal_placeholder.rs"]
mod compose_vocal_placeholder;
#[path = "compose/compose_workspace_collapse.rs"]
mod compose_workspace_collapse;
#[path = "compose/drum_kit_pads.rs"]
mod drum_kit_pads;
#[path = "compose/drum_pattern_library.rs"]
mod drum_pattern_library;
#[path = "compose/drum_section_coverage.rs"]
mod drum_section_coverage;
#[path = "compose/generator_section.rs"]
mod generator_section;
#[path = "compose/global_tracks_edit_cycle.rs"]
mod global_tracks_edit_cycle;
#[path = "compose/global_tracks_shelf.rs"]
mod global_tracks_shelf;
#[path = "compose/seed_markers_from_sections.rs"]
mod seed_markers_from_sections;
#[path = "compose/template_instantiate.rs"]
mod template_instantiate;
#[path = "compose/templates_save.rs"]
mod templates_save;
#[path = "compose/templates_scan.rs"]
mod templates_scan;
