//! Drums plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout: a top chrome bar (traffic-light dots + Resonance / Drums brand)
//! sits above a tab bar with the module nav (Pads · Mics · Articulations ·
//! Mod · FX), the KIT preset pill, and the PADS badge. The central body
//! dispatches per tab — the Pads tab renders the two-column pad list +
//! pad detail surface with the KIT and GLOBAL cards on a bottom row. A
//! status bar (sample rate, buffer size, OUT meter) sits along the
//! bottom edge.
//!
//! Every control comes from `wayland_plugin_gui::widgets` — the knobs
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

// Re-exported so the per-section modules (`pad_inspector`, `kit_browser`)
// can keep their existing `super::reload_kit` import path. The helper
// itself lives at `crate::reload` because the articulation watcher needs
// it in headless builds too.
pub(crate) use crate::reload::reload_kit;
