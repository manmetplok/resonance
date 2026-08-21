//! Cocoa-native plugin GUI runtime.
//!
//! Hosts an [`egui`] UI inside a floating `NSWindow`, driving everything from
//! the AppKit **main thread** so it can be spawned from inside a CLAP/VST
//! plugin dylib without owning a run loop of its own.
//!
//! # Model
//!
//! The Wayland runtime's model — one dedicated thread per editor — is
//! impossible on macOS: AppKit requires NSWindow/NSView creation, event
//! handling, and teardown to happen on the process main thread. So this
//! runtime inverts it (macos-editor-plan.md §1):
//!
//! - The window, its GL view, and the [`EditorApp`] live in a **main-thread
//!   controller**, reachable only through a main-thread registry.
//! - The caller-facing [`Editor`] handle is `Send` and owns no Objective-C
//!   objects — only the editor's registry id and the [`SharedSize`] mirror.
//!   Its methods dispatch onto the main queue (`show`/`hide`/`set_size`
//!   asynchronously; `new` and `destroy` synchronously, mirroring the Wayland
//!   handle's ready-handshake and thread-join). A caller already on the main
//!   thread runs the work inline instead — dispatching synchronously to
//!   yourself is a deadlock.
//! - Repaint is paced by a 60 Hz `NSTimer` on the main run loop (common
//!   modes, so frames keep coming through live resizes and modal loops); a
//!   tick repaints only when egui or an input event asked for it, so an idle
//!   editor costs a few atomic reads per tick.
//! - The host's run loop pumps all events — inside a CLAP host we never own
//!   one. The standalone `hello` example has to pump `NSApplication` itself;
//!   see `examples/hello.rs`.
//!
//! The plugin's `ui()` therefore runs on the main thread, where AppKit modal
//! loops (`rfd` file dialogs, `NSOpenPanel`) are legal. A modal loop re-enters
//! the main run loop, which can deliver a fresh `drawRect:` for our view while
//! a frame is still being built inside `ui()` — a reentrancy guard skips those
//! nested paints (macos-editor-plan.md item 3h).
//!
//! **Deadlock rule** (plan §1): the main thread must never block on the
//! thread calling [`Editor::new`]/[`Editor::destroy`] while that call is in
//! flight. Our host calls the CLAP gui extension from the engine control
//! thread, which only ever sends non-blocking commands to the main thread —
//! the invariant this runtime's `exec_sync` calls rely on.
//!
//! # Scope
//!
//! macOS only, floating-only (no `set_parent` — embedded mode is
//! macos-editor-plan.md item 3g). The window uses real server-side
//! decorations — AppKit titlebars always have a working close button, so
//! nothing like the Wayland runtime's CSD fallback frame exists here. The
//! close button feeds `EditorApp::on_close` exactly once. Clipboard, DnD, and
//! IME are not implemented in the initial version, matching the Wayland
//! runtime's v1 scope.
//!
//! On non-macOS targets the windowing body of this crate is compiled out and
//! [`Editor`] is a stub whose `new` always errors, mirroring how
//! `wayland-plugin-gui` stubs itself off Linux — both crates stay workspace
//! members and `cargo check --workspace` passes on both platforms.

#[cfg(target_os = "macos")]
mod editor;
#[cfg(target_os = "macos")]
mod gl_context;
#[cfg(target_os = "macos")]
mod window_main_thread;

// NSEvent-value → egui translation. Pure (egui + std only) and compiled
// unconditionally so its unit tests run on every platform.
#[doc(hidden)]
pub mod input;

pub use plugin_gui_core::{theme, widgets, EditorApp, EditorError, EditorOptions};

#[cfg(target_os = "macos")]
pub use editor::Editor;

// Re-export egui so consumers don't need to pin a matching version themselves.
pub use plugin_gui_core::egui;

/// The runtime-driven size feedback cell, exposed for integration tests only
/// (plugins read the live size through [`Editor::get_size`]).
#[doc(hidden)]
pub use plugin_gui_core::SharedSize;

/// Non-macOS stub of the [`Editor`] handle: the same public surface, with
/// `new` always returning an error because there is no AppKit to talk to.
/// Exists so `resonance_plugin::editor_host` can name this crate on every
/// platform once the factories migrate (macos-editor-plan.md item 3c); none
/// of the other methods can ever run, since no instance can be constructed.
#[cfg(not(target_os = "macos"))]
mod editor_stub {
    use plugin_gui_core::{EditorApp, EditorError, EditorOptions};

    pub struct Editor {
        size: (u32, u32),
        resizable: bool,
    }

    impl Editor {
        /// Always fails: this runtime is Cocoa-only. The Linux runtime
        /// is `wayland-plugin-gui`.
        pub fn new<A: EditorApp>(_app: A, _options: EditorOptions) -> Result<Self, EditorError> {
            Err(EditorError::Cocoa(
                "no Cocoa on this platform (the Linux runtime is wayland-plugin-gui)".to_string(),
            ))
        }

        pub fn show(&self) {}

        pub fn hide(&self) {}

        pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), EditorError> {
            self.size = (width, height);
            Ok(())
        }

        pub fn get_size(&self) -> (u32, u32) {
            self.size
        }

        pub fn is_resizable(&self) -> bool {
            self.resizable
        }

        pub fn destroy(self) {}
    }
}

#[cfg(not(target_os = "macos"))]
pub use editor_stub::Editor;
