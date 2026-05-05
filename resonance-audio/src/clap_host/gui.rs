//! CLAP GUI extension wrapper. Drives the plugin's editor window
//! through the standard `is_api_supported → create → get_size → show`
//! sequence on Wayland. We don't currently implement the embedding
//! path — every editor opens as a floating top-level window.

use clap_sys::ext::gui::CLAP_WINDOW_API_WAYLAND;

use super::instance::ClapInstance;

impl ClapInstance {
    /// Whether the plugin exposes a GUI that the host can open.
    pub fn has_gui(&self) -> bool {
        self.gui_ext.is_some()
    }

    /// Whether the GUI is currently open (i.e. `gui_create` was called).
    pub fn is_gui_open(&self) -> bool {
        self.gui_open
    }

    /// Open the plugin's editor window as a floating Wayland window.
    ///
    /// Walks the full CLAP GUI negotiation sequence:
    /// `is_api_supported` → `create` → `get_size` → `show`. Returns `false`
    /// at any step failure. If the GUI is already open, this is a no-op.
    pub fn open_gui(&mut self) -> bool {
        let Some(gui) = self.gui_ext else {
            return false;
        };
        if self.gui_open {
            return true;
        }
        unsafe {
            let Some(is_supported) = (*gui).is_api_supported else {
                return false;
            };
            if !is_supported(self.plugin, CLAP_WINDOW_API_WAYLAND.as_ptr(), true) {
                return false;
            }
            let Some(create) = (*gui).create else {
                return false;
            };
            if !create(self.plugin, CLAP_WINDOW_API_WAYLAND.as_ptr(), true) {
                return false;
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
                return false;
            };
            if !show(self.plugin) {
                if let Some(destroy) = (*gui).destroy {
                    destroy(self.plugin);
                }
                return false;
            }
        }
        self.gui_open = true;
        true
    }

    /// Close the plugin's editor window (hide + destroy).
    pub fn close_gui(&mut self) {
        if !self.gui_open {
            return;
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
    }
}
