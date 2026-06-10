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
//! Wayland only, floating-only (no `set_parent`), server-side decorations preferred
//! with a minimal client-side fallback. Clipboard, DnD, and IME are not implemented
//! in the initial version.

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
