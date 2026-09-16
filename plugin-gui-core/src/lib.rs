//! Platform-neutral core of the plugin editor GUI.
//!
//! Everything here is pure egui + std — no windowing, no platform APIs —
//! so it builds everywhere. The per-platform runtimes (`wayland-plugin-gui`
//! today, `cocoa-plugin-gui` per `macos-editor-plan.md`) depend on this
//! crate for the shared editor contract and re-export it, so plugin code
//! keeps a single import surface:
//!
//! - [`EditorApp`] — the two-method trait a hosted editor implements.
//! - [`EditorOptions`] / [`EditorError`] — the `Editor::new` contract.
//! - [`SharedSize`] — the applied-size feedback cell a runtime publishes into.
//! - [`theme`] / [`widgets`] — the fleet palette and the pure widget set.
//! - [`repaint`] — the shared egui repaint-request scheduling decision.

// The module split mirrors the pre-extraction layout in wayland-plugin-gui
// so the runtimes can keep referring to `app` / `error` / `size` as
// modules; the types below are the supported way in for everyone else.
#[doc(hidden)]
pub mod app;
#[doc(hidden)]
pub mod error;
mod options;
pub mod repaint;
#[doc(hidden)]
pub mod size;
pub mod theme;
pub mod widgets;

pub use app::EditorApp;
pub use error::EditorError;
pub use options::EditorOptions;

// Re-export egui so consumers don't need to pin a matching version themselves.
pub use egui;

/// The runtime-driven size feedback cell, exposed for the runtimes and
/// their integration tests only: plugins read the live size through the
/// runtime's `Editor::get_size`, never through this type.
#[doc(hidden)]
pub use size::SharedSize;
