//! `midi` test group.
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

#[path = "midi/midi_bulk_edit_mirror.rs"]
mod midi_bulk_edit_mirror;
#[path = "midi/midi_clip_trim_tempo.rs"]
mod midi_clip_trim_tempo;
#[path = "midi/midi_editor_selection.rs"]
mod midi_editor_selection;
#[path = "midi/midi_groove_ui.rs"]
mod midi_groove_ui;
#[path = "midi/midi_learn.rs"]
mod midi_learn;
#[path = "midi/midi_map_mirror.rs"]
mod midi_map_mirror;
#[path = "midi/midi_marquee.rs"]
mod midi_marquee;
#[path = "midi/midi_quantize_handlers.rs"]
mod midi_quantize_handlers;
#[path = "midi/midi_quantize_overlay.rs"]
mod midi_quantize_overlay;
#[path = "midi/midi_quantize_panel.rs"]
mod midi_quantize_panel;
#[path = "midi/note_drag_reorder.rs"]
mod note_drag_reorder;
