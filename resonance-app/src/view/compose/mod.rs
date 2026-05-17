//! Compose tab view tree. The top-level `view_compose` body lives in
//! [`page`]; this file keeps module declarations and shared re-exports
//! (workspace geometry primitives, group-header styles) so the call
//! sites continue to address them via `super::*`.

pub mod chord_lane;
pub mod drum_groups_manager;
pub mod drumroll;
pub mod expanded_editor;
pub mod global_tracks;
pub mod group_header;
pub mod lane_inspector;
pub mod lane_side;
mod layout;
pub mod manual_motif_canvas;
mod page;
pub mod popover;
pub mod scale_stripe;
pub mod strip;
pub mod tracks;
pub mod vocal_lane;
pub mod vocal_roll;

// Re-export the shared layout primitives so the existing call sites keep
// working (`super::workspace_width`, `super::section_total_beats`, etc.).
#[allow(unused_imports)]
pub use layout::{section_total_beats, workspace_width, BEAT_PX_COMPOSE};
