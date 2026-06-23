//! Top of the application — the window chrome (62px) and the transport
//! bar (74px). The chrome carries brand identity, the project title, and
//! the segmented view selector. The transport bar carries playback
//! controls, the BPM / time-sig / position readouts, and the master
//! stereo meter.
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::text::LineHeight;
use iced::widget::{
    button, canvas as canvas_widget, column, container, mouse_area, row, text, text_input, Space,
};
use iced::{alignment, mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::state::ViewMode;
use crate::theme::{self, fa};
use crate::Resonance;

pub(crate) const CHROME_HEIGHT: f32 = 62.0;
pub(crate) const TRANSPORT_HEIGHT: f32 = 74.0;
/// Horizontal lead-in/lead-out padding of both shell rows.
const SHELL_HPAD: f32 = 30.0;

pub(crate) fn view_transport(r: &Resonance) -> Element<'_, Message> {
    column![view_chrome(r), view_playback_bar(r)]
        .spacing(0)
        .into()
}

// ---------------------------------------------------------------------------
// Chrome row — brand, project title, view tabs, ghost actions.
// ---------------------------------------------------------------------------

fn view_chrome(r: &Resonance) -> Element<'_, Message> {
    let brand = row![
        text("\u{25cf}")
            .size(11)
            .color(theme::ACCENT)
            .line_height(LineHeight::Relative(1.0)),
        Space::new().width(7),
        text("Resonance")
            .size(13)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1)
            .line_height(LineHeight::Relative(1.0)),
    ]
    .align_y(alignment::Vertical::Center);

    let separator = text("/").size(13).color(theme::TEXT_4);

    let project_name = r
        .io
        .project_path
        .as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Untitled".to_string());

    let title = text(project_name)
        .size(17)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1)
        .line_height(LineHeight::Relative(1.0));

    let dirty = if r.dirty { "· unsaved" } else { "· saved" };
    let dirty_label = text(dirty)
        .size(12)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    // 14px between every element of the title cluster — brand / "/" /
    // project title / dirty label stay on one line.
    let left = row![brand, separator, title, dirty_label]
        .spacing(14)
        .align_y(alignment::Vertical::Center);

    let tabs = container(
        row![
            tab_button("Arrange", ViewMode::Arrange, r.view_mode),
            tab_button("Mixer", ViewMode::Mixer, r.view_mode),
            tab_button("Compose", ViewMode::Compose, r.view_mode),
            tab_button("Performance", ViewMode::Performance, r.view_mode),
        ]
        .spacing(3)
        .padding(4),
    )
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    });

    // Window-chrome import affordance — the inbound sibling of the
    // (forthcoming) Export… entry. Opens the MIDI Import modal; dragging a
    // `.mid` onto the window is the other route (see `update.rs`).
    let import_btn = button(
        text("Import\u{2026}")
            .size(12)
            .font(theme::UI_FONT_MEDIUM)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Import(ImportMessage::Open))
    .padding([7, 14])
    .height(28)
    .style(|_theme, status| theme::ghost_button_style(status));

    let settings_btn = button(centered_icon(fa::BARS, theme::TEXT_2, 13, 28))
        .on_press(Message::Ui(UiMessage::OpenSettings))
        .padding(0)
        .width(36)
        .height(28)
        .style(|_theme, status| theme::ghost_button_style(status));

    // "REF" toggle for the Reference & A/B right-rail. Only meaningful in
    // the Mix view (the panel lives there), so it's hidden elsewhere; a
    // zero-width spacer keeps the right cluster's spacing identical when
    // absent.
    let ref_toggle: Element<'_, Message> = if matches!(r.view_mode, ViewMode::Mixer) {
        let active = r.mixer.reference_panel_open;
        let btn = button(
            text("REF")
                .size(11)
                .font(theme::UI_FONT_SEMIBOLD)
                .line_height(LineHeight::Relative(1.0)),
        )
        .on_press(Message::Ui(UiMessage::ToggleReferencePanel))
        .padding([6, 12])
        .height(28)
        .style(move |_theme, status| theme::toggle_button_style(active, theme::ACCENT, true, status));
        // Trailing gap rides inside the element so the right cluster keeps
        // identical spacing in views that hide the toggle.
        row![btn, Space::new().width(10)].into()
    } else {
        Space::new().width(0).into()
    };

    let chrome_row = row![
        Space::new().width(SHELL_HPAD),
        left,
        Space::new().width(Length::Fill),
        tabs,
        Space::new().width(Length::Fill),
        ref_toggle,
        import_btn,
        Space::new().width(10),
        settings_btn,
        Space::new().width(SHELL_HPAD),
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center)
    .height(CHROME_HEIGHT);

    container(chrome_row)
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn tab_button<'a>(
    label: &'a str,
    mode: ViewMode,
    current: ViewMode,
) -> iced::widget::Button<'a, Message> {
    let active = current == mode;
    button(
        text(label)
            .size(12)
            .font(theme::UI_FONT_MEDIUM)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Ui(UiMessage::SwitchView(mode)))
    .style(move |_theme, status| theme::tab_button_style(active, status))
    .padding([7, 18])
}

// ---------------------------------------------------------------------------
// Transport row — playback controls + position/BPM/sig/key/loop + meter.
// ---------------------------------------------------------------------------

fn view_playback_bar(r: &Resonance) -> Element<'_, Message> {
    // Cached stat-block labels (position / time / sig / key / loop).
    // Refreshed by `Resonance::refresh_transport_labels` after every
    // update — the view only reads them, so they can be borrowed as
    // `&str` for the lifetime of the returned Element (no clones).
    let labels = &r.transport_labels;

    let any_armed = r.registry.tracks.iter().any(|t| t.record_armed);

    let prev_btn = transport_btn(fa::BACKWARD_STEP, Message::Transport(TransportMessage::SkipBack));
    let stop_btn = transport_btn(fa::STOP, Message::Transport(TransportMessage::Stop));
    let next_btn = transport_btn(
        fa::FORWARD_STEP,
        Message::Transport(TransportMessage::SkipForward),
    );

    let play_pause = play_pause_button(r);
    let rec_btn = record_button(any_armed);

    let loop_btn = loop_toggle_button(r);
    let metronome_btn = metronome_toggle_button(r);

    let left = row![
        prev_btn,
        stop_btn,
        play_pause,
        rec_btn,
        next_btn,
        vertical_divider(),
        loop_btn,
        metronome_btn,
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    // Stat groups (POSITION | TIME | BPM | SIG | KEY | LOOP).
    let pos_value = text(labels.position.as_str())
        .size(13)
        .font(theme::MONO_FONT)
        .color(theme::ACCENT_SOFT)
        .line_height(LineHeight::Relative(1.0));
    let position_block = stat_block("POSITION", pos_value, 108).align_x(alignment::Horizontal::Center);

    let time_value = text(labels.time.as_str())
        .size(13)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_1)
        .line_height(LineHeight::Relative(1.0));
    let time_block = stat_block("TIME", time_value, 120);

    let bpm_input = text_input("120", &r.transport.bpm_input)
        .on_input(|s| Message::Transport(TransportMessage::SetBpmText(s)))
        .on_submit(Message::Transport(TransportMessage::CommitBpm))
        .width(50)
        .size(17)
        .font(theme::MONO_FONT)
        .align_x(alignment::Horizontal::Center)
        .padding(0)
        .style(theme::borderless_text_input_style);
    let bpm_block = stat_block("BPM", bpm_input, 84);

    let sig_value = mouse_area(
        text(labels.sig.as_str())
            .size(13)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_1)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Transport(TransportMessage::CycleTimeSignature));
    let sig_block = stat_block("SIG", sig_value, 76);

    let key_value = text(labels.key.as_str())
        .size(13)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1)
        .line_height(LineHeight::Relative(1.0));
    let key_block = stat_block("KEY", key_value, 84);

    let loop_value = text(labels.loop_text.as_str())
        .size(13)
        .font(theme::MONO_FONT)
        .color(if r.transport.loop_enabled {
            theme::TEXT_1
        } else {
            theme::TEXT_3
        })
        .line_height(LineHeight::Relative(1.0));
    let loop_block = stat_block("LOOP", loop_value, 100);

    let center = row![
        position_block,
        stat_separator(),
        time_block,
        stat_separator(),
        bpm_block,
        stat_separator(),
        sig_block,
        stat_separator(),
        key_block,
        stat_separator(),
        loop_block,
    ]
    .align_y(alignment::Vertical::Center);

    let meter = canvas_widget(MasterMeter {
        level_l: r.master_level_l,
        level_r: r.master_level_r,
    })
    .width(90)
    .height(10);

    let cpu_text = text("CPU —")
        .size(11)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    let right = row![meter, Space::new().width(20), cpu_text]
        .spacing(0)
        .align_y(alignment::Vertical::Center);

    let row = row![
        Space::new().width(SHELL_HPAD),
        left,
        Space::new().width(Length::Fill),
        center,
        Space::new().width(Length::Fill),
        right,
        Space::new().width(SHELL_HPAD),
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center)
    .height(TRANSPORT_HEIGHT);

    container(row)
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Wrap an icon glyph in a fixed-square centered container so the
/// glyph sits dead center inside its button cell. Iced's button
/// content takes its natural size and aligns top-left without an
/// explicit center container — without this, glyphs drift visibly.
fn centered_icon<'a>(
    glyph: char,
    color: iced::Color,
    icon_size: u16,
    cell: u16,
) -> iced::widget::Container<'a, Message> {
    container(
        theme::icon(glyph)
            .size(f32::from(icon_size))
            .color(color)
            .line_height(LineHeight::Relative(1.0)),
    )
    .width(f32::from(cell))
    .height(f32::from(cell))
    .center_x(Length::Fill)
    .center_y(Length::Fill)
}

fn transport_btn<'a>(glyph: char, on_press: Message) -> iced::widget::Button<'a, Message> {
    button(centered_icon(glyph, theme::TEXT_1, 14, 32))
        .on_press(on_press)
        .padding(0)
        .width(32)
        .height(32)
        .style(|_theme, status| theme::transport_button_style(status))
}

fn play_pause_button(r: &Resonance) -> Element<'_, Message> {
    let glyph = if r.transport.playing {
        fa::PAUSE
    } else {
        fa::PLAY
    };
    let msg = if r.transport.playing {
        Message::Transport(TransportMessage::Pause)
    } else {
        Message::Transport(TransportMessage::Play)
    };
    button(centered_icon(glyph, theme::BG_0, 15, 36))
        .on_press(msg)
        .padding(0)
        .width(36)
        .height(36)
        .style(|_theme, status| theme::primary_button_style(status))
        .into()
}

fn record_button<'a>(any_armed: bool) -> Element<'a, Message> {
    let color = if any_armed {
        theme::BAD
    } else {
        theme::TEXT_3
    };
    let mut btn = button(centered_icon(fa::CIRCLE, color, 11, 32))
        .padding(0)
        .width(32)
        .height(32)
        .style(move |_theme, status| {
            if any_armed {
                theme::record_armed_button_style(status)
            } else {
                theme::transport_button_style(status)
            }
        });
    if any_armed {
        btn = btn.on_press(Message::Transport(TransportMessage::Record));
    }
    btn.into()
}

fn loop_toggle_button(r: &Resonance) -> Element<'_, Message> {
    let active = r.transport.loop_enabled;
    let color = if active { theme::WARM } else { theme::TEXT_2 };
    button(centered_icon(fa::BULLSEYE, color, 14, 32))
        .on_press(Message::Transport(TransportMessage::ToggleLoop))
        .padding(0)
        .width(32)
        .height(32)
        .style(move |_theme, status| theme::toggle_button_style(active, theme::WARM, false, status))
        .into()
}

fn metronome_toggle_button(r: &Resonance) -> Element<'_, Message> {
    let active = r.transport.metronome_enabled;
    let color = if active { theme::GOOD } else { theme::TEXT_2 };
    button(centered_icon(fa::METRONOME, color, 14, 32))
        .on_press(Message::Transport(TransportMessage::ToggleMetronome))
        .padding(0)
        .width(32)
    .height(32)
    .style(move |_theme, status| theme::toggle_button_style(active, theme::GOOD, false, status))
    .into()
}

fn vertical_divider<'a>() -> iced::widget::Container<'a, Message> {
    container(Space::new().height(18))
        .width(1)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE)),
            ..Default::default()
        })
}

fn stat_separator<'a>() -> iced::widget::Container<'a, Message> {
    container(Space::new().height(28))
        .width(1)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE_2)),
            ..Default::default()
        })
}

fn stat_block<'a>(
    label: &'static str,
    value: impl Into<Element<'a, Message>>,
    width: u16,
) -> iced::widget::Container<'a, Message> {
    let label = text(label)
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    let body = column![
        container(value)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center),
        Space::new().height(3),
        container(label)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
    ]
    .align_x(alignment::Horizontal::Center);

    container(body)
        .width(Length::Fixed(width as f32))
        .padding([4, 22])
}

// ---------------------------------------------------------------------------
// Master stereo meter — two stacked bars rendered as a tiny canvas.
// ---------------------------------------------------------------------------

struct MasterMeter {
    level_l: f32,
    level_r: f32,
}

impl<Message> canvas::Program<Message> for MasterMeter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        draw_meter_bar(&mut frame, 0.0, 4.0, bounds.width, self.level_l);
        draw_meter_bar(&mut frame, 6.0, 4.0, bounds.width, self.level_r);
        vec![frame.into_geometry()]
    }
}

fn draw_meter_bar(frame: &mut Frame, y: f32, height: f32, width: f32, level: f32) {
    let bg = Path::rectangle(Point::new(0.0, y), Size::new(width, height));
    frame.fill(&bg, theme::BG_3);

    let level = level.clamp(0.0, 1.0);
    let fill_w = width * level;
    if fill_w > 0.0 {
        // Three-stop gradient approximation: green up to 70%, amber 70-90%,
        // red above. Filling as up to three rects keeps the canvas cheap.
        let green_w = (width * 0.70).min(fill_w);
        if green_w > 0.0 {
            let p = Path::rectangle(Point::new(0.0, y), Size::new(green_w, height));
            frame.fill(&p, theme::GOOD);
        }
        if fill_w > width * 0.70 {
            let amber_start = width * 0.70;
            let amber_w = (width * 0.90 - amber_start).min(fill_w - amber_start);
            let p = Path::rectangle(Point::new(amber_start, y), Size::new(amber_w, height));
            frame.fill(&p, theme::WARM);
        }
        if fill_w > width * 0.90 {
            let red_start = width * 0.90;
            let red_w = fill_w - red_start;
            let p = Path::rectangle(Point::new(red_start, y), Size::new(red_w, height));
            frame.fill(&p, theme::BAD);
        }
    }

    let outline = Path::rectangle(Point::new(0.0, y), Size::new(width, height));
    frame.stroke(
        &outline,
        Stroke::default().with_color(theme::LINE_2).with_width(1.0),
    );
}
