//! Options for creating an editor window, shared by every runtime.

/// Options passed to a runtime's `Editor::new`.
#[derive(Debug, Clone)]
pub struct EditorOptions {
    pub title: String,
    /// Wayland `app_id` (reverse-DNS) for the editor window. The
    /// compositor uses this for taskbar grouping, `xdg-foreign`
    /// parenting, and window-rule matching. Plugins **must** override
    /// the default to a plugin-specific id; otherwise every editor
    /// hosted by this crate collides in the compositor. Runtimes on
    /// platforms without the concept (Cocoa) ignore it.
    pub app_id: String,
    pub initial_size: (u32, u32),
    pub min_size: (u32, u32),
    pub resizable: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            title: "Plugin Editor".to_string(),
            // Generic fallback. Callers should pass their own.
            app_id: "com.resonance.plugin".to_string(),
            initial_size: (800, 600),
            min_size: (400, 300),
            resizable: true,
        }
    }
}
