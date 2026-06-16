//! Wayland-native plugin GUI runtime.
//!
//! Hosts an [`egui`] UI inside a floating top-level Wayland window, running on its
//! own thread so it can be spawned from inside a CLAP/VST plugin dylib without
//! fighting the host application's event loop.
//!
//! # Model
//!
//! - The runtime owns a single thread which opens a `wl_display` connection, creates
//!   an xdg_toplevel window, establishes an EGL context on the `wl_surface`, and runs
//!   an SCTK event loop driving `egui_glow` each frame.
//! - The caller interacts with the runtime through the [`Editor`] handle, which sends
//!   commands (show/hide/resize/destroy) to the editor thread over a channel.
//! - The caller supplies an [`EditorApp`] whose `ui()` method is invoked on the editor
//!   thread every frame.
//!
//! # Scope
//!
//! Wayland only, floating-only (no `set_parent`). Decorations are negotiated:
//! the toplevel requests server-side decorations (`WindowDecorations::RequestServer`),
//! and when the compositor instead forces client-side mode (or offers no
//! `zxdg_decoration_manager_v1` at all) the runtime draws its own minimal CSD
//! frame — border, titlebar, and a working close button — so the window is
//! usable on every compositor. Clipboard, DnD, and IME are not implemented in
//! the initial version.

mod app;
mod editor;
mod egl_context;
mod error;
mod input;
pub mod theme;
pub mod widgets;
mod window_thread;

pub use app::EditorApp;
pub use editor::{Editor, EditorOptions};
pub use error::EditorError;

// Re-export egui so consumers don't need to pin a matching version themselves.
pub use egui;

/// CSD fallback-frame geometry, exposed for integration tests only.
///
/// Not part of the supported public API (the layout constants and rects are an
/// implementation detail of the client-side decoration fallback). Re-exported
/// here so the `tests/` close-button hit-test can drive the same pure geometry
/// the live paint path uses, without an in-crate `#[cfg(test)]` module.
#[doc(hidden)]
pub use window_thread::decorations as csd_geometry;
