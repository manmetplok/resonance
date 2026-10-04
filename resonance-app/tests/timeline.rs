//! `timeline` test group.
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

#[path = "timeline/arrange_cull.rs"]
mod arrange_cull;
#[path = "timeline/arrange_layout.rs"]
mod arrange_layout;
#[path = "timeline/selection_commands.rs"]
mod selection_commands;
#[path = "timeline/transport_control.rs"]
mod transport_control;
#[path = "timeline/arrangement_marker_reducers.rs"]
mod arrangement_marker_reducers;
#[path = "timeline/automation_device_params.rs"]
mod automation_device_params;
#[path = "timeline/automation_edits.rs"]
mod automation_edits;
#[path = "timeline/automation_live_tint.rs"]
mod automation_live_tint;
#[path = "timeline/automation_mirror.rs"]
mod automation_mirror;
#[path = "timeline/automation_persistence.rs"]
mod automation_persistence;
#[path = "timeline/bpm_input_validation.rs"]
mod bpm_input_validation;
#[path = "timeline/clip_delete_echo_owed.rs"]
mod clip_delete_echo_owed;
#[path = "timeline/clip_fade_gain_draw.rs"]
mod clip_fade_gain_draw;
#[path = "timeline/clip_fade_gain_handlers.rs"]
mod clip_fade_gain_handlers;
#[path = "timeline/clip_fade_gain_hit_test.rs"]
mod clip_fade_gain_hit_test;
#[path = "timeline/clip_fade_gain_mirror.rs"]
mod clip_fade_gain_mirror;
#[path = "timeline/clip_fade_gain_persistence.rs"]
mod clip_fade_gain_persistence;
#[path = "timeline/clip_fade_gain_snapshot.rs"]
mod clip_fade_gain_snapshot;
#[path = "timeline/clip_inspector_flyout.rs"]
mod clip_inspector_flyout;
#[path = "timeline/clip_warp_snapshot.rs"]
mod clip_warp_snapshot;
#[path = "timeline/clip_warp_ui.rs"]
mod clip_warp_ui;
#[path = "timeline/drag_after_auto_follow.rs"]
mod drag_after_auto_follow;
#[path = "timeline/drag_placement_handlers.rs"]
mod drag_placement_handlers;
#[path = "timeline/drag_placement_visuals.rs"]
mod drag_placement_visuals;
#[path = "timeline/marker_hit_test.rs"]
mod marker_hit_test;
#[path = "timeline/layout_memo.rs"]
mod layout_memo;
#[path = "timeline/marker_ui_reducers.rs"]
mod marker_ui_reducers;
#[path = "timeline/markers_overview_snapshot.rs"]
mod markers_overview_snapshot;
#[path = "timeline/markers_overview_ui.rs"]
mod markers_overview_ui;
#[path = "timeline/playhead_follow.rs"]
mod playhead_follow;
#[path = "timeline/quantize_persistence.rs"]
mod quantize_persistence;
#[path = "timeline/engine_clip_names.rs"]
mod engine_clip_names;
#[path = "timeline/recording_undo.rs"]
mod recording_undo;
#[path = "timeline/track_delete_cleanup.rs"]
mod track_delete_cleanup;
#[path = "timeline/undo_noop_gesture.rs"]
mod undo_noop_gesture;
#[path = "timeline/stale_clip_import.rs"]
mod stale_clip_import;
#[path = "timeline/undo_before_echo.rs"]
mod undo_before_echo;
#[path = "timeline/undo_clip_audio_persist.rs"]
mod undo_clip_audio_persist;
#[path = "timeline/undo_view_state_keeps_redo.rs"]
mod undo_view_state_keeps_redo;
#[path = "timeline/undo_transient_dialog_state.rs"]
mod undo_transient_dialog_state;
#[path = "timeline/render_cache.rs"]
mod render_cache;
#[path = "timeline/selection_bar.rs"]
mod selection_bar;
#[path = "timeline/snap_signature_map.rs"]
mod snap_signature_map;
#[path = "timeline/take_comp_edits.rs"]
mod take_comp_edits;
#[path = "timeline/take_group_mirror.rs"]
mod take_group_mirror;
#[path = "timeline/take_lane_input.rs"]
mod take_lane_input;
#[path = "timeline/take_lane_render.rs"]
mod take_lane_render;
#[path = "timeline/take_removal_audible.rs"]
mod take_removal_audible;
#[path = "timeline/test_arrangement_markers.rs"]
mod test_arrangement_markers;
#[path = "timeline/timeline_automation_device_lanes.rs"]
mod timeline_automation_device_lanes;
#[path = "timeline/timeline_automation_group.rs"]
mod timeline_automation_group;
#[path = "timeline/timeline_automation_input.rs"]
mod timeline_automation_input;
#[path = "timeline/timeline_automation_lane_rows.rs"]
mod timeline_automation_lane_rows;
#[path = "timeline/timeline_automation_lane_selector.rs"]
mod timeline_automation_lane_selector;
#[path = "timeline/timeline_automation_render.rs"]
mod timeline_automation_render;
#[path = "timeline/timeline_group_hit_test.rs"]
mod timeline_group_hit_test;
#[path = "timeline/timeline_group_lane.rs"]
mod timeline_group_lane;
#[path = "timeline/timeline_markers_snapshot.rs"]
mod timeline_markers_snapshot;
#[path = "timeline/track_group_cycles.rs"]
mod track_group_cycles;
#[path = "timeline/track_header_alignment.rs"]
mod track_header_alignment;
#[path = "timeline/track_header_automation_lane_rows.rs"]
mod track_header_automation_lane_rows;
#[path = "timeline/track_header_freeze_button.rs"]
mod track_header_freeze_button;
#[path = "timeline/tempo_drag_selection.rs"]
mod tempo_drag_selection;
#[path = "timeline/undo_coalesce.rs"]
mod undo_coalesce;
#[path = "timeline/undo_history.rs"]
mod undo_history;
#[path = "timeline/editor_key_focus.rs"]
mod editor_key_focus;
#[path = "timeline/load_scroll_reset.rs"]
mod load_scroll_reset;
#[path = "timeline/timeline_key_grant.rs"]
mod timeline_key_grant;
#[path = "timeline/status_area_keeps_widget_state.rs"]
mod status_area_keeps_widget_state;
#[path = "timeline/vertical_scrollbar_visible.rs"]
mod vertical_scrollbar_visible;
#[path = "timeline/vertical_scroll_clamp.rs"]
mod vertical_scroll_clamp;
#[path = "timeline/clip_drag_tempo_snap.rs"]
mod clip_drag_tempo_snap;
