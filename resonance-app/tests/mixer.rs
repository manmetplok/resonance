//! `mixer` test group.
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

#[path = "mixer/aux_send_handlers.rs"]
mod aux_send_handlers;
#[path = "mixer/aux_send_mirror.rs"]
mod aux_send_mirror;
#[path = "mixer/aux_send_persistence.rs"]
mod aux_send_persistence;
#[path = "mixer/frost_treatment_swatch.rs"]
mod frost_treatment_swatch;
#[path = "mixer/group_creation_from_selection.rs"]
mod group_creation_from_selection;
#[path = "mixer/group_id_after_load.rs"]
mod group_id_after_load;
#[path = "mixer/group_header.rs"]
mod group_header;
#[path = "mixer/group_identity_rail.rs"]
mod group_identity_rail;
#[path = "mixer/group_macro_level.rs"]
mod group_macro_level;
#[path = "mixer/group_macro_mute.rs"]
mod group_macro_mute;
#[path = "mixer/group_macro_solo.rs"]
mod group_macro_solo;
#[path = "mixer/group_member_track_header.rs"]
mod group_member_track_header;
#[path = "mixer/group_membership_drag.rs"]
mod group_membership_drag;
#[path = "mixer/mixer_automation_controls.rs"]
mod mixer_automation_controls;
#[path = "mixer/mixer_chain_reorder.rs"]
mod mixer_chain_reorder;
#[path = "mixer/mixer_generic_param_panel.rs"]
mod mixer_generic_param_panel;
#[path = "mixer/generic_panel_display.rs"]
mod generic_panel_display;
#[path = "mixer/mixer_group_clustering.rs"]
mod mixer_group_clustering;
#[path = "mixer/inspector_lazy_fingerprint.rs"]
mod inspector_lazy_fingerprint;
#[path = "mixer/mixer_inspector_bus.rs"]
mod mixer_inspector_bus;
#[path = "mixer/mixer_inspector_bus_snapshot.rs"]
mod mixer_inspector_bus_snapshot;
#[path = "mixer/mixer_inspector_collapse.rs"]
mod mixer_inspector_collapse;
#[path = "mixer/mixer_inspector_empty_project.rs"]
mod mixer_inspector_empty_project;
#[path = "mixer/mixer_inspector_external_instrument.rs"]
mod mixer_inspector_external_instrument;
#[path = "mixer/mixer_inspector_external_status.rs"]
mod mixer_inspector_external_status;
#[path = "mixer/mixer_inspector_external_toggle.rs"]
mod mixer_inspector_external_toggle;
#[path = "mixer/mixer_inspector_sends.rs"]
mod mixer_inspector_sends;
#[path = "mixer/mixer_inspector_sends_snapshot.rs"]
mod mixer_inspector_sends_snapshot;
#[path = "mixer/mixer_strip_external_instrument.rs"]
mod mixer_strip_external_instrument;
#[path = "mixer/mixer_sub_track_grouping.rs"]
mod mixer_sub_track_grouping;
#[path = "mixer/plugin_bypass_persistence.rs"]
mod plugin_bypass_persistence;
#[path = "mixer/recording_overflow_banner.rs"]
mod recording_overflow_banner;
#[path = "mixer/sidechain_persistence.rs"]
mod sidechain_persistence;
#[path = "mixer/delete_track_confirm_undo.rs"]
mod delete_track_confirm_undo;
#[path = "mixer/tick_gating.rs"]
mod tick_gating;
#[path = "mixer/track_group_registry.rs"]
mod track_group_registry;
#[path = "mixer/pan_knob_drag.rs"]
mod pan_knob_drag;
#[path = "mixer/plugin_panel_fingerprint.rs"]
mod plugin_panel_fingerprint;
