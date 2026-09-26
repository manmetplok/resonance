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

pub use layout::{
    bar_to_section_tick, bars_span, sample_to_section_tick, section_bars_in_range,
    section_bars_in_range_every, section_total_beats, section_total_ticks, tempo_map_hash,
    visible_x_window, workspace_width, BarUnit, SectionBar, WORKSPACE_PAD_X,
};
