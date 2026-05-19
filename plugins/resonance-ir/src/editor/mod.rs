//! IR plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Mirrors the amp / reverb editors: a Factory that produces a
//! `RuntimeEditorHandle`, which wraps the `wayland_plugin_gui::Editor`
//! and drives an `EditorApp` implementation on the editor thread.
//!
//! Layout (top → bottom):
//!
//! - Top strip: header with title, "Load IR…" button, Prev/Next and
//!   the current filename + position counter.
//! - Centre: waveform view (left) + frequency-response view (right)
//!   drawn from the `IrSnapshot` published by the loader thread, plus
//!   a stereo IN/OUT meter strip along the bottom.
//! - Bottom: the dry/wet and output-gain control strip.

mod app;
mod controls;
mod factory;
mod header;
mod meters;
mod response_view;
mod theme;
mod waveform_view;

pub use factory::IrEditorFactory;

// Re-exported so the per-section modules (e.g. `header.rs`) can keep
// their existing `super::IrEditorApp` import path.
pub(crate) use app::IrEditorApp;
