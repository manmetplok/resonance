//! Editor palette — the canonical lavender palette from
//! `wayland_plugin_gui::theme` (ba todo #1338), plus the per-channel echo
//! colours.
//!
//! The two channels were a blue/orange pair picked around the old blue
//! accent. They are now the canonical accent/warm pair — the same two
//! tokens the shared bipolar knob and slider use to tell one side from
//! the other — so the echo view reads as this product rather than as its
//! own. The soft accent, not the accent, because these are 3 px traces
//! and a 10 pt label.

use plugin_gui_core::egui;

pub use plugin_gui_core::theme::lavender::*;

pub const ECHO_L: egui::Color32 = ACCENT_SOFT;
pub const ECHO_R: egui::Color32 = WARM;
