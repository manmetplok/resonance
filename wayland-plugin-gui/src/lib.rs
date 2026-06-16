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
//! Wayland only, floating-only (no `set_parent`). By default the runtime draws
//! its own client-side decoration frame — border, titlebar, and a working close
//! button — on every compositor (`WindowDecorations::RequestClient`). This is
//! deliberate: "server-side decorations" does not imply a close button, and
//! wlroots compositors (Hyprland, Sway) honour an SSD request but render only a
//! thin border with no titlebar and no close affordance, leaving the window with
//! no way to be closed from itself. Drawing our own frame guarantees an
//! identical, always-usable close button (GNOME/Mutter, KDE/KWin, Hyprland,
//! Sway). The close button feeds the same `close_requested` path as a server-side
//! `xdg_toplevel.close`, so `EditorApp::on_close` is invoked exactly once either
//! way. Setting `WPG_FORCE_SSD` opts back into requesting server-side
//! decorations and only falling back to the client frame when the compositor
//! forces client-side mode (useful where a real native titlebar exists, e.g.
//! KWin). Clipboard, DnD, and IME are not implemented in the initial version.

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
