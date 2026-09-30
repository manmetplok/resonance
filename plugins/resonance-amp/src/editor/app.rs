//! The actual egui app: state and update/view orchestration for the amp editor.
//!
//! `AmpEditorApp` is the `EditorApp` the runtime drives each frame. It paints
//! the chrome panels (header, tuner, control strip) and dispatches the centre
//! to the scope/curve/meters views, or to the missing-model banner.

use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use parking_lot::Mutex;

use plugin_gui_core::{egui, EditorApp};

use crate::params::AmpParams;
use crate::tone3000::worker::WorkerHandle;
use crate::viz::AmpViz;

use resonance_plugin::library_view::BrowserModel;

use super::library_panel::{self, LibraryPanelState};
use super::missing_banner::{self, MissingBannerState};
use super::tone3000_panel::Tone3000PanelState;
use super::{controls, curve_view, header, meters, scope_view, theme, tuner_view};
use crate::library_rows::ModelRows;
use resonance_common::library_marks::{FreshnessPoll, BAR_POLL_INTERVAL, BROWSER_POLL_INTERVAL};

pub(crate) struct AmpEditorApp {
    pub(crate) params: Arc<AmpParams>,
    pub(crate) load_request: Arc<AtomicI32>,
    pub(crate) viz: Arc<AmpViz>,
    pub(crate) tone3000: Arc<WorkerHandle>,
    pub(crate) tone3000_panel: Tone3000PanelState,
    /// This plugin ships no factory presets, so the bank is the user's
    /// own directory alone (ba todo #1358).
    pub(crate) bank: resonance_plugin::presets::PresetBank,
    /// Shared with the plugin struct, so what the bar shows is what
    /// `save_state` persists.
    pub(crate) presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Transient bar state (open combo, in-progress rename), editor-only.
    pub(crate) preset_editor: resonance_plugin::presets::PresetEditor,
    pub(crate) missing: MissingBannerState,
    /// A one-line message for the header (import results, errors).
    pub(crate) notice: Option<String>,
    pub(crate) library_panel: LibraryPanelState,
    /// The Installed tab's view-state. Lives here, not in the panel, because
    /// the header's ◀/▶ step through the same view with the panel closed.
    pub(crate) browser: BrowserModel,
    /// The library as browser rows, rebuilt when the library changes.
    pub(crate) rows: ModelRows,
    /// The `+ tag` field's text in the detail pane.
    pub(crate) tag_draft: String,
    /// Change detection for other processes' marks writes.
    pub(crate) marks_poll: FreshnessPoll,
    /// Change detection for the library folders and index (files added or
    /// deleted in a file manager, another process's rescan).
    pub(crate) library_poll: FreshnessPoll,
    /// The background rescan, if one is running. Joined before the editor
    /// goes away, so no thread outlives the plugin image.
    pub(crate) rescan_thread: Option<std::thread::JoinHandle<()>>,
    /// Set by a finished re-download whose bytes differ from the saved id.
    pub(crate) redownload_notice: Arc<Mutex<Option<String>>>,
}

impl AmpEditorApp {
    pub(crate) fn new(
        params: Arc<AmpParams>,
        load_request: Arc<AtomicI32>,
        viz: Arc<AmpViz>,
        tone3000: Arc<WorkerHandle>,
        presets: Arc<resonance_plugin::presets::PresetSession>,
    ) -> Self {
        Self {
            params,
            load_request,
            viz,
            tone3000,
            tone3000_panel: Tone3000PanelState::default(),
            bank: resonance_plugin::presets::PresetBank::new(
                <crate::ResonanceAmp as resonance_plugin::ResonancePlugin>::CLAP_ID,
                <crate::ResonanceAmp as resonance_plugin::ResonancePlugin>::FACTORY_PRESETS,
            ),
            presets,
            preset_editor: resonance_plugin::presets::PresetEditor::default(),
            missing: MissingBannerState::default(),
            notice: None,
            library_panel: LibraryPanelState::default(),
            browser: BrowserModel::new(),
            rows: ModelRows::default(),
            tag_draft: String::new(),
            marks_poll: FreshnessPoll::new(Vec::new(), BAR_POLL_INTERVAL),
            library_poll: FreshnessPoll::new(Vec::new(), BAR_POLL_INTERVAL),
            rescan_thread: None,
            redownload_notice: Arc::new(Mutex::new(None)),
        }
    }

    /// Open the Library overlay.
    pub(crate) fn open_library(&mut self) {
        library_panel::open(self);
    }

    /// Rebuild the rows if the library or the marks changed, and refresh
    /// the view.
    pub(crate) fn refresh_rows(&mut self) {
        let revision = self.params.library.revision();
        let marks_gen = self.params.library.marks_generation();
        if self.rows.built_from != (revision, marks_gen) || self.rows.rows.is_empty() {
            let lib = self.params.library.read();
            let marks = self.params.library.marks();
            self.rows = ModelRows::build(&lib, Some(&marks), (revision, marks_gen));
        }
        // The view's cache key folds both counters together.
        self.browser
            .refresh(&self.rows, revision.wrapping_mul(1_000_003) ^ marks_gen);
    }

    /// Poll for other processes' changes: every 500 ms while the Library is
    /// open, every 2 s from the header, one `stat` per path each time.
    pub(crate) fn poll_freshness(&mut self) {
        if self.marks_poll.targets().is_empty() {
            let marks = self.params.library.marks().path();
            self.marks_poll = FreshnessPoll::new(vec![marks], BAR_POLL_INTERVAL);
        }
        self.marks_poll.set_interval(if self.library_panel.open {
            BROWSER_POLL_INTERVAL
        } else {
            BAR_POLL_INTERVAL
        });
        let now = std::time::Instant::now();
        if self.marks_poll.check(now) {
            self.params.library.refresh_marks();
        }

        if self.library_poll.targets().is_empty() {
            let paths = self.params.library.read().watch_paths();
            self.library_poll = FreshnessPoll::new(paths, BAR_POLL_INTERVAL);
            self.library_poll.mark_seen(now);
        }
        self.library_poll.set_interval(self.marks_poll_interval());
        if self.rescan_thread.as_ref().is_some_and(|t| t.is_finished()) {
            if let Some(t) = self.rescan_thread.take() {
                let _ = t.join();
            }
        }
        if self.rescan_thread.is_none() && self.library_poll.check(now) {
            // Off the UI thread: a new file has to be hashed. The rescan
            // only re-hashes files whose size or mtime changed.
            let library = self.params.library.clone();
            self.rescan_thread = std::thread::Builder::new()
                .name("amp-library-rescan".into())
                .spawn(move || {
                    if let Err(e) = library.rescan() {
                        tracing::warn!("model library rescan failed: {e}");
                    }
                })
                .ok();
        }
        // The rescan's own index write shows up as one more change on the
        // next poll; that rescan finds nothing to hash and writes nothing,
        // so the poll settles.
    }

    fn marks_poll_interval(&self) -> std::time::Duration {
        if self.library_panel.open {
            BROWSER_POLL_INTERVAL
        } else {
            BAR_POLL_INTERVAL
        }
    }

    /// Toggle the favourite of model `id`, reporting a failed write.
    pub(crate) fn toggle_favorite(&mut self, id: &str) {
        if let Err(e) = self.params.library.toggle_favorite(id) {
            self.browser.set_error(format!("could not save the favourite: {e}"));
        }
    }

    /// How many amps in this process are playing `id`.
    pub(crate) fn usage_count(&self, id: &str) -> usize {
        self.params.library.usage_count(id)
    }
}

impl Drop for AmpEditorApp {
    fn drop(&mut self) {
        if let Some(t) = self.rescan_thread.take() {
            let _ = t.join();
        }
    }
}

impl EditorApp for AmpEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        theme::apply(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
        self.poll_freshness();

        egui::Panel::top("amp_header")
            .exact_size(38.0)
            .show_inside(ui, |ui| header::draw(ui, self));

        egui::Panel::top("amp_tuner")
            .exact_size(72.0)
            .show_inside(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                let painter = ui.painter_at(rect);
                tuner_view::draw(&painter, rect, &self.viz);
            });

        egui::Panel::bottom("amp_strip")
            .exact_size(140.0)
            .show_inside(ui, |ui| controls::draw(ui, &self.params));

        egui::CentralPanel::default().show_inside(ui, |ui| draw_center(ui, self));

        if self.library_panel.open {
            library_panel::draw(ui, self);
        }
    }
}

fn draw_center(ui: &mut egui::Ui, app: &mut AmpEditorApp) {
    let avail = ui.available_rect_before_wrap();
    let gap = 8.0f32;
    let meter_h = 28.0f32;

    let viz_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.top() + gap),
        egui::pos2(avail.right() - gap, avail.bottom() - meter_h - gap),
    );
    let meter_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left() + gap, avail.bottom() - meter_h),
        egui::pos2(avail.right() - gap, avail.bottom() - 2.0),
    );

    // Split the viz area: scope (left ~65%) + transfer curve (right ~35%).
    let curve_w = 280.0f32.min(viz_rect.width() * 0.4);
    let scope_rect = egui::Rect::from_min_max(
        viz_rect.min,
        egui::pos2(viz_rect.right() - curve_w - gap, viz_rect.bottom()),
    );
    let curve_rect = egui::Rect::from_min_max(
        egui::pos2(scope_rect.right() + gap, viz_rect.top()),
        viz_rect.max,
    );

    let painter = ui.painter_at(avail);
    let status = app.params.status.lock().clone();
    if status.is_missing() {
        // The scope and curve have nothing real to draw with no model: the
        // banner takes their place (nam-model-library.md §6.4).
        missing_banner::draw(ui, viz_rect, app, &status);
    } else {
        scope_view::draw(&painter, scope_rect, &app.viz);
        curve_view::draw(&painter, curve_rect, &app.viz);
    }
    meters::draw(&painter, meter_rect, &app.viz);
}
