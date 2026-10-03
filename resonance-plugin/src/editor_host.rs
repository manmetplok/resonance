//! The bridge from a live GUI-runtime window to the host.
//!
//! [`crate::gui::PluginEditor`] is what the CLAP bridge calls when a host
//! opens, moves, resizes or closes an editor; [`RuntimeEditor`] — the
//! platform GUI runtime's `Editor` — is the window that actually exists.
//! The two are deliberately unrelated — `gui.rs` names no GUI runtime so a
//! plugin can be built on anything — so *something* has to sit between
//! them, and that something is [`RuntimeEditorHandle`].
//!
//! It lives here, feature-gated behind `editor-widgets` alongside the other
//! editor glue, because it is the same code for every plugin: the adapter
//! is a property of the two traits, not of any plugin's UI. All 11
//! first-party plugins carried a verbatim copy of it until ba todo #1336.
//!
//! This module is also where the platform runtime is *selected*: the
//! [`RuntimeEditor`] alias and [`native_api`] are the only places in the
//! plugin stack that name a platform, so factories import everything from
//! here and a future win32 runtime is a one-line cfg (macos-editor-plan.md
//! §4).
//!
//! DSP-only consumers of this crate never enable the feature and so never
//! pull in the GUI stack.

/// The platform's GUI runtime editor, re-exported so plugin factories can
/// name `editor_host::RuntimeEditor` instead of a platform crate
/// (macos-editor-plan.md items 3a/3c).
#[cfg(target_os = "linux")]
pub use wayland_plugin_gui::Editor as RuntimeEditor;
#[cfg(target_os = "macos")]
pub use cocoa_plugin_gui::Editor as RuntimeEditor;

/// The CLAP window-api name of the platform's native GUI runtime —
/// `"wayland"` where [`RuntimeEditor`] is the Wayland runtime, `"cocoa"`
/// where it is the Cocoa one. Factories use it in `supports`/`preferred`
/// so negotiation is platform-correct everywhere at once instead of 11
/// hard-coded strings (macos-editor-plan.md item 3c).
pub fn native_api() -> &'static str {
    if cfg!(target_os = "macos") {
        "cocoa"
    } else {
        "wayland"
    }
}

// The platform-neutral halves of the editor contract, re-exported for the
// same reason: one import surface for factories on every platform.
pub use plugin_gui_core::{EditorApp, EditorError, EditorOptions};

use crate::gui::PluginEditor;

/// Adapts a running [`RuntimeEditor`] to [`PluginEditor`].
///
/// Construct one with [`RuntimeEditorHandle::new`] in an
/// [`EditorFactory::create`](crate::gui::EditorFactory::create) and box it;
/// the host owns it from then on and dropping the box closes the window.
///
/// Two details are load-bearing:
///
/// * the runtime is held in an `Option` purely so `Drop` can move it out —
///   `Editor::destroy` consumes the editor, and a `Drop` impl only ever
///   gets `&mut self`. Outside teardown the slot is always `Some`;
/// * every method tolerates the empty slot instead of unwrapping, so a
///   host that calls into a handle it is in the middle of dropping gets a
///   no-op rather than a panic across the FFI boundary.
///
/// `set_title` is left at the trait's no-op default: the runtime has no
/// command for retitling a live window yet, and no host we support calls
/// `suggest_title`.
pub struct RuntimeEditorHandle {
    runtime: Option<RuntimeEditor>,
    /// Last size we know about, used only when `runtime` is gone.
    size: (u32, u32),
}

impl RuntimeEditorHandle {
    /// Take ownership of a freshly created editor window.
    pub fn new(runtime: RuntimeEditor) -> Self {
        // Seeded from the runtime rather than from a second caller-supplied
        // size, which is the same value (the window was just created from
        // `EditorOptions::initial_size`) but cannot drift away from it.
        let size = runtime.get_size();
        Self {
            runtime: Some(runtime),
            size,
        }
    }
}

impl PluginEditor for RuntimeEditorHandle {
    fn show(&mut self) {
        if let Some(r) = &self.runtime {
            r.show();
        }
    }

    fn hide(&mut self) {
        if let Some(r) = &self.runtime {
            r.hide();
        }
    }

    fn size(&self) -> (u32, u32) {
        // The compositor owns the window size, so report what the
        // runtime last applied rather than what was last requested:
        // that is what lets the host persist and restore the size the
        // user actually left the window at (ba todo #1337). The cached
        // field is only a fallback for a handle whose runtime is gone.
        self.runtime
            .as_ref()
            .map(|r| r.get_size())
            .unwrap_or(self.size)
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if let Some(r) = &mut self.runtime {
            if r.set_size(width, height).is_ok() {
                self.size = (width, height);
                return true;
            }
        }
        false
    }

    fn can_resize(&self) -> bool {
        self.runtime
            .as_ref()
            .map(|r| r.is_resizable())
            .unwrap_or(false)
    }

    /// Forwarded to the runtime, which raises it after
    /// [`EditorApp::on_close`] on a user close (and when the editor dies),
    /// and disarms it in `destroy` — so dropping this handle never reports
    /// a close. This is what gets every plugin `clap_host_gui.closed()`
    /// without any plugin implementing anything (PLG-01).
    fn set_closed_callback(&mut self, on_closed: Box<dyn FnOnce() + Send>) {
        if let Some(r) = &self.runtime {
            r.set_closed_callback(on_closed);
        }
    }
}

impl Drop for RuntimeEditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() {
            r.destroy();
        }
    }
}

/// An editor app with the plugin's [`crate::host::EditAnnouncer`] lent to
/// every frame, so the param-bound controls in
/// [`crate::editor_widgets`] can tell the host about the user's edits
/// (code review PUX-01). Build it with [`with_announcer`] in the
/// factory's `create`, and hand it to [`RuntimeEditor::new`] in place
/// of the bare app.
pub struct AnnouncingApp<A: EditorApp> {
    app: A,
    announcer: crate::host::EditAnnouncer,
}

/// Wrap `app` so its controls announce through `announcer`.
pub fn with_announcer<A: EditorApp>(
    app: A,
    announcer: crate::host::EditAnnouncer,
) -> AnnouncingApp<A> {
    AnnouncingApp { app, announcer }
}

impl<A: EditorApp> EditorApp for AnnouncingApp<A> {
    fn ui(&mut self, ui: &mut plugin_gui_core::egui::Ui) {
        crate::editor_widgets::install_announcer(ui.ctx(), &self.announcer);
        self.app.ui(ui);
    }

    fn on_close(&mut self) {
        self.app.on_close();
    }
}
