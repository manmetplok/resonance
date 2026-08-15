//! CLAP GUI extension wrapper. Drives the plugin's editor window
//! through the standard `is_api_supported → create → get_size → show`
//! sequence on Wayland. We don't currently implement the embedding
//! path — every editor opens as a floating top-level window.
//!
//! Every step that can refuse is classified into a
//! [`PluginEditorFailure`] rather than a bare `false`, so the engine can
//! report *which* plugin failed and *why* (ba todo #1347); the engine
//! turns that into `AudioEvent::PluginEditorState`.

use clap_sys::ext::gui::CLAP_WINDOW_API_WAYLAND;

use crate::types::PluginEditorFailure;

use super::instance::ClapInstance;

impl ClapInstance {
    /// Whether the plugin exposes a GUI that the host can open.
    pub fn has_gui(&self) -> bool {
        self.gui_ext.is_some()
    }

    /// Whether the host currently believes this plugin's editor window
    /// is open. Flipped by [`Self::open_gui`] / [`Self::close_gui`] and
    /// by a plugin-initiated close picked up through
    /// [`Self::take_gui_closed`].
    pub fn gui_open(&self) -> bool {
        self.gui_open
    }

    /// Open the plugin's editor window as a floating Wayland window.
    ///
    /// Walks the full CLAP GUI negotiation sequence:
    /// `is_api_supported` → `create` → `get_size` → `show`. Returns the
    /// refusing step as a [`PluginEditorFailure`] on failure; on failure
    /// the plugin is left exactly as it was (a successful `create`
    /// followed by a failing `show` is rolled back with `destroy`). If
    /// the GUI is already open, this is a no-op that reports success.
    pub fn open_gui(&mut self) -> Result<(), PluginEditorFailure> {
        let Some(gui) = self.gui_ext else {
            return Err(PluginEditorFailure::NoEditor);
        };
        if self.gui_open {
            return Ok(());
        }
        unsafe {
            let Some(is_supported) = (*gui).is_api_supported else {
                return Err(PluginEditorFailure::UnsupportedWindowApi);
            };
            if !is_supported(self.plugin, CLAP_WINDOW_API_WAYLAND.as_ptr(), true) {
                return Err(PluginEditorFailure::UnsupportedWindowApi);
            }
            let Some(create) = (*gui).create else {
                return Err(PluginEditorFailure::CreateFailed);
            };
            if !create(self.plugin, CLAP_WINDOW_API_WAYLAND.as_ptr(), true) {
                return Err(PluginEditorFailure::CreateFailed);
            }
            // Best-effort size negotiation (ignore errors — the plugin has
            // its own preferred size baked into its factory).
            if let Some(get_size) = (*gui).get_size {
                let mut w: u32 = 0;
                let mut h: u32 = 0;
                get_size(self.plugin, &mut w, &mut h);
                if let Some(set_size) = (*gui).set_size {
                    if w > 0 && h > 0 {
                        set_size(self.plugin, w, h);
                    }
                }
            }
            let Some(show) = (*gui).show else {
                // If show isn't exposed, roll back the create.
                if let Some(destroy) = (*gui).destroy {
                    destroy(self.plugin);
                }
                return Err(PluginEditorFailure::ShowFailed);
            };
            if !show(self.plugin) {
                if let Some(destroy) = (*gui).destroy {
                    destroy(self.plugin);
                }
                return Err(PluginEditorFailure::ShowFailed);
            }
        }
        self.gui_open = true;
        // A `closed()` the plugin fired during a previous editor's life
        // must not be attributed to the window we just opened.
        self.host_data.take_gui_closed();
        Ok(())
    }

    /// Close the plugin's editor window (hide + destroy).
    ///
    /// Returns true when this call actually closed an open editor, so
    /// the engine only reports a transition that happened.
    pub fn close_gui(&mut self) -> bool {
        if !self.gui_open {
            return false;
        }
        if let Some(gui) = self.gui_ext {
            unsafe {
                if let Some(hide) = (*gui).hide {
                    hide(self.plugin);
                }
                if let Some(destroy) = (*gui).destroy {
                    destroy(self.plugin);
                }
            }
        }
        self.gui_open = false;
        // We closed it ourselves; a `closed()` the plugin fires as part
        // of this teardown is not a separate user-initiated close.
        self.host_data.take_gui_closed();
        true
    }

    /// Consume a plugin-initiated close: the user closed the floating
    /// editor from its own titlebar and the plugin told us through
    /// `clap_host_gui.closed()`. Returns true when the editor really
    /// went from open to closed, which is the engine's cue to emit
    /// `AudioEvent::PluginEditorState { open: false, failure: None }`.
    ///
    /// Per the CLAP spec the host owns the teardown unless the plugin
    /// already destroyed the window itself (`was_destroyed`), so we call
    /// `destroy` exactly when the plugin did not.
    ///
    /// Called from the engine thread's once-per-iteration poll while
    /// holding the instance lock — never from the audio callback.
    pub fn take_gui_closed(&mut self) -> bool {
        let Some(was_destroyed) = self.host_data.take_gui_closed() else {
            return false;
        };
        if !self.gui_open {
            // A stale notification for a window we already tore down.
            return false;
        }
        if !was_destroyed {
            if let Some(gui) = self.gui_ext {
                unsafe {
                    if let Some(destroy) = (*gui).destroy {
                        destroy(self.plugin);
                    }
                }
            }
        }
        self.gui_open = false;
        true
    }
}
