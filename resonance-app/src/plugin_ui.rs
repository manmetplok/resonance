//! The host's generic parameter panel for plugins without a custom UI:
//! the `UiParam` rows the mixer inspector draws, the event they emit, and
//! the display rule `param_display`.
//!
//! This is host-side iced code. It used to live in `resonance-plugin` behind
//! a `ui` feature, which gave every plugin author an optional iced dependency
//! in their SDK for nothing; the app is its only consumer (ARCH-08).

use iced::widget::{button, column, row, slider, text};
use iced::{Element, Font, Length};

// -- Data types ---------------------------------------------------------------

/// Parameter data passed from the host to plugin views.
#[derive(Debug, Clone, Default)]
pub struct UiParam {
    pub id: u32,
    pub name: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub current_value: f64,
    /// The plugin's own rendering of `current_value` — `"40 %"`,
    /// `"-6.0 dB"`, `"Low-pass"` (ba todo #1290, finding X8).
    ///
    /// The panel prints this instead of the raw number whenever it is
    /// non-empty. The plugins declare it — 100+ `with_value_to_string`
    /// call sites across the fleet — and until this field existed the
    /// generic panel threw all of it away and showed `{:.2}`, so a
    /// filter type read `2.00` and a mix read `0.40`.
    pub text: String,
    /// True when the parameter moves in whole numbers, so the slider
    /// steps by one instead of sliding through values the plugin will
    /// only round away.
    pub stepped: bool,
    /// CLAP `IS_READONLY`: an output only the plugin writes (a load
    /// progress, a meter). Drawn as its value alone, with no slider.
    pub read_only: bool,
    /// The MIDI Learn badge for this parameter: the control it is bound
    /// to (`CC74`), `LEARN` while learn is armed on it, else `None`.
    pub midi: Option<String>,
    /// Learn is armed on this parameter: the row is outlined.
    pub learning: bool,
}

/// Events emitted by plugin UIs, mapped to host messages by the app.
#[derive(Debug, Clone)]
pub enum PluginUiEvent {
    SetParam(u32, f64),
    /// Right-click on a parameter's row, at a window position: the host
    /// opens its MIDI menu there.
    ParamMenu(u32, iced::Point),
}

// -- Theme constants ----------------------------------------------------------

pub const TEXT: iced::Color = iced::Color::from_rgb(
    0xe0 as f32 / 255.0,
    0xe0 as f32 / 255.0,
    0xe0 as f32 / 255.0,
);

pub const TEXT_DIM: iced::Color = iced::Color::from_rgb(
    0x80 as f32 / 255.0,
    0x80 as f32 / 255.0,
    0x80 as f32 / 255.0,
);

pub fn small_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => iced::Color::from_rgb(0.22, 0.22, 0.22),
        button::Status::Pressed => iced::Color::from_rgb(0.15, 0.15, 0.15),
        _ => iced::Color::TRANSPARENT,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT,
        border: iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: 2.0.into(),
        },
        ..Default::default()
    }
}

/// What the generic panel prints beside a parameter's slider.
///
/// The plugin's own rendering when it has one — `"40 %"`, `"1/8D"`,
/// `"Low-pass"` — and the bare number only when it does not (ba todo
/// #1290, finding X8). Pure, so `tests/generic_panel.rs` can hold the
/// rule without a GUI: this is the whole of the panel's decision, and
/// the rest is layout.
pub fn param_display(param: &UiParam) -> String {
    if param.text.trim().is_empty() {
        format!("{:.2}", param.current_value)
    } else {
        param.text.clone()
    }
}

// -- Shared widgets -----------------------------------------------------------

/// Render a generic param slider view for any plugin.
pub fn view_generic_params<'a>(params: &[UiParam]) -> Element<'a, PluginUiEvent> {
    let mut controls = column![].spacing(2);
    for param in params {
        let param_id = param.id;
        let range = param.min_value..=param.max_value;
        let param_slider = slider(range, param.current_value, move |v| {
            PluginUiEvent::SetParam(param_id, v)
        })
        .width(Length::Fill)
        // A stepped parameter has no values between its steps: sliding
        // through 2.37 of a filter type only sends the plugin numbers it
        // rounds away.
        .step(if param.stepped { 1.0 } else { 0.001 });

        let param_label = text(param.name.clone()).size(8).color(TEXT_DIM);
        // What the PLUGIN calls this value, falling back to the bare
        // number only when it declares no formatting: "Low-pass" is what
        // the parameter means, "2.00" is the enum index behind it.
        let param_value_text = text(param_display(param))
            .size(8)
            .font(Font::MONOSPACE)
            .color(TEXT_DIM);

        let mut header = row![
            param_label,
            iced::widget::Space::new().width(Length::Fill),
            param_value_text
        ]
        .spacing(2);
        if let Some(badge) = &param.midi {
            let color = if param.learning { crate::theme::ACCENT_SOFT } else { TEXT_DIM };
            header = header.push(text(badge.clone()).size(9).font(Font::MONOSPACE).color(color));
        }
        // An output has nothing to drag: a slider would only send writes
        // the plugin ignores — nor anything to learn.
        if param.read_only {
            controls = controls.push(column![header].spacing(1));
            continue;
        }
        let learning = param.learning;
        let param_row = iced::widget::container(column![header, param_slider].spacing(1)).style(
            move |_theme| iced::widget::container::Style {
                border: iced::Border {
                    color: if learning {
                        crate::theme::ACCENT
                    } else {
                        iced::Color::TRANSPARENT
                    },
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..Default::default()
            },
        );
        controls = controls.push(crate::view::context_area::context_area(param_row, move |p| {
            PluginUiEvent::ParamMenu(param_id, p)
        }));
    }
    controls.into()
}
