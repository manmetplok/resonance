//! Quantize panel for the MIDI editor chrome (todo #392, doc #163).
//!
//! A compact toolbar row hosting the timing-quantize controls: a grid
//! division picker, Strength and Swing sliders, a Start-only / Start+Length
//! mode toggle, "Quantize note ends" and "Iterative/soft" checkboxes, and
//! an Apply button. The controls are bound to
//! [`MidiQuantizePanelState`](crate::state::MidiQuantizePanelState); Apply
//! reads that state to dispatch the bulk
//! [`MidiEditorMessage::Quantize`](crate::message::MidiEditorMessage),
//! which the existing handler (#391) applies to the active note selection —
//! or the whole clip when nothing is selected.
//!
//! The grid pick_list options are a constant set, so they're built once and
//! shared (a `&'static` slice) rather than re-allocated every frame, per the
//! view-layer performance rules.

use std::sync::OnceLock;

use iced::widget::{button, checkbox, column, container, pick_list, row, slider, text, Space};
use iced::{alignment, Element, Length};

use resonance_audio::quantize::QuantizeMode;

use crate::message::{Message, MidiEditorMessage};
use crate::state::{GridChoice, MidiQuantizePanelState};
use crate::theme;

/// The grid pick_list option set, built once. Constant for the life of the
/// process, so we hand the pick_list a `&'static [GridChoice]` instead of
/// allocating a fresh `Vec` per paint.
fn grid_options() -> &'static [GridChoice] {
    static OPTS: OnceLock<Vec<GridChoice>> = OnceLock::new();
    OPTS.get_or_init(|| GridChoice::ALL.to_vec()).as_slice()
}

/// Build the Quantize panel toolbar for the piano-roll editor.
///
/// * `state` — current panel settings (drives every control's value).
/// * `selected` — number of currently selected notes, shown as the scope
///   hint so the user can see whether Apply hits the selection or the whole
///   clip.
pub(crate) fn view(state: &MidiQuantizePanelState, selected: usize) -> Element<'_, Message> {
    // -- Grid division picker (static, cached options) --
    let grid_picker = pick_list(grid_options(), Some(state.grid), |grid| {
        Message::MidiEditor(MidiEditorMessage::SetQuantizeGrid(grid))
    })
    .text_size(12)
    .padding([3, 6]);

    // -- Strength + Swing sliders (0..=1, shown as a percentage) --
    let strength = labeled_slider("Strength", state.strength, |v| {
        Message::MidiEditor(MidiEditorMessage::SetQuantizeStrength(v))
    });
    let swing = labeled_slider("Swing", state.swing, |v| {
        Message::MidiEditor(MidiEditorMessage::SetQuantizeSwing(v))
    });

    // -- Start-only / Start+Length mode toggle --
    let start_active = matches!(state.mode, QuantizeMode::StartOnly);
    let len_active = matches!(state.mode, QuantizeMode::StartAndLength);
    let start_btn = button(text("Start").size(11))
        .padding([3, 8])
        .on_press(Message::MidiEditor(MidiEditorMessage::SetQuantizeMode(
            QuantizeMode::StartOnly,
        )))
        .style(move |_t, status| theme::toggle_button_style(start_active, theme::ACCENT, true, status));
    let len_btn = button(text("Start+Len").size(11))
        .padding([3, 8])
        .on_press(Message::MidiEditor(MidiEditorMessage::SetQuantizeMode(
            QuantizeMode::StartAndLength,
        )))
        .style(move |_t, status| theme::toggle_button_style(len_active, theme::ACCENT, true, status));
    let mode_toggle = row![
        text("Mode").size(11).color(theme::TEXT_3),
        start_btn,
        len_btn,
    ]
    .spacing(4)
    .align_y(alignment::Vertical::Center);

    // -- Boolean options --
    let ends_check = checkbox(state.quantize_ends)
        .label("Quantize note ends")
        .text_size(11)
        .size(15)
        .on_toggle(|on| Message::MidiEditor(MidiEditorMessage::SetQuantizeEnds(on)));
    let soft_check = checkbox(state.iterative)
        .label("Iterative/soft")
        .text_size(11)
        .size(15)
        .on_toggle(|on| Message::MidiEditor(MidiEditorMessage::SetQuantizeIterative(on)));

    // -- Apply: build the bulk quantize from the current panel state --
    let apply_msg = Message::MidiEditor(MidiEditorMessage::Quantize {
        grid: state.grid.division(),
        strength: state.strength,
        swing: state.swing,
        mode: state.mode,
        quantize_ends: state.quantize_ends,
        iterative: state.iterative,
    });
    let apply_btn = button(text("Quantize").size(11).color(theme::PANEL_DARK))
        .padding([4, 12])
        .on_press(apply_msg)
        .style(|_t, status| theme::primary_button_style(status));

    // Scope hint: which notes Apply will affect.
    let scope = if selected == 0 {
        "whole clip".to_string()
    } else {
        format!("{} selected", selected)
    };
    let scope_hint = text(scope)
        .size(10)
        .color(theme::TEXT_3)
        .font(theme::MONO_FONT);

    let header = row![
        theme::icon(theme::fa::SLIDERS).size(12).color(theme::ACCENT),
        text("Quantize").size(12).color(theme::ACCENT),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let controls = row![
        header,
        Space::new().width(Length::Fixed(6.0)),
        labeled("Grid", grid_picker.into()),
        strength,
        swing,
        mode_toggle,
        ends_check,
        soft_check,
        Space::new().width(Length::Fill),
        scope_hint,
        apply_btn,
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .padding([4, 8]);

    container(controls)
        .width(Length::Fill)
        .style(theme::panel_outlined)
        .into()
}

/// A short caption stacked above a control, keeping the toolbar compact.
fn labeled<'a>(caption: &'static str, control: Element<'a, Message>) -> Element<'a, Message> {
    column![
        text(caption).size(9).color(theme::TEXT_3),
        control,
    ]
    .spacing(2)
    .into()
}

/// A captioned percentage slider over `0.0..=1.0`. The caption shows the
/// current value as a whole-number percent (e.g. "Strength 100%").
fn labeled_slider<'a>(
    caption: &str,
    value: f32,
    on_change: impl Fn(f32) -> Message + 'a,
) -> Element<'a, Message> {
    let pct = (value.clamp(0.0, 1.0) * 100.0).round() as u32;
    let cap = text(format!("{caption} {pct}%"))
        .size(9)
        .color(theme::TEXT_3);
    let s = slider(0.0..=1.0f32, value, on_change)
        .step(0.01f32)
        .width(Length::Fixed(96.0));
    column![cap, s].spacing(2).into()
}
