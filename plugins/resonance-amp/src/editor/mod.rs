//! Amp plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout mirrors the reverb editor's three-zone structure:
//!
//! - Top strip: header with title, the `Library…` button, the preset bar,
//!   ◀/▶ over the Library's view, ☆/★ and the current model name.
//! - Below the header: a dedicated tuner strip.
//! - Centre: the main visualisation area — live oscilloscope on the
//!   left, static transfer-curve plot on the right, with stereo peak
//!   meters along the bottom — or the missing-model banner.
//! - Bottom: the gain control strip.
//! - Over everything, when open: the Library overlay (`library_panel`).

mod actions;
mod app;
mod controls;
mod curve_view;
mod factory;
mod header;
mod library_panel;
mod meters;
pub mod missing_banner;
mod scope_view;
mod theme;
#[cfg(feature = "editor")]
// `pub` so `tests/tone3000_browser.rs` can assert the presentation-only
// helpers (result heading, filter labels) without an egui context.
pub mod tone3000_panel;
pub mod tuner_view;

pub use factory::AmpEditorFactory;

// Re-exported so the per-section modules can keep their existing
// `super::AmpEditorApp` import path.
pub(crate) use app::AmpEditorApp;

/// The amp editor driven headless, one CPU-only egui frame at a time, for
/// `tests/editor_render.rs`: id collisions, borrow conflicts and panics in
/// the Library overlay are frame-time failures no unit test would see (the
/// plugin editors have no golden-image harness). Not plugin API.
#[doc(hidden)]
pub struct HeadlessEditor {
    app: AmpEditorApp,
    ctx: plugin_gui_core::egui::Context,
}

#[doc(hidden)]
impl HeadlessEditor {
    pub fn new(amp: &crate::ResonanceAmp) -> Self {
        let app = AmpEditorApp::new(
            amp.params.clone(),
            amp.load_request.clone(),
            amp.viz.clone(),
            crate::tone3000::worker::shared(amp.params.library.clone()),
            amp.presets.clone(),
        );
        Self {
            app,
            ctx: plugin_gui_core::egui::Context::default(),
        }
    }

    /// Run one frame at the editor's initial size.
    pub fn frame(&mut self) {
        use plugin_gui_core::{egui, EditorApp};
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(960.0, 620.0),
            )),
            ..Default::default()
        };
        let app = &mut self.app;
        let _ = self.ctx.run_ui(input, |ui| app.ui(ui));
    }

    pub fn open_library(&mut self) {
        self.app.open_library();
        self.app.library_panel.tab = library_panel::Tab::Installed;
    }

    /// Select the first row of the Installed view.
    pub fn select_first(&mut self) {
        self.app.refresh_rows();
        if let Some(&row) = self.app.browser.view().first() {
            let key = self.app.rows.rows[row].key.clone();
            self.app.browser.select(key);
        }
    }

    /// Start the two-click delete of the selected row.
    pub fn begin_delete_selected(&mut self) {
        if let Some(key) = self.app.browser.selected().map(str::to_string) {
            self.app.browser.begin_delete(key);
        }
    }

    pub fn set_query(&mut self, query: &str) {
        self.app.browser.set_query(query);
    }

    pub fn is_library_open(&self) -> bool {
        self.app.library_panel.open
    }
}
