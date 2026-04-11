//! Top transport bar: view tabs, transport buttons, loop toggle, BPM /
//! time-sig / metronome panel, settings button. Rendered above everything
//! else, not specific to the current view mode.
use iced::widget::text::LineHeight;
use iced::widget::{button, column, container, mouse_area, row, text, text_input, Space};
use iced::{alignment, Element, Font, Length};
use resonance_audio::types::TempoMap;

use crate::message::*;
use crate::state::ViewMode;
use crate::theme::{self, fa};
use crate::Resonance;

pub(crate) fn view_transport(r: &Resonance) -> Element<'_, Message> {
    let tempo = TempoMap {
        bpm: r.transport.bpm,
        numerator: r.transport.time_sig_num,
        denominator: r.transport.time_sig_den,
        metronome_enabled: r.transport.metronome_enabled,
    };
    let bar_beat_str = tempo.format_position(r.transport.playhead, r.sample_rate);
    let time_str = tempo.format_time(r.transport.playhead, r.sample_rate);

    // ---- Transport buttons (uniform size/padding/style) -----------------
    const TRANSPORT_ICON_SIZE: u16 = 16;
    let button_pad = iced::Padding::from([6, 10]);

    let skip_back = transport_icon_btn(fa::BACKWARD_STEP, Message::Transport(TransportMessage::SkipBack), button_pad);
    let stop_btn = transport_icon_btn(fa::STOP, Message::Transport(TransportMessage::Stop), button_pad);
    let skip_fwd = transport_icon_btn(fa::FORWARD_STEP, Message::Transport(TransportMessage::SkipForward), button_pad);

    let play_pause: Element<'_, Message> = if r.transport.playing {
        button(
            theme::icon(fa::PAUSE)
                .size(TRANSPORT_ICON_SIZE)
                .color(theme::TEXT),
        )
        .on_press(Message::Transport(TransportMessage::Pause))
        .padding(button_pad)
        .style(|_theme, status| theme::transport_button_style(status))
        .into()
    } else {
        button(
            theme::icon(fa::PLAY)
                .size(TRANSPORT_ICON_SIZE)
                .color(theme::ACCENT),
        )
        .on_press(Message::Transport(TransportMessage::Play))
        .padding(button_pad)
        .style(|_theme, status| theme::transport_button_style(status))
        .into()
    };

    // Record button: grayed out and unclickable when no track is armed.
    let any_armed = r.registry.tracks.iter().any(|t| t.record_armed);
    let rec_color = if any_armed {
        theme::RECORD_RED
    } else {
        theme::TEXT_DIM
    };
    let mut rec_btn = button(
        theme::icon(fa::CIRCLE)
            .size(TRANSPORT_ICON_SIZE)
            .color(rec_color),
    )
    .padding(button_pad)
    .style(move |_theme, status| {
        if any_armed {
            theme::record_armed_button_style(status)
        } else {
            theme::transport_button_style(status)
        }
    });
    if any_armed {
        rec_btn = rec_btn.on_press(Message::Transport(TransportMessage::Record));
    }

    let timing_panel = timing_panel(r, bar_beat_str, time_str);
    let loop_btn = loop_button(r, button_pad, TRANSPORT_ICON_SIZE);

    // Settings icon (Font Awesome bars)
    let settings_btn = button(
        theme::icon(fa::BARS)
            .size(TRANSPORT_ICON_SIZE)
            .color(theme::TEXT),
    )
    .on_press(Message::Ui(UiMessage::OpenSettings))
    .padding(button_pad)
    .style(|_theme, status| theme::transport_button_style(status));

    // View mode tabs
    let arrange_tab = tab_button("Arrange", ViewMode::Arrange, r.view_mode);
    let mixer_tab = tab_button("Mixer", ViewMode::Mixer, r.view_mode);
    let compose_tab = tab_button("Compose", ViewMode::Compose, r.view_mode);

    let transport_row = row![
        Space::with_width(10),
        arrange_tab,
        mixer_tab,
        compose_tab,
        Space::with_width(10),
        skip_back,
        stop_btn,
        play_pause,
        rec_btn,
        skip_fwd,
        Space::with_width(8),
        loop_btn,
        Space::with_width(16),
        timing_panel,
        Space::with_width(Length::Fill),
        settings_btn,
        Space::with_width(10),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center)
    .height(56);

    container(transport_row)
        .width(Length::Fill)
        .style(theme::panel_bg)
        .into()
}

fn transport_icon_btn<'a>(
    glyph: char,
    on_press: Message,
    pad: iced::Padding,
) -> iced::widget::Button<'a, Message> {
    button(theme::icon(glyph).size(16).color(theme::TEXT))
        .on_press(on_press)
        .padding(pad)
        .style(|_theme, status| theme::transport_button_style(status))
}

fn tab_button<'a>(
    label: &'a str,
    mode: ViewMode,
    current: ViewMode,
) -> iced::widget::Button<'a, Message> {
    let active = current == mode;
    button(text(label).size(12))
        .on_press(Message::Ui(UiMessage::SwitchView(mode)))
        .style(move |_theme, status| theme::tab_button_style(active, status))
        .padding([4, 8])
}

/// Central timing panel: position (bars.beats + mm:ss), BPM, time sig,
/// metronome + precount. Every sub-block is a two-row column with
/// identical structure so all values share the same baseline. Every text
/// element uses `line_height(1.0)` so its layout box equals its font-size;
/// the icon font and monospace font have wildly different hhea line
/// metrics, and centering within a fixed-height row otherwise pushes them
/// to different vertical positions.
fn timing_panel<'a>(
    r: &Resonance,
    bar_beat_str: String,
    time_str: String,
) -> iced::widget::Container<'a, Message> {
    const VALUE_SIZE: u16 = 18;
    const LABEL_SIZE: u16 = 9;
    const BLOCK_HEIGHT: f32 = 40.0;
    const VALUE_ROW_HEIGHT: f32 = 22.0;
    const LABEL_ROW_HEIGHT: f32 = 12.0;

    let tight = LineHeight::Relative(1.0);

    fn value_cell<'a>(
        content: impl Into<Element<'a, Message>>,
    ) -> iced::widget::Container<'a, Message> {
        container(content)
            .width(Length::Fill)
            .height(VALUE_ROW_HEIGHT)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
    }

    fn label_cell<'a>(
        content: impl Into<Element<'a, Message>>,
    ) -> iced::widget::Container<'a, Message> {
        container(content)
            .width(Length::Fill)
            .height(LABEL_ROW_HEIGHT)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
    }

    let position_block = column![
        value_cell(
            text(bar_beat_str)
                .size(VALUE_SIZE)
                .line_height(tight)
                .font(Font::MONOSPACE)
                .color(theme::ACCENT),
        ),
        label_cell(
            text(time_str)
                .size(LABEL_SIZE + 1)
                .line_height(tight)
                .font(Font::MONOSPACE)
                .color(theme::TEXT_DIM),
        ),
    ]
    .width(112)
    .align_x(alignment::Horizontal::Center);

    let bpm_field = text_input("120", &r.transport.bpm_input)
        .on_input(|s| Message::Transport(TransportMessage::SetBpmText(s)))
        .on_submit(Message::Transport(TransportMessage::CommitBpm))
        .width(52)
        .size(VALUE_SIZE)
        .font(Font::MONOSPACE)
        .align_x(alignment::Horizontal::Center)
        .padding(0)
        .style(theme::borderless_text_input_style);
    let bpm_block = column![
        value_cell(bpm_field),
        label_cell(
            text("BPM")
                .size(LABEL_SIZE)
                .line_height(tight)
                .color(theme::TEXT_DIM),
        ),
    ]
    .width(60)
    .align_x(alignment::Horizontal::Center);

    let time_sig_str = format!("{}/{}", r.transport.time_sig_num, r.transport.time_sig_den);
    let time_sig_value = mouse_area(
        text(time_sig_str)
            .size(VALUE_SIZE)
            .line_height(tight)
            .font(Font::MONOSPACE)
            .color(theme::TEXT),
    )
    .on_press(Message::Transport(TransportMessage::CycleTimeSignature));
    let time_sig_block = column![
        value_cell(time_sig_value),
        label_cell(
            text("SIG")
                .size(LABEL_SIZE)
                .line_height(tight)
                .color(theme::TEXT_DIM),
        ),
    ]
    .width(48)
    .align_x(alignment::Horizontal::Center);

    let met_color = if r.transport.metronome_enabled {
        theme::METRONOME_ON
    } else {
        theme::TEXT_DIM
    };
    let met_icon = mouse_area(
        theme::icon(fa::METRONOME)
            .size(VALUE_SIZE)
            .line_height(tight)
            .color(met_color),
    )
    .on_press(Message::Transport(TransportMessage::ToggleMetronome));

    let precount_label = if r.transport.precount_bars == 0 {
        "OFF".to_string()
    } else {
        format!("{} BAR", r.transport.precount_bars)
    };
    let precount_text = mouse_area(
        text(precount_label)
            .size(LABEL_SIZE)
            .line_height(tight)
            .font(Font::MONOSPACE)
            .color(theme::TEXT_DIM),
    )
    .on_press(Message::Transport(TransportMessage::CyclePrecountBars));

    let met_block = column![value_cell(met_icon), label_cell(precount_text)]
        .width(52)
        .align_x(alignment::Horizontal::Center);

    let sep = || container(Space::new(1, BLOCK_HEIGHT - 12.0)).style(theme::separator_bg);

    let timing_panel_row = row![
        position_block,
        sep(),
        bpm_block,
        sep(),
        time_sig_block,
        sep(),
        met_block,
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center)
    .height(BLOCK_HEIGHT);

    container(timing_panel_row)
        .padding(iced::Padding::from([4, 12]))
        .style(theme::timing_panel_style)
}

fn loop_button<'a>(
    r: &Resonance,
    button_pad: iced::Padding,
    icon_size: u16,
) -> iced::widget::Button<'a, Message> {
    let loop_enabled = r.transport.loop_enabled;
    let loop_color = if loop_enabled {
        theme::LOOP_MARKER
    } else {
        theme::TEXT_DIM
    };
    button(theme::icon(fa::BULLSEYE).size(icon_size).color(loop_color))
        .on_press(Message::Transport(TransportMessage::ToggleLoop))
        .padding(button_pad)
        .style(move |_theme, status| {
            if loop_enabled {
                let bg = match status {
                    iced::widget::button::Status::Hovered => {
                        iced::Color::from_rgb(0.25, 0.20, 0.10)
                    }
                    iced::widget::button::Status::Pressed => {
                        iced::Color::from_rgb(0.20, 0.15, 0.08)
                    }
                    _ => iced::Color::from_rgb(0.22, 0.18, 0.08),
                };
                iced::widget::button::Style {
                    background: Some(iced::Background::Color(bg)),
                    text_color: theme::LOOP_MARKER,
                    border: iced::Border {
                        color: theme::LOOP_MARKER,
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                }
            } else {
                theme::transport_button_style(status)
            }
        })
}
