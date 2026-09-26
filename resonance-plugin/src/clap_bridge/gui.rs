//! Embedded GUI lifecycle for the CLAP bridge.

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize, PluginGuiImpl, Window};
use clack_plugin::prelude::*;

use super::shared::ClapMainThread;
use crate::plugin::ResonancePlugin;

impl<'a, P: ResonancePlugin> PluginGuiImpl for ClapMainThread<'a, P> {
    fn is_api_supported(&mut self, configuration: GuiConfiguration) -> bool {
        let Some(factory) = &self.editor_factory else {
            return false;
        };
        let Ok(api) = configuration.api_type.0.to_str() else {
            return false;
        };
        factory.supports(api, configuration.is_floating)
    }

    fn get_preferred_api(&mut self) -> Option<GuiConfiguration<'_>> {
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

    fn create(&mut self, configuration: GuiConfiguration) -> Result<(), PluginError> {
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
        self.editor_serial += 1;
        let serial = self.editor_serial;
        let host_handle = self.host_handle.clone();
        editor.set_closed_callback(Box::new(move || {
            host_handle.report_gui_closed(serial);
        }));
        self.editor = Some(editor);
        Ok(())
    }

    fn destroy(&mut self) {
        self.editor = None;
    }

    fn set_scale(&mut self, _scale: f64) -> Result<(), PluginError> {
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
        // `Err` to a plain `false` return at the C boundary (no host
        // error log).
        Err(PluginError::Message(
            "scale is derived from the window system",
        ))
    }

    fn get_size(&mut self) -> Option<GuiSize> {
        if let Some(editor) = &self.editor {
            let (w, h) = editor.size();
            Some(GuiSize {
                width: w,
                height: h,
            })
        } else if let Some(factory) = &self.editor_factory {
            let (w, h) = factory.preferred_size();
            Some(GuiSize {
                width: w,
                height: h,
            })
        } else {
            None
        }
    }

    fn can_resize(&mut self) -> bool {
        self.editor
            .as_ref()
            .map(|e| e.can_resize())
            .unwrap_or(false)
    }

    fn set_size(&mut self, size: GuiSize) -> Result<(), PluginError> {
        let editor = self
            .editor
            .as_mut()
            .ok_or(PluginError::Message("no editor to resize"))?;
        if editor.set_size(size.width, size.height) {
            Ok(())
        } else {
            Err(PluginError::Message("set_size refused"))
        }
    }

    fn set_parent(&mut self, _window: Window) -> Result<(), PluginError> {
        // Floating-only in v1 on both platforms, so there is nothing to
        // parent to. Pretend to succeed so hosts that call set_parent
        // unconditionally (even with is_floating=true) don't fail the
        // handshake. This becomes a real implementation when embedded
        // mode lands (macos-editor-plan.md item 3g: attach the runtime's
        // NSView to the host-supplied one), not before.
        Ok(())
    }

    fn set_transient(&mut self, _window: Window) -> Result<(), PluginError> {
        // v1: no-op. Could later map to xdg-foreign-unstable-v2 on Wayland
        // (or the host NSWindow on macOS) to mark the plugin window as
        // transient for the host window.
        Ok(())
    }

    fn suggest_title(&mut self, title: &str) {
        if let Some(editor) = &mut self.editor {
            editor.set_title(title);
        }
    }

    fn show(&mut self) -> Result<(), PluginError> {
        if let Some(editor) = &mut self.editor {
            editor.show();
            Ok(())
        } else {
            Err(PluginError::Message("no editor to show"))
        }
    }

    fn hide(&mut self) -> Result<(), PluginError> {
        if let Some(editor) = &mut self.editor {
            editor.hide();
            Ok(())
        } else {
            Err(PluginError::Message("no editor to hide"))
        }
    }
}
