//! Param-typed egui widget helpers for plugin editors.
//!
//! Binds the pure egui widgets from `plugin_gui_core::widgets` (and
//! plain egui controls) to this crate's parameter types: each helper
//! reads the param, draws the widget, and writes the value back if it
//! changed. Feature-gated behind `editor-widgets` so DSP-only consumers
//! don't pull in the GUI stack.
//!
//! # Everything comes from the parameter (ba todo #1281, finding F4)
//!
//! These helpers take *only* the `FloatParam` and the two caption
//! strings. Range, default, skew, unit and formatter are read off the
//! param itself, so a control cannot disagree with the parameter it
//! edits. The previous signature took range, default and the value text
//! as arguments, and four editors had silently drifted from their own
//! `params.rs` (reverb 4 knobs, compressor 4, amp 1, IR 1); the same
//! call sites also hardcoded `logarithmic: false`, which threw away
//! every declared `FloatRange::Skewed`.
//!
//! The controls therefore work in **normalized 0..1 travel** and let
//! [`FloatParam::plain_at_normalized`] apply the declared curve, rather
//! than asking the widget for a logarithmic drag. Two consequences worth
//! knowing:
//!
//! * the arc/groove follows the param's own skew, so 50 % travel is
//!   whatever the parameter says 50 % is;
//! * a range whose `min` is exactly `0.0` (the gate's `key_hpf`) works
//!   without a special case — the power-law mapping has no logarithm to
//!   clamp away from zero, unlike `widgets::knob`'s `logarithmic: true`
//!   path, which silently raises such a minimum to 0.001.
//!
//! # Typing an exact value (ba todo #1287, finding F5)
//!
//! Click a knob's value readout — the number under the dial — and it
//! becomes a text field; the slider's value box does the same through
//! egui's own click-to-type. Both parse through
//! [`crate::param::Param::apply_typed_entry`], so `-18 dB` reaches the
//! compressor's threshold exactly. Enter commits, Esc (or clicking away)
//! cancels, an entry the parameter cannot read is refused, and one that
//! is out of range clamps to the declared bounds. Before this, `parse`
//! and the bridge's `text_to_value` were fully implemented and unit
//! tested with nothing in the fleet calling them.

use crate::param::{BoolParam, FloatParam, IntParam, Param};
use plugin_gui_core::egui;

/// The cell `plugin_gui_core::widgets::knob` allocates: 64 px wide,
/// a 40 px dial plus 36 px of label and value rows.
///
/// Mirrored here so the text field that replaces a knob during entry
/// occupies exactly the same space and the row does not jump. A
/// `debug_assert` in [`float_knob`] catches the shared widget changing
/// its geometry out from under this.
const KNOB_CELL: egui::Vec2 = egui::vec2(64.0, 76.0);

/// Height of the click target over a knob's value readout, measured up
/// from the bottom of the cell. Covers the readout row in both of the
/// shared widget's layouts (with and without a sub-label).
const READOUT_STRIP_H: f32 = 18.0;

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
fn entry_id(ui: &egui::Ui, param: &FloatParam) -> egui::Id {
    ui.make_persistent_id(("resonance_param_entry", param.id()))
}

/// Horizontal slider bound to a `FloatParam`.
///
/// Moves in normalized travel and formats through the param's own
/// display, which also makes egui's click-to-type entry parse through
/// [`crate::param::Param::parse`].
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
    if ui.add(slider).changed() {
        param.set_normalized(normalized);
    }
}

/// Rotary knob bound to a `FloatParam`.
///
/// `label` and `sub_label` are the only caller-supplied inputs: they are
/// captions for a specific cell in a specific layout (`"SC HPF"`,
/// `"early refl."`), not facts about the parameter. Pass
/// `param.name()` when the full name fits.
///
/// Drag to sweep, Shift to fine-adjust, double-click the dial to reset to
/// the declared default, click the readout to type an exact value.
pub fn float_knob(ui: &mut egui::Ui, param: &FloatParam, label: &str, sub_label: &str) {
    let id = entry_id(ui, param);
    if let Some(entry) = ui.data(|d| d.get_temp::<TypedEntry>(id)) {
        draw_entry(ui, param, id, entry);
        return;
    }

    let mut normalized = param.normalized_value();
    let value_text = param.display(param.value() as f64);
    let drawn = ui.scope(|ui| {
        plugin_gui_core::widgets::knob(
            ui,
            &mut normalized,
            0.0..=1.0,
            param.default_normalized(),
            label,
            sub_label,
            &value_text,
            // The skew already lives in the normalized mapping; asking
            // the widget for a second, logarithmic curve would apply it
            // twice.
            false,
        )
    });
    if drawn.inner {
        param.set_normalized(normalized);
    }

    let cell = drawn.response.rect;
    debug_assert_eq!(
        cell.size(),
        KNOB_CELL,
        "the shared knob's cell size changed; KNOB_CELL must follow it"
    );

    // The readout row doubles as the entry affordance. The strip sits
    // below the dial, so the dial keeps every gesture it had — drag,
    // Shift-drag and double-click-to-reset all still reach the knob.
    let strip = egui::Rect::from_min_max(
        egui::pos2(cell.left(), cell.bottom() - READOUT_STRIP_H),
        cell.max,
    );
    let readout = ui.interact(strip, id.with("open"), egui::Sense::click());
    if readout.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    if readout.clicked() {
        ui.data_mut(|d| {
            d.insert_temp(
                id,
                TypedEntry {
                    // Seed with what the knob was showing, so a small
                    // correction is an edit rather than a retype.
                    text: value_text.clone(),
                    focus_requested: false,
                },
            )
        });
    }
}

/// The knob cell while a value is being typed into it.
fn draw_entry(ui: &mut egui::Ui, param: &FloatParam, id: egui::Id, mut entry: TypedEntry) {
    let (cell, _) = ui.allocate_exact_size(KNOB_CELL, egui::Sense::hover());
    let field = egui::Rect::from_center_size(cell.center(), egui::vec2(cell.width(), 20.0));

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
            param.apply_typed_entry(&entry.text);
        }
        ui.data_mut(|d| d.remove::<TypedEntry>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, entry));
    }
}

/// Checkbox bound to a `BoolParam`.
pub fn bool_checkbox(ui: &mut egui::Ui, param: &BoolParam, label: &str) {
    let mut v = param.value();
    if ui.checkbox(&mut v, label).changed() {
        param.set_value(v);
    }
}

/// Combo box bound to a choice `IntParam` (one declared with
/// [`IntParam::with_choices`]).
///
/// The labels come off the parameter's own table, so the editor, the
/// host's display and the control API's `choices[]` all read the same
/// words. `width` is the combo's width in px — a layout fact, not a fact
/// about the parameter. A param without a choice table shows its number.
pub fn int_choice(ui: &mut egui::Ui, param: &IntParam, width: f32) {
    let min = param.min_plain() as i32;
    let current = param.value();
    let labels = param.choices().unwrap_or(&[]);
    let selected = param.display(current as f64);
    egui::ComboBox::from_id_salt(("resonance_int_choice", param.id()))
        .width(width)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                let v = min + i as i32;
                if ui.selectable_label(v == current, *label).clicked() {
                    param.set_value(v);
                }
            }
        });
}
