//! Param-bound controls: every editable control in the editor goes
//! through here, so every edit is **one undoable host edit**.
//!
//! A continuous control (knob, fader) writes its param on every frame of
//! a gesture — the sound follows the pointer — and tells the host once,
//! when the gesture ends (`GestureEdit::ended`), and only if the value
//! moved ([`Gestures`]): a drag back to where it started, a reset of a
//! value already at its default, or a click tell the host nothing, and a
//! run of arrow keys is one edit. A discrete control (a chip, a segment,
//! a combo pick) writes and tells the host on the click, when it picked
//! something new. Both go through [`KitBridge::announce_param_edit`],
//! which is `HostHandle::announce_param_change`: the host records the
//! param's new value as one edit it can undo.
//!
//! The readouts are cached ([`Labels`]): a param's display text is
//! rebuilt only when its value moves, not every frame for every row of
//! the Mix tab's table.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use plugin_gui_core::egui;
use plugin_gui_core::widgets::{self, HSlider, ThemedKnob};
use resonance_plugin::param::Param;
use resonance_plugin::{BoolParam, FloatParam, IntParam};

use crate::choice::ChoiceParam;
use crate::params::DrumParams;
use crate::KitBridge;

use super::{probe, probed, theme};

/// Display text per param, rebuilt only when the param's value moves.
///
/// Keyed by the param's address (the params live in one `Arc` for the
/// editor's lifetime) plus a formatter tag, so one param can be shown two
/// ways (pan as `L 25` on a knob) without the two evicting each other.
#[derive(Default)]
pub(crate) struct Labels {
    map: HashMap<(usize, u8), (u64, String)>,
}

impl Labels {
    /// `param`'s own display text (`Param::display`).
    pub(crate) fn of(&mut self, param: &dyn Param) -> &str {
        self.with(param, 0, |v| param.display(v))
    }

    /// `param`'s value through `format` (tag it: one tag per formatter).
    pub(crate) fn with(
        &mut self,
        param: &dyn Param,
        tag: u8,
        format: impl FnOnce(f64) -> String,
    ) -> &str {
        let key = (param as *const dyn Param as *const () as usize, tag);
        let plain = param.get_plain();
        let bits = plain.to_bits();
        let entry = self.map.entry(key).or_insert_with(|| (!bits, String::new()));
        if entry.0 != bits {
            entry.0 = bits;
            entry.1 = format(plain);
        }
        &entry.1
    }
}

/// The continuous gestures in flight, by param id: the value each param
/// held when its gesture began, so the gesture's end announces **one**
/// undoable edit — and only when the value really moved.
///
/// A drag that ends where it started, a double-click reset of a value
/// already at its default, a click with no drag: each used to announce
/// an edit that changed nothing, an empty undo entry. And a run of arrow
/// keys announced one edit per press.
///
/// It is also what catches the ends a widget never reports
/// ([`Self::stale`]): a table row scrolled out of view mid-drag (the
/// Mix table under a wheel) stops being drawn, so its slider never sees
/// the release; and an editor closed mid-drag never draws again at all
/// (the app flushes what is open when it is dropped).
///
/// Owned by the app and lent to the controls through the egui context
/// ([`gestures`]), so the helpers keep their signatures.
#[derive(Default)]
pub(crate) struct Gestures {
    open: Mutex<HashMap<String, OpenGesture>>,
}

#[derive(Clone, Copy, Debug)]
struct OpenGesture {
    /// The param's plain value when the gesture began.
    start: f64,
    /// When the gesture last moved the value (egui time, seconds).
    touched: f64,
    /// Driven by the pointer (a drag) rather than the keyboard.
    pointer: bool,
}

/// How long after its last step a keyboard gesture whose widget stopped
/// reporting (scrolled away, tab switched) is closed for it — just past
/// the widget's own idle end, so the widget gets to close it first.
const STALE_KEY_SECS: f64 = widgets::KEY_GESTURE_IDLE_SECS + 0.1;

impl Gestures {
    /// Note that `id` is being moved now, from `value` if this is the
    /// gesture's first frame.
    fn touch(&self, id: &str, value: f64, now: f64, pointer: bool) {
        let mut open = self.open.lock();
        let g = open.entry(id.to_string()).or_insert(OpenGesture {
            start: value,
            touched: now,
            pointer,
        });
        g.touched = now;
        g.pointer |= pointer;
    }

    /// Close `id`'s gesture at `value`: whether it moved the param, i.e.
    /// whether there is an edit to announce. `false` for a gesture this
    /// ledger never saw open (or already closed).
    fn finish(&self, id: &str, value: f64) -> bool {
        self.open
            .lock()
            .remove(id)
            .is_some_and(|g| g.start.to_bits() != value.to_bits())
    }

    /// Whether any gesture is open.
    pub(crate) fn any_open(&self) -> bool {
        !self.open.lock().is_empty()
    }

    /// Close the gestures whose widget will not end them: a pointer
    /// gesture once no drag is in progress, a keyboard one once it has
    /// been idle past [`STALE_KEY_SECS`]. Returns their ids and starts.
    pub(crate) fn stale(&self, now: f64, dragging: bool) -> Vec<(String, f64)> {
        let mut open = self.open.lock();
        let ids: Vec<String> = open
            .iter()
            .filter(|(_, g)| {
                if g.pointer {
                    !dragging
                } else {
                    now - g.touched >= STALE_KEY_SECS
                }
            })
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .filter_map(|id| open.remove(&id).map(|g| (id, g.start)))
            .collect()
    }

    /// Close every open gesture (the editor is going away).
    pub(crate) fn drain(&self) -> Vec<(String, f64)> {
        self.open
            .lock()
            .drain()
            .map(|(id, g)| (id, g.start))
            .collect()
    }
}

/// Announce each closed gesture of `closed` whose param moved since it
/// began. For [`Gestures::stale`] and [`Gestures::drain`].
pub(crate) fn announce_closed(bridge: &KitBridge, params: &DrumParams, closed: Vec<(String, f64)>) {
    for (id, start) in closed {
        let moved = (0..crate::params::PARAM_COUNT)
            .map(|i| params.param_at(i))
            .find(|p| p.id() == id)
            .is_some_and(|p| p.get_plain().to_bits() != start.to_bits());
        if moved {
            bridge.announce_param_edit(&id);
        }
    }
}

fn gestures_id() -> egui::Id {
    egui::Id::new("resonance_drums_gestures")
}

/// The ledger the app lent this frame's context — or, for a control
/// drawn without the app (a test drawing the inspector alone), one the
/// context keeps for itself.
pub(crate) fn gestures(ctx: &egui::Context) -> Arc<Gestures> {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Arc<Gestures>>(gestures_id()).clone())
}

/// Lend `gestures` to `ctx` for this frame's controls.
pub(crate) fn lend_gestures(ctx: &egui::Context, gestures: &Arc<Gestures>) {
    ctx.data_mut(|d| d.insert_temp(gestures_id(), gestures.clone()));
}

/// Apply one frame of a continuous control to `param`: note the gesture
/// before the first write (its start value), `write` the new value, and
/// announce one edit when the gesture ends having moved the param.
fn continuous(
    ui: &egui::Ui,
    bridge: &KitBridge,
    param: &dyn Param,
    edit: widgets::GestureEdit,
    write: impl FnOnce(f32),
) {
    let ledger = gestures(ui.ctx());
    if edit.began || edit.value.is_some() {
        let (now, pointer) = ui.input(|i| (i.time, i.pointer.any_down() || i.pointer.any_released()));
        ledger.touch(param.id(), param.get_plain(), now, pointer);
    }
    if let Some(v) = edit.value {
        write(v);
    }
    if edit.ended && ledger.finish(param.id(), param.get_plain()) {
        bridge.announce_param_edit(param.id());
    }
}

/// Pan as the inspector and the Mix table read it: `C`, `L 25`, `R 40`.
pub(crate) fn pan_text(pan: f64) -> String {
    if pan.abs() < 0.005 {
        "C".to_string()
    } else if pan > 0.0 {
        format!("R {:.0}", pan * 100.0)
    } else {
        format!("L {:.0}", -pan * 100.0)
    }
}

/// The `Labels` tag for [`pan_text`].
pub(crate) const PAN_TAG: u8 = 1;

/// A knob over `param`'s own travel (its skew), reading `text`.
pub(crate) fn knob(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    caption: &str,
    param: &FloatParam,
    text: &str,
    bipolar: bool,
) {
    let knob = ThemedKnob::new(caption, param.normalized_value(), text, param.default_normalized())
        .bipolar(bipolar);
    // `allocate_ui`, not a scope: it takes its place through the layout's
    // placer, so a row of knobs in `horizontal_wrapped` wraps.
    let shown = ui.allocate_ui(knob.style.cell(), |ui| widgets::knob_themed_edit(ui, &knob));
    probe(ui, name, shown.response.rect);
    continuous(ui, bridge, param, shown.inner, |v| param.set_normalized(v));
}

/// Width kept for a fader's value readout.
pub(crate) const VALUE_W: f32 = 58.0;

/// A horizontal fader over `param`'s own travel, `width` wide including
/// its value readout on the right. Probed as `name` (the slider) and
/// `name.value` (the readout).
pub(crate) fn fader(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    param: &FloatParam,
    text: &str,
    width: f32,
    bipolar: bool,
) {
    fader_with(ui, bridge, name, param, text, width, bipolar, VALUE_W);
}

/// Width of a table fader's readout (`-12.5 dB`, `L 25`, in a smaller face).
pub(crate) const TABLE_VALUE_W: f32 = 46.0;

/// [`fader`] for a table row: a narrower readout.
pub(crate) fn table_fader(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    param: &FloatParam,
    text: &str,
    width: f32,
    bipolar: bool,
) {
    fader_with(ui, bridge, name, param, text, width, bipolar, TABLE_VALUE_W);
}

#[allow(clippy::too_many_arguments)]
fn fader_with(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    param: &FloatParam,
    text: &str,
    width: f32,
    bipolar: bool,
    value_w: f32,
) {
    let track = (width - value_w - 6.0).max(24.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let slider = HSlider::new(track, param.normalized_value())
            .bipolar(bipolar)
            .default_unit(param.default_normalized());
        let edit = probed(ui, name, |ui| widgets::slider_edit(ui, &slider));
        continuous(ui, bridge, param, edit, |v| param.set_normalized(v));
        value_text(ui, format_args!("{name}.value"), text, value_w);
    });
}

/// A fader over an integer param (polyphony), stepping by whole values.
pub(crate) fn int_fader(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    param: &IntParam,
    text: &str,
    width: f32,
) {
    let (min, max) = (param.min_plain() as i32, param.max_plain() as i32);
    let span = (max - min).max(1) as f32;
    let unit = (param.value() - min) as f32 / span;
    let track = (width - VALUE_W - 6.0).max(24.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let default = (param.default_plain() as f32 - min as f32) / span;
        let slider = HSlider::new(track, unit).default_unit(default);
        let edit = probed(ui, name, |ui| widgets::slider_edit(ui, &slider));
        continuous(ui, bridge, param, edit, |v| {
            param.set_value(min + (v * span).round() as i32)
        });
        value_text(ui, format_args!("{name}.value"), text, VALUE_W);
    });
}

/// A fader's right-hand readout, a fixed width so a column of faders
/// lines up and a value never pushes its slider about.
fn value_text(ui: &mut egui::Ui, name: impl super::ProbeName, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 16.0), egui::Sense::hover());
    let size = if width < VALUE_W { 9.5 } else { 10.5 };
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::monospace(size),
        theme::TEXT_1,
    );
    let pos = egui::pos2(rect.right() - galley.size().x, rect.center().y - galley.size().y * 0.5);
    let shown = egui::Rect::from_min_size(pos, galley.size());
    ui.painter_at(rect).galley(pos, galley, theme::TEXT_1);
    probe(ui, name, shown);
}

/// A chip that toggles a bool param (Mute).
pub(crate) fn toggle(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    label: &str,
    param: &BoolParam,
) {
    let on = param.value();
    if probed(ui, name, |ui| widgets::chip_button(ui, label, on)) {
        param.set_value(!on);
        bridge.announce_param_edit(param.id());
    }
}

/// A segmented control over a choice param. Returns whether the user
/// changed it (after it was written and announced).
pub(crate) fn segmented(
    ui: &mut egui::Ui,
    bridge: &KitBridge,
    name: &str,
    param: &ChoiceParam,
    labels: &[&str],
) -> bool {
    let current = param.value().max(0) as usize;
    match probed(ui, name, |ui| widgets::segmented(ui, labels, current)) {
        Some(picked) if picked != current => {
            param.set_value(picked as i32);
            bridge.announce_param_edit(param.id());
            true
        }
        _ => false,
    }
}

/// A dropdown of `options` (value, text), showing `selected_text`.
/// Returns the value picked, if the user picked one other than `current`.
pub(crate) fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    name: &str,
    width: f32,
    selected_text: &str,
    current: T,
    options: impl IntoIterator<Item = (T, String)>,
) -> Option<T> {
    let mut picked = None;
    // In a box of exactly `width`: a combo grows to fit its text
    // otherwise, and a long kit-port name pushed a table row out of its
    // card. Inside the box `truncate` elides the text instead.
    let width = width.max(24.0);
    let boxed = ui.allocate_ui_with_layout(
        egui::vec2(width, 20.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_max_width(width);
            egui::ComboBox::from_id_salt(name)
                .width(width - ui.spacing().icon_width - ui.spacing().button_padding.x * 2.0)
                .truncate()
                .selected_text(egui::RichText::new(selected_text).color(theme::TEXT_1).size(11.0))
                .show_ui(ui, |ui| {
                    for (value, text) in options {
                        if ui.selectable_label(value == current, text).clicked() && value != current {
                            picked = Some(value);
                        }
                    }
                })
        },
    );
    probe(ui, name, boxed.inner.response.rect);
    picked
}

/// A small caps section heading.
pub(crate) fn heading(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.label(
        egui::RichText::new(text)
            .color(theme::TEXT_3)
            .size(10.5)
            .strong(),
    )
}

/// A row label of fixed width, so the controls after it line up.
pub(crate) fn row_label(ui: &mut egui::Ui, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::hover());
    let galley = ui.painter().layout(
        text.to_string(),
        egui::FontId::proportional(10.5),
        theme::TEXT_2,
        f32::INFINITY,
    );
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter_at(rect).galley(pos, galley, theme::TEXT_2);
}

/// `text` on one line, elided with `…` past `width` (a table cell).
pub(crate) fn one_line(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat::simple(egui::FontId::proportional(size), color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(1.0));
    ui.painter().layout_job(job)
}

/// Paint [`one_line`] text into a fixed `width` × `height` cell, left and
/// vertically centred. Returns the cell.
pub(crate) fn text_cell(
    ui: &mut egui::Ui,
    text: &str,
    color: egui::Color32,
    width: f32,
    height: f32,
    sense: egui::Sense,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), sense);
    let galley = one_line(ui, text, 10.5, color, width - 6.0);
    let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter_at(rect).galley(pos, galley, color);
    response
}

/// The frame every card in the tab bodies is drawn in.
pub(crate) fn card() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::same(12))
}
