//! Drums plugin editor: an egui UI hosted by the platform GUI runtime.
//!
//! Layout: a chrome bar (Resonance / Drums brand, the preset bar, and the
//! "Open kit file…" / "Download kits…" buttons) sits above a second bar
//! carrying the PADS badge and the KIT preset pill (◀ name ▶). Pads is
//! the editor's only view, so there is no tab strip to switch it with.
//! The central body is a fixed-height KIT + GLOBAL card row at the
//! bottom and, above it, the two-column pad list + pad detail, each
//! scrolling independently so the window can be resized down to its
//! declared minimum without losing either (`factory.rs`,
//! drums-plugin-rework.md §6.1). A status bar (sample rate, buffer size,
//! OUT meter) sits along the bottom edge, and the Download Kits overlay
//! (`download_panel.rs`) draws over everything else when open.
//!
//! Every control comes from `plugin_gui_core::widgets` — the knobs
//! always did, and ba todo #1335 retired the local `editor/widgets/`
//! copies of the chip, the segmented control and the slider. That
//! module's own comment admitted they were "duplicated from
//! resonance-wavetable so the two editors can evolve independently";
//! what they actually did was drift (ba doc #275).

mod app;
mod chrome;
mod download_panel;
mod factory;
mod kit_browser;
mod pad_grid;
mod pad_inspector;
mod theme;

pub use factory::DrumsEditorFactory;

use plugin_gui_core::egui;

// Re-exported so the per-section modules (`pad_inspector`, `kit_browser`)
// can keep their existing `super::reload_kit` import path. The helper
// itself lives at `crate::reload` because the articulation watcher needs
// it in headless builds too.
pub(crate) use crate::reload::reload_kit;

/// The width a card's body should claim inside `ui`, once `inset` — the
/// frame's own horizontal margin — is taken off.
///
/// Exists to floor the subtraction. Every card in this editor sizes
/// itself as "whatever is available, less my margins", and a window
/// narrower than those margins makes that negative. `Ui::set_min_width`
/// carries a `debug_assert!(0.0 <= width)`, so in a debug build that is a
/// panic on the editor thread; in release the assert is compiled out and
/// egui's placer discards any non-positive width anyway. A compositor
/// that tiles plugin windows can produce the narrow case without the user
/// doing anything unusual (ba todo #1377).
///
/// Zero is the honest floor rather than a minimum like 40px: this is an
/// *expansion* hint — it asks the card to fill its row, and nothing about
/// the content depends on it. Asking for nothing lets egui lay the
/// content out and clip it, which degrades far better than forcing a
/// width the window does not have and pushing the card off its own edge.
pub(crate) fn body_width(ui: &egui::Ui, inset: f32) -> f32 {
    (ui.available_width() - inset).max(0.0)
}

/// Test-only: draw the pad inspector into `ui`, so a test can hand it a
/// width and see what it does with it.
///
/// The editor module is private and the inspector needs a kit bridge and
/// a mic catalogue, which is why this hook exists rather than the test
/// calling `pad_inspector::draw` itself.
#[doc(hidden)]
pub fn test_draw_pad_inspector(ui: &mut egui::Ui, bridge: &crate::KitBridge, selected_pad: usize) {
    pad_inspector::draw(
        ui,
        &bridge.params,
        bridge,
        &crate::mic_catalog::ManifestMicCatalog::default(),
        selected_pad,
    );
}
