//! Optional GUI hook for `ResonancePlugin`.
//!
//! Plugins that want to provide their own editor window implement
//! [`EditorFactory`] and return an `Arc<dyn EditorFactory>` from
//! [`crate::plugin::ResonancePlugin::editor_factory`]. The clap_bridge
//! exposes this through `CLAP_EXT_GUI`, which the host (or any CLAP host)
//! calls to negotiate the windowing API, create the editor window, resize
//! it, and destroy it.
//!
//! The factory is created once per plugin instance (at
//! `new_main_thread` time) and kept around for the plugin's lifetime. Each
//! `gui_create` call asks the factory to produce a fresh [`PluginEditor`].
//!
//! These traits are deliberately minimal so plugin authors can build on any
//! GUI runtime (egui, iced, raw OpenGL, etc.) without taking a dependency on
//! a specific one. For the wavetable synth we use the sibling
//! `wayland-plugin-gui` crate, but nothing here requires it.

/// A live editor window created by an [`EditorFactory`]. Owns the GUI thread
/// and resources; dropping the box must tear everything down cleanly.
pub trait PluginEditor: Send {
    /// Show the editor window. Idempotent.
    fn show(&mut self);

    /// Hide the editor window. Idempotent.
    fn hide(&mut self);

    /// Current window size in logical pixels.
    fn size(&self) -> (u32, u32);

    /// Request the window be resized. Returns `true` on success.
    fn set_size(&mut self, width: u32, height: u32) -> bool;

    /// Whether this editor can be resized by the host.
    fn can_resize(&self) -> bool;

    /// Set the window title. Mostly useful for floating windows so the host
    /// can push e.g. the track name into the title bar.
    fn set_title(&mut self, _title: &str) {}
}

/// Factory that produces [`PluginEditor`] instances on demand.
///
/// The factory is a pure handle to the plugin's shared state (parameters,
/// visualization atomics, etc.) and should be cheap to construct and clone.
/// Each `create` call is expected to spawn a fresh editor window.
pub trait EditorFactory: Send + Sync {
    /// Whether this factory can produce an editor for the given windowing
    /// API and floating mode. `api_name` is the CLAP api string (e.g.
    /// `"wayland"`, `"x11"`, `"win32"`, `"cocoa"`).
    fn supports(&self, api_name: &str, is_floating: bool) -> bool;

    /// The factory's preferred API/floating combo, reported back to the host
    /// via `clap_plugin_gui::get_preferred_api`. `None` means no preference.
    fn preferred(&self) -> Option<(&'static str, bool)>;

    /// Preferred initial window size in logical pixels.
    fn preferred_size(&self) -> (u32, u32);

    /// Create a new editor. Returns `None` if this api/floating combo isn't
    /// supported or the editor couldn't spawn.
    fn create(&self, api_name: &str, is_floating: bool) -> Option<Box<dyn PluginEditor>>;
}
