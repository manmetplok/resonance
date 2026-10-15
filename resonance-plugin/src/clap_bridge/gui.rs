//! Embedded GUI lifecycle for the CLAP bridge.

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize, PluginGuiImpl, Window};
use clack_plugin::prelude::*;

use super::shared::ClapMainThread;
use crate::plugin::ResonancePlugin;

impl<'a, P: ResonancePlugin> PluginGuiImpl for ClapMainThread<'a, P> {
    fn is_api_supported(&self, configuration: GuiConfiguration) -> bool {
        let Some(factory) = &self.editor_factory else {
            return false;
        };
        let Ok(api) = configuration.api_type.0.to_str() else {
            return false;
        };
        factory.supports(api, configuration.is_floating)
    }

    fn get_preferred_api(&self) -> Option<GuiConfiguration<'_>> {
        let factory = self.editor_factory.as_ref()?;
        let (api, is_floating) = factory.preferred()?;
        let api_type = match api {
            "wayland" => GuiApiType::WAYLAND,
            "x11" => GuiApiType::X11,
            "win32" => GuiApiType::WIN32,
            "cocoa" => GuiApiType::COCOA,
            _ => return None,
        };
        Some(GuiConfiguration {
            api_type,
            is_floating,
        })
    }

    fn create(&self, configuration: GuiConfiguration) -> Result<(), PluginError> {
        let factory = self
            .editor_factory
            .as_ref()
            .ok_or(PluginError::Message("plugin has no editor factory"))?;
        let api = configuration
            .api_type
            .0
            .to_str()
            .map_err(|_| PluginError::Message("invalid GUI api string"))?;
        let mut editor = factory
            .create(api, configuration.is_floating)
            .ok_or(PluginError::Message("editor creation failed"))?;
        // Route a self-close (the user closed the window from its own
        // titlebar) to `clap_host_gui.closed()`, once, for every plugin:
        // the runtime latches it on the handle from its own thread and
        // `on_main_thread` delivers it (PLG-01).
        let serial = self.editor_serial.get() + 1;
        self.editor_serial.set(serial);
        let host_handle = self.host_handle.clone();
        editor.set_closed_callback(Box::new(move || {
            host_handle.report_gui_closed(serial);
        }));
        // A create over a live editor (a host that skipped `destroy`)
        // replaces it; the old one is torn down outside the borrow.
        let previous = self
            .editor
            .try_borrow_mut()
            .map_err(|_| PluginError::Message("gui.create re-entered an editor call"))?
            .replace(editor);
        self.editor_destroy_pending.set(false);
        drop(previous);
        Ok(())
    }

    fn destroy(&self) {
        self.destroy_editor();
    }

    fn set_scale(&self, _scale: f64) -> Result<(), PluginError> {
        // Deliberately refused, per the CLAP gui contract: `set_scale`
        // "can be ignored if the plugin will query the OS directly", and
        // returning false tells the host exactly that. On both platforms
        // the windowing API speaks logical pixels and the window system is
        // authoritative for the scale factor, so the GUI runtime queries
        // it directly: the Wayland runtime derives its integer buffer
        // scale from `CompositorHandler::scale_factor_changed`
        // (wayland-plugin-gui's `State::buffer_scale` feeds
        // `wl_surface.set_buffer_scale`, the EGL buffer size, and egui's
        // `pixels_per_point`), and the Cocoa runtime reads the view's
        // `backingScaleFactor`, re-read on
        // `viewDidChangeBackingProperties`. Applying a host-supplied
        // factor on top would double-scale either one. clack maps this
        // `Err` to a `false` return at the C boundary and, since 0.2
        // (cddee07), logs its message through `clap_host_log` (stderr when
        // the host has no log extension) — one line per `set_scale` call,
        // which hosts make once per editor open at most.
        Err(PluginError::Message(
            "scale is derived from the window system",
        ))
    }

    fn get_size(&self) -> Option<GuiSize> {
        let (width, height) = match self.with_editor(|editor| editor.size()) {
            Some(size) => size,
            None => self.editor_factory.as_ref()?.preferred_size(),
        };
        Some(GuiSize { width, height })
    }

    fn can_resize(&self) -> bool {
        self.with_editor(|editor| editor.can_resize())
            .unwrap_or(false)
    }

    fn set_size(&self, size: GuiSize) -> Result<(), PluginError> {
        match self.with_editor(|editor| editor.set_size(size.width, size.height)) {
            Some(true) => Ok(()),
            Some(false) => Err(PluginError::Message("set_size refused")),
            None => Err(PluginError::Message("no editor to resize")),
        }
    }

    fn set_parent(&self, _window: Window) -> Result<(), PluginError> {
        // Floating-only in v1 on both platforms, so there is nothing to
        // parent to. Pretend to succeed so hosts that call set_parent
        // unconditionally (even with is_floating=true) don't fail the
        // handshake. This becomes a real implementation when embedded
        // mode lands (macos-editor-plan.md item 3g: attach the runtime's
        // NSView to the host-supplied one), not before.
        Ok(())
    }

    fn set_transient(&self, _window: Window) -> Result<(), PluginError> {
        // v1: no-op. Could later map to xdg-foreign-unstable-v2 on Wayland
        // (or the host NSWindow on macOS) to mark the plugin window as
        // transient for the host window.
        Ok(())
    }

    fn suggest_title(&self, title: &str) {
        self.with_editor(|editor| editor.set_title(title));
    }

    fn show(&self) -> Result<(), PluginError> {
        self.with_editor(|editor| editor.show())
            .ok_or(PluginError::Message("no editor to show"))
    }

    fn hide(&self) -> Result<(), PluginError> {
        self.with_editor(|editor| editor.hide())
            .ok_or(PluginError::Message("no editor to hide"))
    }
}
