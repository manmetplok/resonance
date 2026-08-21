//! The trait a hosted editor implements.

/// A thin trait implemented by the hosted editor application.
///
/// Called on the editor thread on every repaint. The `ui()` method receives a
/// top-level [`egui::Ui`] that covers the whole window; use `show_inside`
/// variants of `egui::CentralPanel` / `TopBottomPanel` / `SidePanel` to
/// subdivide it.
///
/// The egui [`egui::Context`] is reachable from the `Ui` via `ui.ctx()` if you
/// need to call things like `request_repaint()` or `pixels_per_point()`.
pub trait EditorApp: Send + 'static {
    /// Build one frame of the UI. Called on the editor thread.
    fn ui(&mut self, ui: &mut egui::Ui);

    /// Called when the window is about to close. Default: no-op.
    fn on_close(&mut self) {}
}
