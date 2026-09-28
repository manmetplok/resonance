//! Palette for the Color editor: the shared lavender palette, plus the
//! colours for the even and odd harmonic bars.

use plugin_gui_core::egui;

pub use plugin_gui_core::theme::lavender::*;

/// Even harmonics — the warm token, since even-dominant is what §2.1
/// calls warm.
pub const EVEN: egui::Color32 = WARM;
/// Odd harmonics — the accent.
pub const ODD: egui::Color32 = ACCENT;
/// The fundamental's bar.
pub const FUNDAMENTAL: egui::Color32 = TEXT_2;
