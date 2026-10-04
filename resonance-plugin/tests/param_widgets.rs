//! The shared param bindings (`editor_widgets`) against real headless
//! frames (code review PUX-01/-02/-06/-11): a knob on a stepped param
//! steps on an ordinary drag, every gesture is one announced host edit,
//! a reset and a typed entry announce, a gesture that ends where it
//! started announces nothing, and `editor_host::with_announcer` lends
//! the plugin's announcer to every frame.

use std::sync::Arc;

use plugin_gui_core::{egui, EditorApp};
use resonance_plugin::editor_widgets::headless::HeadlessEditor;
use resonance_plugin::editor_widgets::{self, ParamKnob, ParamSlider};
use resonance_plugin::{BoolParam, EditAnnouncer, FloatParam, FloatRange, IntParam, IntRange};

struct Params {
    on: BoolParam,
    mode: IntParam,
    cutoff: FloatParam,
}

fn params() -> Arc<Params> {
    Arc::new(Params {
        on: BoolParam::new("on", "On", false),
        mode: IntParam::new("mode", "Mode", 0, IntRange::Linear { min: 0, max: 4 }),
        cutoff: FloatParam::new(
            "cutoff",
            "Cutoff",
            1_000.0,
            FloatRange::Skewed {
                min: 20.0,
                max: 20_000.0,
                factor: FloatRange::skew_factor(-2.0),
            },
        ),
    })
}

/// Three knobs in a row and a slider under them.
struct App(Arc<Params>);

impl EditorApp for App {
    fn ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            editor_widgets::param_knob(ui, ParamKnob::new(&self.0.on, "On"));
            editor_widgets::param_knob(ui, ParamKnob::new(&self.0.mode, "Mode"));
            editor_widgets::float_knob(ui, &self.0.cutoff, "Cutoff", "Hz");
        });
        editor_widgets::param_slider(ui, ParamSlider::new(&self.0.cutoff, 200.0));
    }
}

fn editor(p: &Arc<Params>) -> HeadlessEditor {
    HeadlessEditor::new(Box::new(App(p.clone())), (600.0, 400.0))
}

fn dial(editor: &mut HeadlessEditor, id: &str) -> egui::Pos2 {
    let frame = editor.settled();
    let r = frame
        .widgets
        .iter()
        .find(|w| w.name == id)
        .unwrap_or_else(|| panic!("{id} not drawn"))
        .rect;
    egui::pos2(r.center().x, r.top() + 18.0)
}

/// PUX-02: 120 px of drag in 2 px steps moves a bool and an int knob.
/// Before, each frame's 2 px (0.01 of travel) was re-derived from the
/// rounded value and snapped back — a bool needed ~100 px in one frame.
#[test]
fn a_slow_drag_steps_a_bool_and_an_int_knob() {
    let p = params();
    let mut ed = editor(&p);
    let from = dial(&mut ed, "on");
    ed.drag(from, from - egui::vec2(0.0, 120.0), 60);
    assert!(p.on.value(), "120 px up did not switch the bool on");

    let from = dial(&mut ed, "mode");
    ed.drag(from, from - egui::vec2(0.0, 120.0), 60);
    assert!(
        p.mode.value() >= 2,
        "120 px up (0.6 of travel over 0..4) left the int at {}",
        p.mode.value()
    );
    assert_eq!(ed.announced(), ["on", "mode"], "one edit per drag");
}

/// A drag is announced once, at its end — not per frame — and a drag
/// that moves nothing announces nothing.
#[test]
fn a_drag_is_one_edit_and_a_no_op_drag_is_none() {
    let p = params();
    let mut ed = editor(&p);
    let from = dial(&mut ed, "cutoff");
    ed.drag(from, from - egui::vec2(0.0, 30.0), 15);
    assert_eq!(ed.announced(), ["cutoff"]);

    // A drag that moves nothing: down from the bottom of the range.
    p.cutoff.set_value(20.0);
    let from = dial(&mut ed, "cutoff");
    ed.drag(from, from + egui::vec2(0.0, 40.0), 20);
    assert_eq!(p.cutoff.value(), 20.0);
    assert_eq!(ed.announced(), ["cutoff"], "a drag that moved nothing announced an edit");
}

/// The knob travels along the declared skew, not linearly.
#[test]
fn a_float_knob_follows_the_declared_skew() {
    let p = params();
    let mut ed = editor(&p);
    let before = p.cutoff.normalized_value();
    let from = dial(&mut ed, "cutoff");
    ed.drag(from, from - egui::vec2(0.0, 40.0), 20);
    let after = p.cutoff.normalized_value();
    assert!(after > before);
    assert!((p.cutoff.value() - p.cutoff.plain_at_normalized(after)).abs() < 1e-2);
}

/// A double-click resets to the declared default — on the knob and on
/// the slider, which gets its default from the param (PUX-06: the EQ's
/// sliders passed none, so a double-click did nothing).
#[test]
fn double_click_resets_knob_and_slider_and_announces() {
    let p = params();
    p.cutoff.set_value(5_000.0);
    let mut ed = editor(&p);
    let at = dial(&mut ed, "cutoff");
    ed.double_click(at);
    assert_eq!(p.cutoff.value(), 1_000.0);

    p.cutoff.set_value(5_000.0);
    let frame = ed.settled();
    let slider = frame
        .widgets
        .iter()
        .filter(|w| w.name == "cutoff")
        .last()
        .expect("slider drawn")
        .rect;
    ed.double_click(slider.center());
    assert_eq!(p.cutoff.value(), 1_000.0);
    assert_eq!(ed.announced(), ["cutoff", "cutoff"]);
}

/// Click the readout under the dial, type, Enter: the exact value lands
/// and is announced; an unreadable entry changes and announces nothing.
#[test]
fn typed_entry_lands_exactly_and_announces() {
    let p = params();
    let mut ed = editor(&p);
    let frame = ed.settled();
    let r = frame.widgets.iter().find(|w| w.name == "cutoff").unwrap().rect;
    // CAPTIONED: 40 px dial, readout from +3.
    let readout = egui::pos2(r.center().x, r.top() + 46.0);
    ed.click(readout);
    ed.type_and_enter("2500");
    assert_eq!(p.cutoff.value(), 2_500.0);
    assert_eq!(ed.announced(), ["cutoff"]);

    ed.click(readout);
    ed.type_and_enter("loud");
    assert_eq!(p.cutoff.value(), 2_500.0);
    assert_eq!(ed.announced(), ["cutoff"]);
}

/// The factory wrapper installs the plugin's announcer before each
/// frame, so the bindings reach the plugin's host.
#[test]
fn with_announcer_lends_the_plugins_announcer_to_every_frame() {
    struct Bare(Arc<Params>);
    impl EditorApp for Bare {
        fn ui(&mut self, ui: &mut egui::Ui) {
            editor_widgets::bool_checkbox(ui, &self.0.on, "On");
        }
    }
    let p = params();
    let plugin_side = EditAnnouncer::recording();
    let app = resonance_plugin::editor_host::with_announcer(Bare(p.clone()), plugin_side.clone());
    let mut ed = HeadlessEditor::new(Box::new(app), (300.0, 200.0));
    let frame = ed.settled();
    let r = frame.widgets.iter().find(|w| w.name == "on").unwrap().rect;
    ed.click(egui::pos2(r.left() + 8.0, r.center().y));
    assert!(p.on.value());
    assert_eq!(plugin_side.announced(), ["on"], "the plugin's own announcer heard it");
}

/// FU-P2e: a gesture whose widget stops being drawn mid-drag (scrolled
/// out of a list, a tab switched away from it) never delivers `ended`,
/// so the shared ledger (`editor_widgets::apply_gesture`) used to keep
/// its start value around forever. That was not just a leak: the
/// *next* real gesture on the same param saw an entry already there
/// and kept the stale start as its own baseline instead of the value
/// it actually began from — so a second gesture that moved nothing at
/// all could still announce a false edit.
#[test]
fn an_abandoned_gesture_does_not_poison_the_next_ones_baseline() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct HideableApp {
        p: Arc<Params>,
        show: Arc<AtomicBool>,
    }
    impl EditorApp for HideableApp {
        fn ui(&mut self, ui: &mut egui::Ui) {
            if self.show.load(Ordering::Relaxed) {
                editor_widgets::float_knob(ui, &self.p.cutoff, "Cutoff", "Hz");
            }
        }
    }

    let p = params();
    let show = Arc::new(AtomicBool::new(true));
    let mut ed = HeadlessEditor::new(
        Box::new(HideableApp { p: p.clone(), show: show.clone() }),
        (300.0, 200.0),
    );

    let button = |pos: egui::Pos2, pressed: bool| {
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }]
    };

    // Begin a drag and move it partway — a real, in-flight gesture.
    let frame = ed.settled();
    let from = frame.widgets.iter().find(|w| w.name == "cutoff").unwrap().rect.center();
    ed.frame(vec![egui::Event::PointerMoved(from)]);
    ed.frame(button(from, true));
    let moved_to = from - egui::vec2(0.0, 30.0);
    ed.frame(vec![egui::Event::PointerMoved(moved_to)]);
    let abandoned_value = p.cutoff.value();
    assert_ne!(abandoned_value, 1_000.0, "the drag should have moved it off its default");
    assert!(ed.announced().is_empty(), "not ended yet, so nothing announced");

    // The widget stops being drawn — nothing ever sees `ended` for this
    // gesture. The pointer is released while it is gone (so the next
    // click starts clean at the egui level); the ledger entry is what
    // this test is really about.
    show.store(false, Ordering::Relaxed);
    ed.frame(button(moved_to, false));
    for _ in 0..3 {
        ed.frame(Vec::new());
    }
    show.store(true, Ordering::Relaxed);

    // A brand-new gesture begins later and makes no *net* change: it
    // really drags (so `began` genuinely fires), out and back to
    // exactly where it started this time — which is nonzero for the
    // stale baseline (1_000.0) but zero for the correct one
    // (`abandoned_value`).
    let frame = ed.settled();
    let from = frame.widgets.iter().find(|w| w.name == "cutoff").unwrap().rect.center();
    ed.frame(vec![egui::Event::PointerMoved(from)]);
    ed.frame(button(from, true));
    ed.frame(vec![egui::Event::PointerMoved(from - egui::vec2(0.0, 20.0))]);
    ed.frame(vec![egui::Event::PointerMoved(from)]);
    ed.frame(button(from, false));

    assert_eq!(p.cutoff.value(), abandoned_value, "the new gesture should have landed back where it started");
    assert!(
        ed.announced().is_empty(),
        "a gesture that moved nothing must not announce, even after an earlier one was abandoned: {:?}",
        ed.announced()
    );
}
