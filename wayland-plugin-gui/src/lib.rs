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
//! The platform-neutral pieces of the editor contract — [`EditorApp`],
//! [`EditorOptions`], [`EditorError`], the [`theme`] and [`widgets`]
//! modules, and the [`egui`] re-export — live in `plugin-gui-core` and are
//! re-exported here, so this crate's API is unchanged by the extraction
//! (macos-editor-plan.md item 3a).
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
//!
//! On non-Linux targets the windowing body of this crate — [`Editor`]
//! included — is compiled out (the Wayland stack does not exist there);
//! only the `plugin-gui-core` re-exports below remain, so the many plugin
//! files that import `wayland_plugin_gui::{egui, theme, widgets, ...}`
//! compile everywhere while the runtime itself is selected per platform
//! in `resonance_plugin::editor_host` (macos-editor-plan.md item 3c).

#[cfg(target_os = "linux")]
mod editor;
#[cfg(target_os = "linux")]
mod egl_context;
// Pure std, no Wayland dependency — compiled everywhere (only the Linux
// windowing body uses it, but keeping it unconditional lets the headless
// watchdog test in `tests/` run on any platform).
mod join;
#[cfg(target_os = "linux")]
mod input;
#[cfg(target_os = "linux")]
mod window_thread;

// Keep `crate::app` / `crate::error` / `crate::size` resolvable for the
// windowing modules above without touching their imports: the modules
// moved to `plugin-gui-core`, these root aliases put them back on the
// old paths crate-internally.
#[cfg(target_os = "linux")]
use plugin_gui_core::{app, error, size};

pub use plugin_gui_core::{theme, widgets, EditorApp, EditorError, EditorOptions};

#[cfg(target_os = "linux")]
pub use editor::Editor;

// Re-export egui so consumers don't need to pin a matching version themselves.
pub use plugin_gui_core::egui;

/// CSD fallback-frame geometry, exposed for integration tests only.
///
/// Not part of the supported public API (the layout constants and rects are an
/// implementation detail of the client-side decoration fallback). Re-exported
/// here so the `tests/` close-button hit-test can drive the same pure geometry
/// the live paint path uses, without an in-crate `#[cfg(test)]` module.
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub use window_thread::decorations as csd_geometry;

/// The compositor-driven size feedback cell, exposed for integration
/// tests only (same rationale as [`csd_geometry`]): plugins read the
/// live size through [`Editor::get_size`], never through this type.
#[doc(hidden)]
pub use plugin_gui_core::SharedSize;

/// The bounded-join teardown watchdog, exposed for integration tests
/// only (same rationale as [`csd_geometry`]): the runtime calls it from
/// `Editor::destroy`/`Drop` so a plugin `ui()` blocked in a modal loop
/// cannot wedge the host thread; the `tests/` coverage drives it with a
/// deliberately-stuck plain thread, which needs no Wayland session.
#[doc(hidden)]
pub use join::join_with_timeout;

/// The bounded startup handshake `Editor::new` waits on (PLG-10), exposed
/// for the same headless tests as [`join_with_timeout`].
#[doc(hidden)]
pub use join::{await_startup, Startup};

/// The SCTK → egui input translator, exposed for headless tests of its
/// button bookkeeping (a button held across a hide, FU-M1c).
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub use input::InputState;

