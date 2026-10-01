//! Drums plugin editor: an egui UI hosted by the platform GUI runtime.
//!
//! Layout: a chrome bar (Resonance / Drums brand, the preset bar, and the
//! "Open kit file…" / "Download kits…" buttons) sits above a second bar
//! carrying the PADS badge and the KIT preset pill (◀ name ▶). Pads is
//! the editor's only view, so there is no tab strip to switch it with.
//! The central body is a fixed-height KIT + GLOBAL card row at the
//! bottom and, above it, the two-column pad list + pad detail, each
//! scrolling independently so the window can be resized down to its
//! declared minimum without losing either (`factory.rs`,
//! drums-plugin-rework.md §6.1). A status bar (sample rate, buffer size,
//! OUT meter) sits along the bottom edge, and the Download Kits overlay
//! (`download_panel.rs`) draws over everything else when open.
//!
//! Every control comes from `plugin_gui_core::widgets` — the knobs
//! always did, and ba todo #1335 retired the local `editor/widgets/`
//! copies of the chip, the segmented control and the slider. That
//! module's own comment admitted they were "duplicated from
//! resonance-wavetable so the two editors can evolve independently";
//! what they actually did was drift (ba doc #275).

mod app;
mod chrome;
mod download_panel;
mod factory;
mod kit_browser;
mod pad_grid;
mod pad_inspector;
mod theme;

pub use factory::DrumsEditorFactory;

use std::sync::Arc;

use parking_lot::Mutex;
use plugin_gui_core::egui;

// Re-exported so the per-section modules (`pad_inspector`, `kit_browser`)
// can keep their existing `super::reload_kit` import path. The helper
// itself lives at `crate::reload` because the articulation watcher needs
// it in headless builds too.
pub(crate) use crate::reload::reload_kit;

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
    pad_inspector::draw(
        ui,
        &bridge.params,
        bridge,
        &crate::mic_catalog::ManifestMicCatalog::default(),
        selected_pad,
    );
}

/// One painted text as it landed on screen: the string, its visual
/// rect, and the clip rect it was painted under. A label scrolled or
/// squeezed out of view is still in the shape list — `Painter::text`
/// emits it whatever the clip — so "the text was drawn" proves nothing;
/// `rect ∩ clip` is what the user actually sees.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ProbedText {
    pub text: String,
    pub rect: egui::Rect,
    pub clip: egui::Rect,
}

/// One widget rect the editor reported through [`probe`], with the clip
/// rect of the `Ui` it was laid out in.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ProbedRect {
    pub name: String,
    pub rect: egui::Rect,
    pub clip: egui::Rect,
}

/// Everything `test_render_editor_frame` read back from a settled frame.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct EditorFrameProbe {
    pub screen: egui::Rect,
    pub texts: Vec<ProbedText>,
    pub widgets: Vec<ProbedRect>,
}

impl EditorFrameProbe {
    /// Just the strings, for "is it drawn at all" checks.
    pub fn strings(&self) -> Vec<String> {
        self.texts.iter().map(|t| t.text.clone()).collect()
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
/// Runs two passes on the same `egui::Context` before reading back the
/// shapes — the preset bar's combo boxes are popups, which (like
/// `egui::Modal`; see `test_run_download_panel_frame`) only report their
/// final layout from the second pass onward. The real runtime repaints
/// continuously, so this is what it would actually show once settled.
#[doc(hidden)]
pub fn test_render_editor_frame(
    plugin: &crate::ResonanceDrums,
    size: (f32, f32),
) -> EditorFrameProbe {
    use plugin_gui_core::EditorApp as _;

    let mut app = app::DrumsEditorApp::new(
        plugin.params.clone(),
        plugin.bridge.clone(),
        plugin.download_worker.clone(),
        plugin.presets.clone(),
    );
    let ctx = egui::Context::default();
    let sink = ProbeSink::default();
    ctx.data_mut(|d| d.insert_temp(probe_id(), sink.clone()));
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let run = |ctx: &egui::Context, app: &mut app::DrumsEditorApp| {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| app.ui(ui))
    };
    let _settle = run(&ctx, &mut app);
    sink.0.lock().clear();
    let output = run(&ctx, &mut app);
    let widgets = std::mem::take(&mut *sink.0.lock());
    EditorFrameProbe {
        screen,
        texts: collect_texts(&output.shapes),
        widgets,
    }
}

/// Test-only: render the Download Kits overlay in isolation at `size`,
/// starting `open`, with `events` injected as the settled frame's input.
/// Returns whether the panel is still open afterwards and every shape
/// that frame painted, so a test can check both what Esc does and the
/// backdrop's paint order relative to the panel without a live window —
/// `download_panel` is a private module outside this crate
/// (drums-plugin-rework.md §9, K0).
///
/// Runs two passes on the same `egui::Context`: `egui::Modal` only knows
/// it is the topmost modal from the second pass onward (the first pass
/// is what registers it in `ctx`'s memory at all), so a single pass sees
/// an unsettled frame — no backdrop click-catching and Esc not yet wired
/// up — the same way the live runtime's first repaint after opening the
/// panel would. `events` apply to the settled (second) pass, matching
/// when a real user's input could first land on it.
#[doc(hidden)]
pub fn test_run_download_panel_frame(
    plugin: &crate::ResonanceDrums,
    size: (f32, f32),
    events: Vec<egui::Event>,
    open: bool,
) -> (bool, Vec<egui::epaint::ClippedShape>) {
    let mut panel = download_panel::DownloadPanelState::default();
    panel.open = open;
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
    let run = |ctx: &egui::Context, panel: &mut download_panel::DownloadPanelState, events| {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            if panel.open {
                download_panel::draw(ui, panel, &plugin.download_worker);
            }
        })
    };
    let _settle = run(&ctx, &mut panel, Vec::new());
    let output = run(&ctx, &mut panel, events);
    (panel.open, output.shapes)
}
