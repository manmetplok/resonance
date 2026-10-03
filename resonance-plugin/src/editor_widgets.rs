//! Param-typed egui widget helpers for plugin editors — the one place a
//! parameter becomes a control.
//!
//! Binds the pure egui widgets from `plugin_gui_core::widgets` (and
//! plain egui controls) to this crate's parameter types: each helper
//! reads the param, draws the widget, writes the value back if it
//! changed and tells the host about the edit. Feature-gated behind
//! `editor-widgets` so DSP-only consumers don't pull in the GUI stack.
//!
//! # Everything comes from the parameter (ba todo #1281, finding F4)
//!
//! These helpers take *only* the parameter and a caption. Range,
//! default, skew, unit and formatter are read off the param itself, so a
//! control cannot disagree with the parameter it edits. The previous
//! signature took range, default and the value text as arguments, and
//! four editors had silently drifted from their own `params.rs`; the
//! same call sites also hardcoded `logarithmic: false`, which threw away
//! every declared `FloatRange::Skewed`.
//!
//! The controls therefore work in **normalized 0..1 travel** ([`KnobParam`])
//! and let [`FloatParam::plain_at_normalized`] apply the declared curve.
//! So the arc follows the param's own skew, and a range whose `min` is
//! exactly `0.0` (the gate's `key_hpf`) needs no special case.
//!
//! # One knob (code review PUX-11)
//!
//! Every param-bound knob — [`float_knob`] and [`param_knob`] — is the
//! themed knob (`widgets::knob_themed_edit`). The range-mapped classic
//! knob the fleet's other nine editors drew is not reachable from a
//! plugin any more (`tools/arch-invariants` fails a plugin source that
//! calls `widgets::knob(`). [`float_knob`] keeps that knob's 64×76 cell
//! ([`KnobStyle::CAPTIONED`]) and its sub-label, so nothing reflowed.
//!
//! Every knob here:
//!
//! * drags with the fleet's one feel, accumulating travel through the
//!   gesture so a stepped (int/bool) parameter steps on an ordinary drag
//!   (PUX-02);
//! * resets to the declared default on a double-click — not optional,
//!   the binding reads it off the param (PUX-06);
//! * takes a typed value: click the readout under the dial and it
//!   becomes a text field, parsed through
//!   [`crate::param::Param::apply_typed_entry`] (ba todo #1287, finding
//!   F5) — Enter commits, Esc or clicking away cancels, an unreadable
//!   entry is refused and an out-of-range one clamps. [`param_readout`]
//!   gives a slider's value box the same entry;
//! * announces the edit to the host.
//!
//! # Announcing edits (code review PUX-01)
//!
//! A host learns about a change the plugin made itself only when the
//! plugin tells it (`HostHandle::announce_param_change`), and records
//! each announcement as one undoable edit. The bindings here announce
//! through the [`EditAnnouncer`] the editor's factory installed in the
//! egui context ([`install_announcer`]; `editor_host::with_announcer`
//! does it for a whole app):
//!
//! * a continuous control (knob, slider) writes the param on every frame
//!   of the drag — the sound follows the pointer — and announces **once,
//!   when the gesture ends**, and only if it moved the value;
//! * a discrete control (chip, segment, combo pick, checkbox, a typed
//!   entry) writes and announces on the click, when it changed the
//!   value.
//!
//! A control drawn outside the shared kit — an EQ node dragged on the
//! curve, a stepper — announces through [`apply_gesture`] (a continuous
//! gesture) or [`commit_plain`] (a discrete write).

use std::sync::{Arc, Mutex};

use crate::host::EditAnnouncer;
use crate::param::{BoolParam, FloatParam, IntParam, Param};
use plugin_gui_core::egui;
use plugin_gui_core::widgets::{
    self, Chip, ChipStyle, GestureEdit, HSlider, KnobStyle, SegmentedStyle, SliderStyle,
    SliderTone, ThemedKnob,
};

// ---------------------------------------------------------------------------
// Announcing
// ---------------------------------------------------------------------------

fn announcer_id() -> egui::Id {
    egui::Id::new("resonance_edit_announcer")
}

/// Lend `announcer` to every control drawn in `ctx` from now on. The
/// editor host does this before each frame (`editor_host::with_announcer`);
/// a test driving an editor app directly calls it once.
pub fn install_announcer(ctx: &egui::Context, announcer: &EditAnnouncer) {
    ctx.data_mut(|d| d.insert_temp(announcer_id(), announcer.clone()));
}

/// The announcer installed in `ctx`, if any.
pub fn announcer(ctx: &egui::Context) -> Option<EditAnnouncer> {
    ctx.data(|d| d.get_temp::<EditAnnouncer>(announcer_id()))
}

/// Tell the host the user finished an edit of `param` (already set).
/// A no-op where no announcer is installed.
pub fn announce_edit(ctx: &egui::Context, param: &dyn Param) {
    if let Some(a) = announcer(ctx) {
        a.announce(param.id());
    }
}

/// A discrete edit from a control drawn outside this kit: set `param`
/// to `plain` and announce it — unless that leaves it where it was (a
/// re-click of the current segment, a write a stepped param rounds back
/// to its value), in which case nothing is announced.
pub fn commit_plain(ctx: &egui::Context, param: &dyn Param, plain: f64) {
    let before = param.get_plain();
    param.set_plain(plain);
    if param.get_plain() != before {
        announce_edit(ctx, param);
    }
}

/// A user action that writes several params at once (an assistant's
/// "Apply suggestions"): run `apply`, then announce every param in
/// `params` it changed — one edit per param that moved.
pub fn apply_and_announce(ctx: &egui::Context, params: &[&dyn Param], apply: impl FnOnce()) {
    let before: Vec<f64> = params.iter().map(|p| p.get_plain()).collect();
    apply();
    for (p, b) in params.iter().zip(before) {
        if p.get_plain() != b {
            announce_edit(ctx, *p);
        }
    }
}

fn gesture_id(param: &dyn Param) -> egui::Id {
    egui::Id::new(("resonance_param_gesture", param.id()))
}

/// Apply one frame of a continuous control's [`GestureEdit`] to `param`:
/// note the value the gesture started from before the first write,
/// `write` each new value, and announce one edit when the gesture ends
/// having moved the param.
///
/// Public for the editors whose continuous controls are drawn by their
/// own code (an EQ node on the curve, the granular hero view).
pub fn apply_gesture(
    ctx: &egui::Context,
    param: &dyn Param,
    edit: GestureEdit,
    write: impl FnOnce(f32),
) {
    let id = gesture_id(param);
    if edit.began || edit.value.is_some() {
        let open = ctx.data(|d| d.get_temp::<f64>(id)).is_some();
        if !open {
            let start = param.get_plain();
            ctx.data_mut(|d| d.insert_temp(id, start));
        }
    }
    if let Some(v) = edit.value {
        write(v);
    }
    if edit.ended {
        let start = ctx.data_mut(|d| d.remove_temp::<f64>(id));
        if start.is_some_and(|s| s != param.get_plain()) {
            announce_edit(ctx, param);
        }
    }
}

// ---------------------------------------------------------------------------
// Layout probes (tests)
// ---------------------------------------------------------------------------

/// One control rect a param binding reported, with the clip of the `Ui`
/// it was laid out in — what [`headless::HeadlessEditor`] reads back.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct ProbedRect {
    /// The param id (or the name a custom control probed itself as).
    pub name: String,
    pub rect: egui::Rect,
    pub clip: egui::Rect,
}

#[derive(Clone, Default)]
struct ProbeSink(Arc<Mutex<Vec<ProbedRect>>>);

fn probe_id() -> egui::Id {
    egui::Id::new("resonance_editor_probe")
}

/// Report a control's rect to a headless test frame. One context lookup
/// when no test is listening.
pub fn probe(ui: &egui::Ui, name: &str, rect: egui::Rect) {
    if let Some(sink) = ui.ctx().data(|d| d.get_temp::<ProbeSink>(probe_id())) {
        if let Ok(mut v) = sink.0.lock() {
            v.push(ProbedRect {
                name: name.to_string(),
                rect,
                clip: ui.clip_rect(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Travel: what a knob or slider reads and writes
// ---------------------------------------------------------------------------

/// A parameter a knob or a slider can drive, in `0..1` travel.
///
/// Implemented for the three parameter types. A `FloatParam` travels
/// along its own declared curve (its skew); an `IntParam` linearly over
/// its steps, rounding; a `BoolParam` is off below half travel.
pub trait KnobParam {
    /// The parameter, for its id, name, display and typed entry.
    fn param(&self) -> &dyn Param;
    /// Current position, `0..1`.
    fn unit(&self) -> f32;
    /// Where the declared default sits, `0..1`.
    fn default_unit(&self) -> f32;
    /// Move to travel position `unit` (rounded by a stepped param).
    fn set_unit(&self, unit: f32);
    /// Whether the range spans zero, so the arc fills from the centre.
    fn spans_zero(&self) -> bool {
        let p = self.param();
        p.min_plain() < 0.0 && p.max_plain() > 0.0
    }
}

impl KnobParam for FloatParam {
    fn param(&self) -> &dyn Param {
        self
    }
    fn unit(&self) -> f32 {
        self.normalized_value()
    }
    fn default_unit(&self) -> f32 {
        self.default_normalized()
    }
    fn set_unit(&self, unit: f32) {
        self.set_normalized(unit);
    }
}

/// Where integer `value` sits on `param`'s travel.
pub fn int_unit(param: &IntParam, value: i32) -> f32 {
    param.range().normalize(value) as f32
}

/// The integer at travel position `unit` — the inverse of [`int_unit`].
pub fn int_at_unit(param: &IntParam, unit: f32) -> i32 {
    let range = param.range();
    let (min, max) = (range.min(), range.max());
    min + (unit.clamp(0.0, 1.0) * (max - min) as f32).round() as i32
}

impl KnobParam for IntParam {
    fn param(&self) -> &dyn Param {
        self
    }
    fn unit(&self) -> f32 {
        int_unit(self, self.value())
    }
    fn default_unit(&self) -> f32 {
        int_unit(self, self.default_value())
    }
    fn set_unit(&self, unit: f32) {
        self.set_value(int_at_unit(self, unit));
    }
}

impl KnobParam for BoolParam {
    fn param(&self) -> &dyn Param {
        self
    }
    fn unit(&self) -> f32 {
        if self.value() {
            1.0
        } else {
            0.0
        }
    }
    fn default_unit(&self) -> f32 {
        if self.default_value() {
            1.0
        } else {
            0.0
        }
    }
    fn set_unit(&self, unit: f32) {
        self.set_value(unit >= 0.5);
    }
}

// ---------------------------------------------------------------------------
// Typed entry
// ---------------------------------------------------------------------------

/// An in-progress typed entry, parked in egui's temporary memory for the
/// param that is being edited.
#[derive(Clone)]
struct TypedEntry {
    text: String,
    /// Focus is requested on the frame the field first appears, and only
    /// then — re-requesting every frame would make clicking away (the
    /// cancel gesture) impossible.
    focus_requested: bool,
}

/// Stable id for a param's entry state. Namespaced by the param's own
/// string id, so two editors' knobs never share a buffer.
fn entry_id(ui: &egui::Ui, param: &dyn Param) -> egui::Id {
    ui.make_persistent_id(("resonance_param_entry", param.id()))
}

/// Open a typed entry for `param` when `readout` (its value text) was
/// clicked, seeded with what it showed.
fn open_entry_on_click(ui: &egui::Ui, param: &dyn Param, readout: &egui::Response, shown: &str) {
    if readout.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if readout.clicked() {
        let id = entry_id(ui, param);
        ui.data_mut(|d| {
            d.insert_temp(
                id,
                TypedEntry {
                    // Seed with what the control was showing, so a small
                    // correction is an edit rather than a retype.
                    text: shown.to_string(),
                    focus_requested: false,
                },
            )
        });
    }
}

/// The entry in progress for `param`, if one is open.
fn open_entry(ui: &egui::Ui, param: &dyn Param) -> Option<TypedEntry> {
    let id = entry_id(ui, param);
    ui.data(|d| d.get_temp::<TypedEntry>(id))
}

/// Draw `param`'s open entry as a text field filling `size`, centred.
fn draw_entry(ui: &mut egui::Ui, param: &dyn Param, mut entry: TypedEntry, size: egui::Vec2) {
    let id = entry_id(ui, param);
    let (cell, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    probe(ui, param.id(), cell);
    let field = egui::Rect::from_center_size(
        cell.center(),
        egui::vec2(cell.width(), 20.0f32.min(cell.height().max(14.0))),
    );

    let response = ui.put(
        field,
        egui::TextEdit::singleline(&mut entry.text)
            .id(id.with("field"))
            .font(egui::TextStyle::Monospace)
            .horizontal_align(egui::Align::Center)
            .margin(egui::Margin::symmetric(2, 2)),
    );
    if !entry.focus_requested {
        response.request_focus();
        entry.focus_requested = true;
    }

    if response.lost_focus() {
        // Enter commits; anything else that takes focus away — Esc,
        // which egui's TextEdit turns into a focus surrender, or a click
        // elsewhere — cancels and leaves the parameter alone.
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            // One write, whatever the user typed on the way there: the
            // buffer only reaches the param here. An entry the param
            // cannot read is refused; one out of range clamps.
            let before = param.get_plain();
            if param.apply_typed_entry(&entry.text) && param.get_plain() != before {
                announce_edit(ui.ctx(), param);
            }
        }
        ui.data_mut(|d| d.remove::<TypedEntry>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, entry));
    }
}

// ---------------------------------------------------------------------------
// Knobs
// ---------------------------------------------------------------------------

/// One param-bound knob, configured. Build with [`ParamKnob::new`], draw
/// with [`param_knob`].
pub struct ParamKnob<'a> {
    param: &'a dyn KnobParam,
    label: &'a str,
    sub_label: &'a str,
    style: KnobStyle,
    bipolar: Option<bool>,
    warm_from: Option<f32>,
    value_text: Option<String>,
}

impl<'a> ParamKnob<'a> {
    /// A default-styled (52 px lavender) knob over `param`, captioned
    /// `label` — a caption for this cell in this layout (`"Reso"`), never
    /// a fact about the param. Pass `param.name()` when it fits.
    pub fn new(param: &'a dyn KnobParam, label: &'a str) -> Self {
        Self {
            param,
            label,
            sub_label: "",
            style: KnobStyle::LAVENDER,
            bipolar: None,
            warm_from: None,
            value_text: None,
        }
    }

    /// A second caption line (use a style with room for it, such as
    /// [`KnobStyle::CAPTIONED`]).
    pub fn sub_label(mut self, sub_label: &'a str) -> Self {
        self.sub_label = sub_label;
        self
    }

    /// Cell geometry.
    pub fn style(mut self, style: KnobStyle) -> Self {
        self.style = style;
        self
    }

    /// Override the polarity read off the range (a range spanning zero
    /// fills from the centre).
    pub fn bipolar(mut self, bipolar: bool) -> Self {
        self.bipolar = Some(bipolar);
        self
    }

    /// Mark an over-unity zone from this travel position.
    pub fn warm_from(mut self, warm_from: Option<f32>) -> Self {
        self.warm_from = warm_from;
        self
    }

    /// Show this readout instead of the param's own display. Only for a
    /// param whose formatter is not on the param yet.
    pub fn value_text(mut self, text: String) -> Self {
        self.value_text = Some(text);
        self
    }
}

/// Draw a param-bound knob: drag (Shift for fine) to sweep, double-click
/// the dial to reset to the declared default, click the readout to type
/// a value. Announces one edit per gesture. Returns the gesture.
pub fn param_knob(ui: &mut egui::Ui, knob: ParamKnob<'_>) -> GestureEdit {
    let param = knob.param.param();
    let cell = knob.style.cell();
    if let Some(entry) = open_entry(ui, param) {
        draw_entry(ui, param, entry, cell);
        return GestureEdit::default();
    }

    let value_text = knob
        .value_text
        .unwrap_or_else(|| param.display(param.get_plain()));
    let themed = ThemedKnob::new(
        knob.label,
        knob.param.unit(),
        &value_text,
        knob.param.default_unit(),
    )
    .bipolar(knob.bipolar.unwrap_or_else(|| knob.param.spans_zero()))
    .warm_from(knob.warm_from)
    .sub_label(knob.sub_label)
    .style(knob.style);
    // `allocate_ui`, not a scope: it takes its place through the
    // layout's placer, so a row of knobs in `horizontal_wrapped` wraps.
    let shown = ui.allocate_ui(cell, |ui| widgets::knob_themed_edit(ui, &themed));
    let rect = shown.response.rect;
    probe(ui, param.id(), rect);
    let edit = shown.inner;
    apply_gesture(ui.ctx(), param, edit, |u| knob.param.set_unit(u));

    // The readout row doubles as the entry affordance. The strip sits
    // below the dial, so the dial keeps every gesture it had — drag,
    // Shift-drag and double-click-to-reset all still reach the knob.
    let style = knob.style;
    let top = rect.top() + style.diameter + style.value_dy - 2.0;
    let strip = egui::Rect::from_min_max(
        egui::pos2(rect.left(), top),
        egui::pos2(rect.right(), (top + style.value_font + 5.0).min(rect.bottom())),
    );
    let readout = ui.interact(strip, entry_id(ui, param).with("open"), egui::Sense::click());
    open_entry_on_click(ui, param, &readout, &value_text);
    edit
}

/// Rotary knob bound to a `FloatParam`, in the 64×76 captioned cell.
///
/// `label` and `sub_label` are the only caller-supplied inputs: they are
/// captions for a specific cell in a specific layout (`"SC HPF"`,
/// `"early refl."`), not facts about the parameter. Pass
/// `param.name()` when the full name fits.
pub fn float_knob(ui: &mut egui::Ui, param: &FloatParam, label: &str, sub_label: &str) {
    param_knob(
        ui,
        ParamKnob::new(param, label)
            .sub_label(sub_label)
            .style(KnobStyle::CAPTIONED),
    );
}

// ---------------------------------------------------------------------------
// Sliders
// ---------------------------------------------------------------------------

/// One param-bound horizontal slider. Build with [`ParamSlider::new`],
/// draw with [`param_slider`].
pub struct ParamSlider<'a> {
    param: &'a dyn KnobParam,
    width: f32,
    bipolar: Option<bool>,
    tone: SliderTone,
    style: SliderStyle,
}

impl<'a> ParamSlider<'a> {
    /// A default-styled slider over `param`, `width` px wide.
    pub fn new(param: &'a dyn KnobParam, width: f32) -> Self {
        Self {
            param,
            width,
            bipolar: None,
            tone: SliderTone::Accent,
            style: SliderStyle::LAVENDER,
        }
    }

    /// Override the polarity read off the range.
    pub fn bipolar(mut self, bipolar: bool) -> Self {
        self.bipolar = Some(bipolar);
        self
    }

    /// Paint with a non-default accent.
    pub fn tone(mut self, tone: SliderTone) -> Self {
        self.tone = tone;
        self
    }

    /// Non-default geometry.
    pub fn style(mut self, style: SliderStyle) -> Self {
        self.style = style;
        self
    }
}

/// Draw a param-bound slider: relative drag (Shift for fine), arrow keys
/// while focused, double-click to reset to the declared default — the
/// reset is always there, it is read off the param. Announces one edit
/// per gesture. Pair it with [`param_readout`] for typed entry.
pub fn param_slider(ui: &mut egui::Ui, slider: ParamSlider<'_>) -> GestureEdit {
    let param = slider.param.param();
    let s = HSlider::new(slider.width, slider.param.unit())
        .default_unit(slider.param.default_unit())
        .bipolar(slider.bipolar.unwrap_or_else(|| slider.param.spans_zero()))
        .tone(slider.tone)
        .style(slider.style);
    let shown = ui.scope(|ui| widgets::slider_edit(ui, &s));
    probe(ui, param.id(), shown.response.rect);
    let edit = shown.inner;
    apply_gesture(ui.ctx(), param, edit, |u| slider.param.set_unit(u));
    edit
}

/// A param's value text in a `width`-px box, one text row tall (a
/// label's height), that turns into a text field when clicked (typed
/// entry; see the module docs). `text` is the value to show — normally
/// `param.display(param.get_plain())` — and seeds the entry; `caption`
/// (empty for none) is drawn in front of it (`Thr -20.0 dB`) but is not
/// part of what the user edits.
pub fn param_readout(
    ui: &mut egui::Ui,
    param: &dyn Param,
    caption: &str,
    text: &str,
    width: f32,
    font: egui::FontId,
    color: egui::Color32,
) {
    let shown = if caption.is_empty() {
        text.to_string()
    } else {
        format!("{caption} {text}")
    };
    let galley = ui.painter().layout_no_wrap(shown, font, color);
    let height = galley.size().y;
    if let Some(entry) = open_entry(ui, param) {
        draw_entry(ui, param, entry, egui::vec2(width, height));
        return;
    }
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    probe(ui, &format!("{}.value", param.id()), rect);
    if ui.is_rect_visible(rect) {
        ui.painter_at(rect).galley(rect.left_top(), galley, color);
    }
    open_entry_on_click(ui, param, &response, text);
}

/// Horizontal `egui::Slider` bound to a `FloatParam`.
///
/// Moves in normalized travel and formats through the param's own
/// display, which also makes egui's click-to-type entry parse through
/// [`crate::param::Param::parse`]. Announces at the end of a drag or an
/// entry. Prefer [`param_slider`], which is the fleet's slider.
pub fn float_slider(ui: &mut egui::Ui, param: &FloatParam) {
    let mut normalized = param.normalized_value();
    let slider = egui::Slider::new(&mut normalized, 0.0..=1.0)
        .custom_formatter(|n, _| param.display(param.plain_at_normalized(n as f32) as f64))
        .custom_parser(|text| {
            // egui hands us the text the user typed into the value box;
            // route it through the param, then back into travel — the
            // slider itself only ever knows 0..1.
            param
                .parse(text)
                .map(|plain| param.range().normalize(plain as f32) as f64)
        })
        .show_value(true);
    let response = ui.add(slider);
    let edit = GestureEdit {
        value: response.changed().then_some(normalized),
        began: response.drag_started(),
        ended: response.drag_stopped() || (response.changed() && !response.dragged()),
    };
    apply_gesture(ui.ctx(), param, edit, |u| param.set_normalized(u));
}

// ---------------------------------------------------------------------------
// Discrete controls
// ---------------------------------------------------------------------------

/// Checkbox bound to a `BoolParam`. Announces each toggle.
pub fn bool_checkbox(ui: &mut egui::Ui, param: &BoolParam, label: &str) {
    let mut v = param.value();
    let response = ui.checkbox(&mut v, label);
    probe(ui, param.id(), response.rect);
    if response.changed() {
        commit_plain(ui.ctx(), param, if v { 1.0 } else { 0.0 });
    }
}

/// A chip toggle bound to a bool-like param (a `BoolParam`, or any param
/// whose plain value is 0/1): lit while on, a click flips it and
/// announces. With `enabled` false it renders greyed and ignores input.
pub fn param_chip(
    ui: &mut egui::Ui,
    param: &dyn Param,
    label: &str,
    enabled: bool,
    style: ChipStyle,
) -> bool {
    let on = param.get_plain() >= 0.5;
    let chip = Chip::new(label, on).enabled(enabled).style(style);
    let shown = ui.scope(|ui| widgets::chip_styled(ui, &chip));
    probe(ui, param.id(), shown.response.rect);
    if shown.inner {
        commit_plain(ui.ctx(), param, if on { 0.0 } else { 1.0 });
        return true;
    }
    false
}

/// [`param_chip`] for a `BoolParam`, in the default chip style.
pub fn bool_chip(ui: &mut egui::Ui, param: &BoolParam, label: &str) -> bool {
    param_chip(ui, param, label, true, ChipStyle::LAVENDER)
}

/// A one-of-N segmented control bound to an int-like param whose plain
/// values run `min..min + labels.len()`. Re-clicking the current segment
/// writes nothing; a new pick writes and announces.
pub fn param_segmented(
    ui: &mut egui::Ui,
    param: &dyn Param,
    labels: &[&str],
    style: &SegmentedStyle,
) -> bool {
    let min = param.min_plain();
    let current =
        ((param.get_plain() - min).round().max(0.0) as usize).min(labels.len().saturating_sub(1));
    let shown = ui.scope(|ui| widgets::segmented_styled(ui, labels, current, style));
    probe(ui, param.id(), shown.response.rect);
    match shown.inner {
        Some(picked) if picked != current => {
            commit_plain(ui.ctx(), param, min + picked as f64);
            true
        }
        _ => false,
    }
}

/// [`param_segmented`] for a choice `IntParam`, labelled from its own
/// choice table (`IntParam::with_choices`).
pub fn choice_segmented(ui: &mut egui::Ui, param: &IntParam, style: &SegmentedStyle) -> bool {
    let labels = param.choices().unwrap_or(&[]);
    param_segmented(ui, param, labels, style)
}

/// Combo box bound to a choice `IntParam` (one declared with
/// [`IntParam::with_choices`]).
///
/// The labels come off the parameter's own table, so the editor, the
/// host's display and the control API's `choices[]` all read the same
/// words. `width` is the combo's width in px — a layout fact, not a fact
/// about the parameter. A param without a choice table lists every value
/// of its range, each shown by the param's own formatter. A pick
/// announces.
pub fn int_choice(ui: &mut egui::Ui, param: &IntParam, width: f32) {
    let current = param.value();
    let selected = param.display(current as f64);
    let shown = egui::ComboBox::from_id_salt(("resonance_int_choice", param.id()))
        .width(width)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (v, label) in int_choice_options(param) {
                if ui.selectable_label(v == current, label).clicked() {
                    commit_plain(ui.ctx(), param, v as f64);
                }
            }
        });
    probe(ui, param.id(), shown.response.rect);
}

/// The entries [`int_choice`] lists: `(value, label)` for the param's
/// choice table, or — for a param declared without one — for every value
/// `min..=max`, labelled by [`Param::display`]. (Without the fallback a
/// plain `IntParam` drew an empty dropdown.) Only built while the combo
/// is open.
pub fn int_choice_options(param: &IntParam) -> Vec<(i32, String)> {
    let min = param.min_plain() as i32;
    match param.choices() {
        Some(labels) => labels
            .iter()
            .enumerate()
            .map(|(i, label)| (min + i as i32, (*label).to_string()))
            .collect(),
        None => (min..=param.max_plain() as i32)
            .map(|v| (v, param.display(v as f64)))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Headless harness (tests)
// ---------------------------------------------------------------------------

/// An editor app driven headless, one CPU-only egui frame at a time,
/// with a recording [`EditAnnouncer`] installed — for the plugins'
/// `tests/editor_*.rs`: what a frame lays out where, and what a drag,
/// a click or a typed entry announces. Not plugin API.
#[doc(hidden)]
pub mod headless {
    use super::*;
    use plugin_gui_core::EditorApp;

    /// One painted text as it landed on screen, with its clip.
    #[derive(Clone, Debug)]
    pub struct ProbedText {
        pub text: String,
        pub rect: egui::Rect,
        pub clip: egui::Rect,
    }

    /// What a frame painted and where its param controls went.
    #[derive(Clone, Debug)]
    pub struct FrameProbe {
        pub screen: egui::Rect,
        pub texts: Vec<ProbedText>,
        /// Every param-bound control, named by its param id.
        pub widgets: Vec<ProbedRect>,
    }

    impl FrameProbe {
        /// The control probed as `name` (a param id), if it was drawn.
        pub fn widget(&self, name: &str) -> Option<&ProbedRect> {
            self.widgets.iter().find(|w| w.name == name)
        }

        /// Whether a text equal to `needle` was painted.
        pub fn shows(&self, needle: &str) -> bool {
            self.texts.iter().any(|t| t.text == needle)
        }

        /// Every probed control not fully visible — inside its clip and
        /// the window, within `tolerance` px — as `(name, why)`.
        pub fn hidden_widgets(&self, tolerance: f32) -> Vec<(String, String)> {
            self.widgets
                .iter()
                .filter_map(|w| {
                    let visible = w.rect.intersect(w.clip).intersect(self.screen);
                    let why = if !(visible.width() > 0.0 && visible.height() > 0.0) {
                        Some(format!("not visible: {:?} clip {:?}", w.rect, w.clip))
                    } else if !w.clip.expand(tolerance).contains_rect(w.rect) {
                        Some(format!("clipped: {:?} outside clip {:?}", w.rect, w.clip))
                    } else if !self.screen.expand(tolerance).contains_rect(w.rect) {
                        Some(format!("off-window: {:?} outside {:?}", w.rect, self.screen))
                    } else {
                        None
                    };
                    why.map(|why| (w.name.clone(), why))
                })
                .collect()
        }
    }

    /// The editor app plus its context, clock and announcer.
    pub struct HeadlessEditor {
        app: Box<dyn EditorApp>,
        ctx: egui::Context,
        size: egui::Vec2,
        time: f64,
        announcer: EditAnnouncer,
        sink: ProbeSink,
        modifiers: egui::Modifiers,
    }

    impl HeadlessEditor {
        /// Drive `app` at `size` (width, height).
        pub fn new(app: Box<dyn EditorApp>, size: (f32, f32)) -> Self {
            let ctx = egui::Context::default();
            let announcer = EditAnnouncer::recording();
            install_announcer(&ctx, &announcer);
            let sink = ProbeSink::default();
            ctx.data_mut(|d| d.insert_temp(probe_id(), sink.clone()));
            Self {
                app,
                ctx,
                size: egui::vec2(size.0, size.1),
                time: 0.0,
                announcer,
                sink,
                modifiers: egui::Modifiers::NONE,
            }
        }

        /// Lay out at `width`×`height` from the next frame on.
        pub fn set_size(&mut self, width: f32, height: f32) {
            self.size = egui::vec2(width, height);
        }

        /// Hold `modifiers` for the following frames (Shift for fine).
        pub fn set_modifiers(&mut self, modifiers: egui::Modifiers) {
            self.modifiers = modifiers;
        }

        /// The param ids announced so far, in order.
        pub fn announced(&self) -> Vec<String> {
            self.announcer.announced()
        }

        /// The egui context the frames run in.
        pub fn ctx(&self) -> &egui::Context {
            &self.ctx
        }

        /// Run one frame with `events`; 1/60 s of egui time passes.
        pub fn frame(&mut self, events: Vec<egui::Event>) -> FrameProbe {
            self.time += 1.0 / 60.0;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(self.time),
                modifiers: self.modifiers,
                events,
                ..Default::default()
            };
            if let Ok(mut v) = self.sink.0.lock() {
                v.clear();
            }
            install_announcer(&self.ctx, &self.announcer);
            let app = &mut self.app;
            let out = self.ctx.run_ui(input, |ui| app.ui(ui));
            let mut texts = Vec::new();
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
            for s in &out.shapes {
                walk(&s.shape, s.clip_rect, &mut texts);
            }
            let widgets = self.sink.0.lock().map(|v| v.clone()).unwrap_or_default();
            FrameProbe {
                screen,
                texts,
                widgets,
            }
        }

        /// Two frames, so popups and first-frame sizing settle.
        pub fn settled(&mut self) -> FrameProbe {
            self.frame(Vec::new());
            self.frame(Vec::new())
        }

        fn button(&self, pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: self.modifiers,
                },
            ]
        }

        /// Press at `from`, move to `to` in `steps` frames, release.
        /// Returns the frame after the release.
        pub fn drag(&mut self, from: egui::Pos2, to: egui::Pos2, steps: usize) -> FrameProbe {
            self.frame(vec![egui::Event::PointerMoved(from)]);
            self.frame(self.button(from, true));
            let steps = steps.max(1);
            for i in 1..=steps {
                let p = from + (to - from) * (i as f32 / steps as f32);
                self.frame(vec![egui::Event::PointerMoved(p)]);
            }
            self.frame(self.button(to, false));
            self.frame(Vec::new())
        }

        /// A click at `pos`.
        pub fn click(&mut self, pos: egui::Pos2) -> FrameProbe {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.frame(self.button(pos, true));
            self.frame(self.button(pos, false))
        }

        /// Two clicks at `pos` in quick succession, after a pause.
        pub fn double_click(&mut self, pos: egui::Pos2) -> FrameProbe {
            for _ in 0..40 {
                self.frame(Vec::new());
            }
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.frame(self.button(pos, true));
            self.frame(self.button(pos, false));
            self.frame(self.button(pos, true));
            self.frame(self.button(pos, false));
            self.frame(Vec::new())
        }

        /// Replace the focused field's text with `text` and press Enter.
        pub fn type_and_enter(&mut self, text: &str) -> FrameProbe {
            // A field opened by the last click takes focus on its first
            // frame; give it that frame before typing.
            self.frame(Vec::new());
            self.frame(vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }]);
            self.frame(vec![egui::Event::Text(text.to_string())]);
            let enter = |pressed| egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![enter(true), enter(false)]);
            self.frame(Vec::new())
        }
    }
}
