//! The bridge from a live `wayland-plugin-gui` window to the host.
//!
//! [`crate::gui::PluginEditor`] is what the CLAP bridge calls when a host
//! opens, moves, resizes or closes an editor; `wayland_plugin_gui::Editor`
//! is the window that actually exists. The two are deliberately unrelated
//! — `gui.rs` names no GUI runtime so a plugin can be built on anything —
//! so *something* has to sit between them, and that something is
//! [`RuntimeEditorHandle`].
//!
//! It lives here, feature-gated behind `editor-widgets` alongside the other
//! wayland-gui glue, because it is the same code for every plugin: the
//! adapter is a property of the two traits, not of any plugin's UI. All 11
//! first-party plugins carried a verbatim copy of it until ba todo #1336.
//!
//! DSP-only consumers of this crate never enable the feature and so never
//! pull in the GUI stack.

/// The platform's GUI runtime editor, re-exported so plugin factories can
/// name `editor_host::RuntimeEditor` instead of a platform crate
/// (macos-editor-plan.md item 3a; the factories migrate in 3c, and the
/// macOS arm switches to `cocoa_plugin_gui::Editor` in 3b).
pub use wayland_plugin_gui::Editor as RuntimeEditor;

// The platform-neutral halves of the editor contract, re-exported for the
// same reason: one import surface for factories on every platform.
pub use plugin_gui_core::{EditorApp, EditorError, EditorOptions};

use crate::gui::PluginEditor;

/// Adapts a running `wayland_plugin_gui::Editor` to [`PluginEditor`].
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
}

impl Drop for RuntimeEditorHandle {
    fn drop(&mut self) {
        if let Some(r) = self.runtime.take() {
            r.destroy();
        }
    }
}
