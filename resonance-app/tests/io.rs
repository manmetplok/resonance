//! `io` test group.
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

#[path = "io/audio_import_entry_points.rs"]
mod audio_import_entry_points;
#[path = "io/autosave_settings.rs"]
mod autosave_settings;
#[path = "io/autosave_write.rs"]
mod autosave_write;
#[path = "io/browser_handlers.rs"]
mod browser_handlers;
#[path = "io/chord_sheet_header.rs"]
mod chord_sheet_header;
#[path = "io/engine_events_plugin_move_mirror.rs"]
mod engine_events_plugin_move_mirror;
#[path = "io/engine_events_pool_mirror.rs"]
mod engine_events_pool_mirror;
#[path = "io/export_dialog_shell.rs"]
mod export_dialog_shell;
#[path = "io/files_tab_rendering.rs"]
mod files_tab_rendering;
#[path = "io/import_dialog.rs"]
mod import_dialog;
#[path = "io/import_dialog_review.rs"]
mod import_dialog_review;
#[path = "io/import_entry_points.rs"]
mod import_entry_points;
#[path = "io/import_placement.rs"]
mod import_placement;
#[path = "io/import_placement_stale.rs"]
mod import_placement_stale;
#[path = "io/import_progress_dialog.rs"]
mod import_progress_dialog;
#[path = "io/media_browser_scaffold.rs"]
mod media_browser_scaffold;
#[path = "io/midi_clip_lossless_roundtrip.rs"]
mod midi_clip_lossless_roundtrip;
#[path = "io/offline_render_gate.rs"]
mod offline_render_gate;
#[path = "io/open_failure_keeps_path.rs"]
mod open_failure_keeps_path;
#[path = "io/pool_persistence.rs"]
mod pool_persistence;
#[path = "io/pool_tab_rendering.rs"]
mod pool_tab_rendering;
#[path = "io/project_atomic_write.rs"]
mod project_atomic_write;
#[path = "io/project_backups.rs"]
mod project_backups;
#[path = "io/project_track_groups_persist.rs"]
mod project_track_groups_persist;
#[path = "io/relink.rs"]
mod relink;
#[path = "io/relink_modal.rs"]
mod relink_modal;
#[path = "io/replay.rs"]
mod replay;
#[path = "io/replay_diff.rs"]
mod replay_diff;
#[path = "io/save_keeps_dirty_for_late_edit.rs"]
mod save_keeps_dirty_for_late_edit;
#[path = "io/take_lanes_persistence.rs"]
mod take_lanes_persistence;
#[path = "io/user_definitions_rescan.rs"]
mod user_definitions_rescan;
#[path = "io/files_listing_fingerprint.rs"]
mod files_listing_fingerprint;
