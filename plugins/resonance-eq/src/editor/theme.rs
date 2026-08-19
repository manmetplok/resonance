//! Editor-local theme constants — re-exports the canonical lavender
//! palette from `wayland_plugin_gui::theme` so all plugins feel like they
//! belong to the same product (ba todo #1338).
//!
//! Nothing is added here any more: the EQ's one local colour was a green
//! `GOOD` of its own, which the shared palette already carries.

pub use wayland_plugin_gui::theme::lavender::*;
