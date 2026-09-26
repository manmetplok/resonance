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
//!   Its methods dispatch onto the main queue (`show`/`hide`/`set_size`/
//!   `destroy` asynchronously; only `new` synchronously, mirroring the
//!   Wayland handle's ready-handshake). A caller already on the main thread
//!   runs the work inline instead — dispatching synchronously to yourself is
//!   a deadlock.
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
//! **Deadlock rule** (plan §1): the main thread must never block on a
//! thread while that thread is inside [`Editor::new`] — the one remaining
//! `exec_sync`. [`Editor::destroy`] no longer waits for the main thread
//! when called from another one (PLG-05): Resonance's quit path breaks the
//! rule for teardown by design — the main thread waits in
//! `AudioEngine::shutdown` for the engine thread, which drops every plugin
//! and so destroys every open editor — so the teardown is queued and runs
//! whenever the main thread next services its queue (or never, if the
//! process exits first, which is harmless: nothing is left to release).
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
//! On non-macOS targets the windowing body of this crate — [`Editor`]
//! included — is compiled out; only the pure [`input`] module and the
//! `plugin-gui-core` re-exports remain, mirroring how `wayland-plugin-gui`
//! compiles itself out off Linux. Both crates stay workspace members and
//! `cargo check --workspace` passes on both platforms; the runtime is
//! selected per platform in `resonance_plugin::editor_host`
//! (macos-editor-plan.md item 3c).

#[cfg(target_os = "macos")]
mod editor;
#[cfg(target_os = "macos")]
mod gl_context;
#[cfg(target_os = "macos")]
mod window_main_thread;

// NSApplication pump for the live `harness = false` tests (this crate's
// `tests/editor_size.rs` and resonance-gate's `tests/editor_open_cocoa.rs`).
// Not part of the editor API.
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub mod test_support;

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

