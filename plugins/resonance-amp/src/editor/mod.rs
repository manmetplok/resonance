//! Amp plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout mirrors the reverb editor's three-zone structure:
//!
//! - Top strip: header with title, Load Model button, file browser,
//!   and current model name.
//! - Below the header: a dedicated tuner strip.
//! - Centre: the main visualisation area — live oscilloscope on the
//!   left, static transfer-curve plot on the right, with stereo peak
//!   meters along the bottom.
//! - Bottom: the gain control strip.

mod app;
mod controls;
mod curve_view;
mod factory;
mod header;
mod meters;
mod scope_view;
mod theme;
#[cfg(feature = "editor")]
mod tone3000_panel;
pub mod tuner_view;

pub use factory::AmpEditorFactory;

// Re-exported so the per-section modules can keep their existing
// `super::AmpEditorApp` import path.
pub(crate) use app::AmpEditorApp;
