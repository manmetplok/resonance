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
mod jobs;
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
/// `tests/model_library.rs`: what the Library overlay, the header and the
/// missing banner actually draw, read back from the frame's text shapes
/// (the plugin editors have no golden-image harness). Not plugin API.
///
/// Uses an offline Tone3000 worker: nothing is read from the user's saved
/// session.
#[doc(hidden)]
pub struct HeadlessEditor {
    app: AmpEditorApp,
    ctx: plugin_gui_core::egui::Context,
    size: plugin_gui_core::egui::Vec2,
}

#[doc(hidden)]
impl HeadlessEditor {
    pub fn new(amp: &crate::ResonanceAmp) -> Self {
        let worker = std::sync::Arc::new(crate::tone3000::worker::spawn_offline(
            amp.params.library.clone(),
        ));
        let mut app = AmpEditorApp::new(
            amp.params.clone(),
            amp.load_request.clone(),
            amp.viz.clone(),
            worker,
            amp.presets.clone(),
        );
        // The editor-open rescan, finished before the first frame.
        if let Some(done) = app.jobs.wait() {
            app.apply_job(done);
        }
        Self {
            app,
            ctx: plugin_gui_core::egui::Context::default(),
            size: plugin_gui_core::egui::vec2(960.0, 620.0),
        }
    }

    /// Lay the editor out at `width`×`height` from now on (the editor's
    /// minimum is 760×520).
    pub fn set_size(&mut self, width: f32, height: f32) {
        self.size = plugin_gui_core::egui::vec2(width, height);
    }

    /// Run one frame and return every text it drew.
    pub fn frame(&mut self) -> Vec<String> {
        use plugin_gui_core::{egui, EditorApp};
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            ..Default::default()
        };
        let app = &mut self.app;
        let out = self.ctx.run_ui(input, |ui| app.ui(ui));
        let mut texts = Vec::new();
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        for s in &out.shapes {
            walk(&s.shape, &mut texts);
        }
        texts
    }

    /// Finish the running background job (rescan, import, delete) and
    /// apply it, as the next frame would once it is done.
    pub fn finish_jobs(&mut self) {
        if let Some(done) = self.app.jobs.wait() {
            self.app.apply_job(done);
        }
    }

    pub fn open_library(&mut self) {
        self.app.open_library();
        self.app.library_panel.tab = library_panel::Tab::Installed;
    }

    pub fn open_tone3000_tab(&mut self) {
        self.app.open_library();
        self.app.library_panel.tab = library_panel::Tab::Tone3000;
    }

    pub fn is_library_open(&self) -> bool {
        self.app.library_panel.open
    }

    /// Select the row at `pos` in the Installed view.
    pub fn select_in_view(&mut self, pos: usize) {
        self.app.refresh_rows();
        if let Some(&row) = self.app.browser.view().get(pos) {
            let key = self.app.rows.rows[row].key.clone();
            self.app.browser.select(key);
        }
    }

    /// The selected row's key.
    pub fn selected(&self) -> Option<String> {
        self.app.browser.selected().map(str::to_string)
    }

    /// Arm the two-click delete of the selected row.
    pub fn begin_delete_selected(&mut self) {
        if let Some(key) = self.app.browser.selected().map(str::to_string) {
            self.app.browser.begin_delete(key);
        }
    }

    pub fn pending_delete(&self) -> Option<String> {
        self.app.browser.pending_delete().map(str::to_string)
    }

    pub fn set_query(&mut self, query: &str) {
        self.app.browser.set_query(query);
    }

    /// ◀ (`-1`) or ▶ (`1`) in the header.
    pub fn step(&mut self, delta: i32) {
        header::step_from_header(&mut self.app, delta);
    }

    /// Sort the Installed view by `key`.
    pub fn set_sort(&mut self, key: resonance_plugin::library_view::SortKey) {
        self.app
            .browser
            .set_sort(resonance_plugin::library_view::Sort::by(key));
    }

    /// The callback a detail-pane Re-download of `entry` would finish with.
    pub fn detail_redownload_done(
        &self,
        entry: &resonance_common::nam_library::Entry,
    ) -> crate::tone3000::worker::DownloadDone {
        actions::redownload_only_done(&self.app, entry)
    }

    /// The header's re-download notice, if live.
    pub fn redownload_notice(&self) -> Option<String> {
        actions::live_notice(&self.app)
    }

    /// Put the missing banner into its "use this file anyway?" state.
    pub fn locate_mismatch(&mut self, path: std::path::PathBuf) {
        self.app.missing.mismatch = Some(path);
    }
}
