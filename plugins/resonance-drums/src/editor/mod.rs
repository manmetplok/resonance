//! Drums plugin editor: an egui UI hosted by the platform GUI runtime.
//!
//! Layout (drums-plugin-rework.md §6, slice K5):
//!
//! - the header: brand, the kit (☆/★, ◀ dropdown ▶), `Library…`, and the
//!   kit's load progress and warnings (`chrome.rs`);
//! - the tab bar: `[Pads | Mix | Setup]` and the labelled preset bar;
//! - the body: the Pads tab (6×5 grid + inspector, `pads_tab.rs`), the
//!   Mix tab (port strips, the per-pad table, globals, `mix_tab.rs`) or
//!   the Setup tab (mic banks, streaming, kit facts, routing, `setup_tab.rs`),
//!   every region scrolling where its content can outgrow the window;
//! - the status bar: only real readings — sample rate, pads, memory,
//!   underruns, the last hit and the OUT meter;
//! - the Library overlay (`library_panel.rs`, with its `plok.org` tab in
//!   `plok_panel.rs`) over everything when open.
//!
//! Every edit of a param is one undoable host edit (`controls.rs`).
//!
//! Every control comes from `plugin_gui_core::widgets` — the knobs
//! always did, and ba todo #1335 retired the local `editor/widgets/`
//! copies of the chip, the segmented control and the slider. That
//! module's own comment admitted they were "duplicated from
//! resonance-wavetable so the two editors can evolve independently";
//! what they actually did was drift (ba doc #275).

mod app;
mod chrome;
mod controls;
mod factory;
mod jobs;
mod kit_browser;
mod library_panel;
mod missing_kit;
mod mix_tab;
mod pad_grid;
mod pad_inspector;
mod pads_tab;
mod plok_panel;
mod setup_tab;
mod theme;

pub use factory::DrumsEditorFactory;

use std::sync::Arc;

use parking_lot::Mutex;
use plugin_gui_core::egui;

// Re-exported so the per-section modules (`pad_inspector`, `kit_browser`)
// can keep their existing `super::reload_kit` import path. The helper
// itself lives at `crate::reload` because the articulation watcher needs
// it in headless builds too.
pub(crate) use crate::reload::reload_kit_acting as reload_kit;

/// The width left in `ui` once `inset` — the fixed gaps of a row about to
/// be split into columns, or a widget's own chrome — is taken off,
/// floored at zero.
///
/// A window narrower than those gaps makes the plain subtraction
/// negative, and `Ui::set_min_width` / `set_width` carry a
/// `debug_assert!(0.0 <= width)`: a panic on the editor thread in a debug
/// build. A compositor that tiles plugin windows can produce the narrow
/// case without the user doing anything unusual (ba todo #1377).
///
/// A frame that only wants to fill its column does not need this: inside
/// `Frame::show` the available width already excludes the frame's
/// margins, so `ui.set_min_width(ui.available_width())` is enough. The
/// cards here used to subtract their margins a second time, and with the
/// body's columns accidentally laid out left to right (see
/// `app::column`) the result could go negative — which is where this
/// floor came from.
///
/// Zero is the honest floor rather than a minimum like 40px: asking for
/// nothing lets egui lay the content out and clip it, which degrades far
/// better than forcing a width the window does not have.
pub(crate) fn body_width(ui: &egui::Ui, inset: f32) -> f32 {
    (ui.available_width() - inset).max(0.0)
}

/// Test-only: draw the pad inspector into `ui`, so a test can hand it a
/// width and see what it does with it.
///
/// The editor module is private and the inspector needs a kit bridge and
/// a mic catalogue, which is why this hook exists rather than the test
/// calling `pad_inspector::draw` itself.
#[doc(hidden)]
pub fn test_draw_pad_inspector(ui: &mut egui::Ui, bridge: &crate::KitBridge, selected_pad: usize) {
    let mut velocity = 100;
    let mut labels = controls::Labels::default();
    pad_inspector::draw(
        ui,
        bridge,
        &crate::mic_catalog::ManifestMicCatalog::default(),
        selected_pad,
        &mut pad_inspector::InspectorState {
            audition_velocity: &mut velocity,
            labels: &mut labels,
        },
    );
}

/// One painted text as it landed on screen: the string, its visual
/// rect, and the clip rect it was painted under. A label scrolled or
/// squeezed out of view is still in the shape list — `Painter::text`
/// emits it whatever the clip — so "the text was drawn" proves nothing;
/// `rect ∩ clip` is what the user actually sees.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ProbedText {
    pub text: String,
    pub rect: egui::Rect,
    pub clip: egui::Rect,
}

/// One widget rect the editor reported through [`probe`], with the clip
/// rect of the `Ui` it was laid out in.
///
/// `probe()` (production code, always compiled under `editor`) only ever
/// *writes* these; only [`EditorFrameProbe::widget`] (`test-hooks`) reads
/// the fields back, so a build without `test-hooks` never reads one.
#[doc(hidden)]
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "test-hooks"), allow(dead_code))]
pub struct ProbedRect {
    pub name: String,
    pub rect: egui::Rect,
    pub clip: egui::Rect,
}

/// Everything `test_render_editor_frame` read back from a settled frame.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct EditorFrameProbe {
    pub screen: egui::Rect,
    pub texts: Vec<ProbedText>,
    pub widgets: Vec<ProbedRect>,
    /// Every shape the frame painted, in paint order.
    pub shapes: Vec<egui::epaint::ClippedShape>,
}

#[cfg(feature = "test-hooks")]
impl EditorFrameProbe {
    /// Just the strings, for "is it drawn at all" checks.
    pub fn strings(&self) -> Vec<String> {
        self.texts.iter().map(|t| t.text.clone()).collect()
    }

    /// Whether `needle` was painted with at least part of it visible
    /// (inside its clip and the window).
    pub fn shows(&self, needle: &str) -> bool {
        self.texts.iter().any(|t| {
            let v = t.rect.intersect(t.clip).intersect(self.screen);
            t.text == needle && v.width() > 0.0 && v.height() > 0.0
        })
    }

    /// The centre of the first visible text equal to `needle`.
    pub fn text_center(&self, needle: &str) -> Option<egui::Pos2> {
        self.texts
            .iter()
            .find(|t| t.text == needle && t.clip.intersects(t.rect))
            .map(|t| t.rect.center())
    }

    /// The probed widget called `name`, if the frame reported one.
    pub fn widget(&self, name: &str) -> Option<&ProbedRect> {
        self.widgets.iter().find(|w| w.name == name)
    }
}

/// Where a test frame's [`probe`] calls land. Only present in a
/// `Context` the test hooks built, so a live editor pays one map lookup
/// per probed control and records nothing.
#[derive(Clone, Default)]
struct ProbeSink(Arc<Mutex<Vec<ProbedRect>>>);

fn probe_id() -> egui::Id {
    egui::Id::new("resonance_drums_layout_probe")
}

/// Report a widget's rect, with the clip it was laid out under, to a test
/// frame's probe sink. A no-op outside the test hooks.
pub(crate) fn probe(ui: &egui::Ui, name: impl Into<String>, rect: egui::Rect) {
    let sink = ui.ctx().data(|d| d.get_temp::<ProbeSink>(probe_id()));
    if let Some(sink) = sink {
        sink.0.lock().push(ProbedRect {
            name: name.into(),
            rect,
            clip: ui.clip_rect(),
        });
    }
}

/// Run `add` in its own scope and [`probe`] the rect it took up — for
/// the `plugin_gui_core` widgets, which hand back a value, not a
/// `Response`.
pub(crate) fn probed<R>(
    ui: &mut egui::Ui,
    name: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let out = ui.scope(add);
    probe(ui, name, out.response.rect);
    out.inner
}

/// Test-only: every text shape a frame painted, walked out of egui's own
/// shape tree, with the clip rect each one was painted under.
#[cfg(feature = "test-hooks")]
fn collect_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<ProbedText> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<ProbedText>) {
        match shape {
            egui::Shape::Text(t) => out.push(ProbedText {
                text: t.galley.text().to_string(),
                rect: t.visual_bounding_rect(),
                clip,
            }),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(&s.shape, s.clip_rect, &mut out);
    }
    out
}

/// Test-only: build a fresh editor app from `plugin` and run its
/// `EditorApp::ui` at `size` (width, height), the same call the platform
/// GUI runtime makes each repaint
/// (`wayland-plugin-gui/src/window_thread/paint.rs`). Returns every text
/// the frame painted and every rect the editor [`probe`]d, each with the
/// clip rect it was drawn under, so a test can check what is actually
/// *visible* — `DrumsEditorApp` and `EditorApp::ui` are both private
/// outside this crate, which is why this hook exists rather than a test
/// constructing the app directly (drums-plugin-rework.md §9, K0).
///
/// The editor reads the process's shared kit library, so this first
/// isolates the process from the user's data dir
/// ([`crate::library::isolate_for_tests`]): a test frame never scans,
/// migrates or downloads into the real library.
///
/// Runs two passes on the same `egui::Context` before reading back the
/// shapes — the preset bar's combo boxes are popups, which (like
/// `egui::Modal`) only report their final layout from the second pass
/// onward. The real runtime repaints continuously, so this is what it
/// would actually show once settled.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn test_render_editor_frame(
    plugin: &crate::ResonanceDrums,
    size: (f32, f32),
) -> EditorFrameProbe {
    crate::library::isolate_for_tests();
    let mut editor = TestEditor::new(plugin, crate::library::shared(), size);
    editor.frame(Vec::new());
    editor.frame(Vec::new())
}

/// Test-only: a whole editor driven frame by frame on one
/// `egui::Context`, over a library the test chose — open the Library,
/// click, type, wait for its jobs — so a test can check the overlay's
/// behaviour without a live window.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub struct TestEditor {
    app: app::DrumsEditorApp,
    ctx: egui::Context,
    sink: ProbeSink,
    screen: egui::Rect,
    time: f64,
}

#[cfg(feature = "test-hooks")]
impl TestEditor {
    /// An editor for `plugin` over `library` (build one at a temp root with
    /// `SharedKitLibrary::open`, or use [`crate::library::shared`] after
    /// [`crate::library::isolate_for_tests`]). Its start-up rescan is
    /// finished before this returns.
    pub fn new(
        plugin: &crate::ResonanceDrums,
        library: Arc<crate::library::SharedKitLibrary>,
        size: (f32, f32),
    ) -> Self {
        crate::library::isolate_for_tests();
        // The plugin's `kit_select` resolves against the same library the
        // editor shows (unless something already opened another).
        plugin.params.selection.library.set(library.clone());
        let mut app = app::DrumsEditorApp::new(
            plugin.params.clone(),
            plugin.bridge.clone(),
            library,
            plugin.presets.clone(),
        );
        app.finish_jobs();
        let ctx = egui::Context::default();
        let sink = ProbeSink::default();
        ctx.data_mut(|d| d.insert_temp(probe_id(), sink.clone()));
        Self {
            app,
            ctx,
            sink,
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
            time: 0.0,
        }
    }

    /// Run one frame with `events` as its input and return what it
    /// painted.
    ///
    /// `egui::Modal` only knows it is the topmost modal from its second
    /// frame onward, so input meant for a freshly opened overlay belongs
    /// in the second frame after opening — the same as the live runtime's
    /// first repaint after the click that opened it.
    pub fn frame(&mut self, events: Vec<egui::Event>) -> EditorFrameProbe {
        use plugin_gui_core::EditorApp as _;
        // A 60 Hz clock, so what animates (a scroll, a cell's hit light)
        // moves from frame to frame as it does live.
        self.time += 1.0 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        self.sink.0.lock().clear();
        let app = &mut self.app;
        let output = self.ctx.run_ui(input, |ui| app.ui(ui));
        let widgets = std::mem::take(&mut *self.sink.0.lock());
        EditorFrameProbe {
            screen: self.screen,
            texts: collect_texts(&output.shapes),
            widgets,
            shapes: output.shapes,
        }
    }

    /// Press and release the primary button at `pos`, over two frames.
    pub fn click(&mut self, pos: egui::Pos2) -> EditorFrameProbe {
        let events = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        self.frame(events(true));
        self.frame(events(false))
    }

    /// Select pad `pad`, as clicking its cell does.
    pub fn select_pad(&mut self, pad: usize) {
        self.app.selected_pad = pad;
    }

    /// The selected pad.
    pub fn selected_pad(&self) -> usize {
        self.app.selected_pad
    }

    /// Show the tab called `name` (`"Pads"`, `"Mix"`, `"Setup"`), as the
    /// tab bar does. Panics on another name.
    pub fn show_view(&mut self, name: &str) {
        let i = app::Tab::LABELS
            .iter()
            .position(|l| *l == name)
            .unwrap_or_else(|| panic!("no tab called {name:?}"));
        self.app.tab = app::Tab::ALL[i];
    }

    /// The tab shown: `"Pads"`, `"Mix"` or `"Setup"`.
    pub fn view(&self) -> &'static str {
        app::Tab::LABELS[self.app.tab.index()]
    }

    /// Turn the mouse wheel by `delta` points (negative y scrolls the
    /// content up, revealing what is below) with the pointer at `at`.
    pub fn wheel(&mut self, at: egui::Pos2, delta: egui::Vec2) -> EditorFrameProbe {
        self.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        // Let a smoothed scroll finish.
        for _ in 0..20 {
            self.frame(Vec::new());
        }
        self.frame(Vec::new())
    }

    /// Press at `from`, drag to `to` in steps, release — over several
    /// frames, as a pointer drag does. Returns the last frame.
    pub fn drag(&mut self, from: egui::Pos2, to: egui::Pos2) -> EditorFrameProbe {
        let button = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        self.frame(button(from, true));
        for i in 1..=6 {
            let p = from + (to - from) * (i as f32 / 6.0);
            self.frame(vec![egui::Event::PointerMoved(p)]);
        }
        self.frame(button(to, false));
        self.frame(Vec::new())
    }

    /// Press the primary button at `from` and move to `to` in steps, but
    /// do not let go — a drag still in progress. Returns the last frame.
    pub fn drag_without_release(&mut self, from: egui::Pos2, to: egui::Pos2) -> EditorFrameProbe {
        self.frame(vec![
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let mut last = None;
        for i in 1..=6 {
            let p = from + (to - from) * (i as f32 / 6.0);
            last = Some(self.frame(vec![egui::Event::PointerMoved(p)]));
        }
        last.expect("six frames ran")
    }

    /// Let go of the primary button at `at`.
    pub fn release(&mut self, at: egui::Pos2) -> EditorFrameProbe {
        self.frame(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }])
    }

    /// Two clicks at `pos` in quick succession — after a pause, so a click
    /// just before cannot make it a triple-click.
    pub fn double_click(&mut self, pos: egui::Pos2) -> EditorFrameProbe {
        self.idle(0.7);
        self.click(pos);
        self.click(pos)
    }

    /// Run frames for `secs` of egui time, at 60 Hz.
    pub fn idle(&mut self, secs: f64) -> EditorFrameProbe {
        let frames = (secs * 60.0).ceil().max(1.0) as usize;
        for _ in 1..frames {
            self.frame(Vec::new());
        }
        self.frame(Vec::new())
    }

    /// What the header's `Library…` button does.
    pub fn open_library(&mut self) {
        self.app.open_library();
    }

    pub fn library_open(&self) -> bool {
        self.app.library_panel.open
    }

    /// The overlay's tab: `"Installed"` or `"plok.org"`.
    pub fn library_tab(&self) -> &'static str {
        self.app.library_panel.tab.label()
    }

    /// Switch the overlay's tab, as its segmented control does.
    pub fn show_tab(&mut self, plok: bool) {
        let tab = if plok {
            library_panel::Tab::Plok
        } else {
            library_panel::Tab::Installed
        };
        library_panel::set_tab(&mut self.app, tab);
    }

    /// Wait for the running library job (rescan, import, delete) and
    /// apply its outcome.
    pub fn finish_jobs(&mut self) {
        self.app.finish_jobs();
    }

    /// Type into the Installed tab's search field.
    pub fn search(&mut self, query: &str) {
        self.app.browser.set_query(query);
    }

    /// Toggle the `★ only` chip.
    pub fn set_favorites_only(&mut self, on: bool) {
        self.app.browser.set_favorites_only(on);
    }

    /// The names of the kits in the Installed tab's current view, in order.
    pub fn view_names(&mut self) -> Vec<String> {
        self.app.refresh_rows();
        self.app
            .browser
            .view()
            .iter()
            .map(|&r| self.app.rows.rows[r].entry.name.clone())
            .collect()
    }

    /// Select the kit called `name` in the Installed tab.
    pub fn select(&mut self, name: &str) -> bool {
        self.app.refresh_rows();
        let key = self
            .app
            .rows
            .rows
            .iter()
            .find(|r| r.entry.name == name)
            .map(|r| r.key.clone());
        match key {
            Some(k) => {
                self.app.browser.select(k);
                true
            }
            None => false,
        }
    }

    /// The row key of a delete armed and awaiting confirmation.
    pub fn pending_delete(&self) -> Option<String> {
        self.app.browser.pending_delete().map(str::to_string)
    }

    /// The overlay's last notice, if any.
    pub fn notice(&self) -> Option<String> {
        self.app.browser.notice().map(|n| n.text().to_string())
    }

    /// The kits this editor asked the download worker for.
    pub fn my_downloads(&self) -> Vec<String> {
        let mut v: Vec<String> = self.app.my_downloads.iter().cloned().collect();
        v.sort();
        v
    }

    /// Mark `name` as a download this editor asked for, as Download /
    /// Re-download / Update do before posting it to the worker.
    pub fn add_my_download(&mut self, name: &str) {
        self.app.my_downloads.insert(name.to_string());
    }

    /// The name of the kit selected in the Installed tab.
    pub fn selected_name(&mut self) -> Option<String> {
        self.app.refresh_rows();
        let row = self.app.browser.selected_row()?;
        Some(self.app.rows.rows[row].entry.name.clone())
    }

    /// The running library job's footer label, if one is running.
    pub fn job_running(&self) -> Option<String> {
        self.app
            .jobs
            .busy()
            .then(|| self.app.jobs.label().unwrap_or_default().to_string())
    }

    /// How many library actions wait for the running job.
    pub fn queued_actions(&self) -> usize {
        self.app.queued.len()
    }

    /// Hold the library's job slot with a scan-kind job that runs until
    /// the returned flag is set — standing in for a background rescan the
    /// freshness poll started.
    pub fn hold_job_slot(&mut self) -> Arc<std::sync::atomic::AtomicBool> {
        let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = release.clone();
        let started = self
            .app
            .jobs
            .start(jobs::JobKind::Scan, "scanning…", false, move |_| {
                while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                jobs::JobDone::Rescanned {
                    result: Ok(()),
                    skipped: false,
                    user: false,
                }
            });
        assert!(started, "a job was already running");
        release
    }

    /// What the import dialog does once the user picked `src`.
    pub fn picked_for_import(&mut self, src: std::path::PathBuf) {
        self.app.run_or_queue(app::Queued::Import(src));
    }

    /// What the missing-kit banner's `Locate folder…` does once the user
    /// picked `dir` (the folder is checked on a job: `finish_jobs`).
    pub fn locate_missing(&mut self, dir: std::path::PathBuf) {
        missing_kit::picked(&mut self.app, dir);
    }

    /// A located folder holding another kit, waiting for "Use this kit
    /// anyway?".
    pub fn missing_mismatch(&self) -> Option<std::path::PathBuf> {
        self.app.missing_kit.mismatch.clone()
    }

    /// The banner's last error, if any.
    pub fn missing_error(&self) -> Option<String> {
        self.app.missing_kit.error.clone()
    }
}

/// Test-only: which kit the header's ◀/▶ step from, given `bridge` and
/// the editor's last load request (`(manifest, load generation)`) —
/// `kit_browser` is private outside this crate.
#[doc(hidden)]
pub fn test_kit_path_for_stepping(
    bridge: &crate::KitBridge,
    requested: Option<(std::path::PathBuf, u64)>,
) -> Option<std::path::PathBuf> {
    let requested =
        requested.map(|(path, generation)| kit_browser::RequestedKit { path, generation });
    kit_browser::kit_path_for_stepping(bridge, requested.as_ref())
}
