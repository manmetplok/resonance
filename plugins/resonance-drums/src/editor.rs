/// Iced-based GUI for the drum sampler plugin.

use nih_plug::prelude::{Editor, GuiContext};
use nih_plug_iced::widgets as nih_widgets;
use nih_plug_iced::*;
use std::sync::Arc;

use crate::drum_map::{NUM_PADS, PAD_MAPPINGS};
use crate::params::DrumParams;

const WINDOW_WIDTH: u32 = 620;
const WINDOW_HEIGHT: u32 = 520;
const PAD_COLS: usize = 4;
const PAD_ROWS: usize = 3;

pub fn default_state() -> Arc<IcedState> {
    IcedState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

pub fn create(params: Arc<DrumParams>) -> Option<Box<dyn Editor>> {
    create_iced_editor::<DrumsEditor>(params.editor_state.clone(), params)
}

struct DrumsEditor {
    params: Arc<DrumParams>,
    context: Arc<dyn GuiContext>,
    selected_pad: usize,

    // Widget states
    pad_button_states: [button::State; NUM_PADS],
    master_slider_state: nih_widgets::param_slider::State,
    pad_volume_slider_state: nih_widgets::param_slider::State,
    pad_pan_slider_state: nih_widgets::param_slider::State,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    ParamUpdate(nih_widgets::ParamMessage),
    SelectPad(usize),
}

impl IcedEditor for DrumsEditor {
    type Executor = executor::Default;
    type Message = Message;
    type InitializationFlags = Arc<DrumParams>;

    fn new(
        params: Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        (
            Self {
                params,
                context,
                selected_pad: 0,
                pad_button_states: Default::default(),
                master_slider_state: Default::default(),
                pad_volume_slider_state: Default::default(),
                pad_pan_slider_state: Default::default(),
            },
            Command::none(),
        )
    }

    fn context(&self) -> &dyn GuiContext {
        self.context.as_ref()
    }

    fn update(
        &mut self,
        _window: &mut WindowQueue,
        message: Self::Message,
    ) -> Command<Self::Message> {
        match message {
            Message::ParamUpdate(msg) => self.handle_param_message(msg),
            Message::SelectPad(idx) => {
                self.selected_pad = idx;
            }
        }
        Command::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        let title = Text::new("RESONANCE DRUMS")
            .size(20)
            .width(Length::Fill)
            .horizontal_alignment(alignment::Horizontal::Center);

        // Master volume slider
        let master_section = Column::new()
            .push(Text::new("Master").size(12))
            .push(
                nih_widgets::ParamSlider::new(
                    &mut self.master_slider_state,
                    &self.params.master_volume,
                )
                .map(Message::ParamUpdate),
            )
            .spacing(4)
            .align_items(Alignment::Center);

        // 4x3 pad grid - split borrows by destructuring the array
        let selected = self.selected_pad;
        let [s0, s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11] =
            &mut self.pad_button_states;
        let all_states: [&mut button::State; NUM_PADS] =
            [s0, s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11];

        let mut pad_buttons: Vec<Element<'_, Message>> = Vec::with_capacity(NUM_PADS);
        for (pad_idx, state) in all_states.into_iter().enumerate() {
            let is_selected = pad_idx == selected;
            let name = PAD_MAPPINGS[pad_idx].name;

            let label = Text::new(name)
                .size(11)
                .horizontal_alignment(alignment::Horizontal::Center);

            let content = Container::new(label)
                .width(Length::Units(130))
                .height(Length::Units(60))
                .center_x()
                .center_y()
                .style(if is_selected {
                    PadStyle::Selected
                } else {
                    PadStyle::Normal
                });

            let btn: Element<'_, Message> = Button::new(state, content)
                .on_press(Message::SelectPad(pad_idx))
                .style(PadButtonStyle { selected: is_selected })
                .into();

            pad_buttons.push(btn);
        }

        // Arrange into 4x3 grid
        let mut grid = Column::new().spacing(4);
        let mut drain = pad_buttons.into_iter();
        for _row in 0..PAD_ROWS {
            let mut grid_row = Row::new().spacing(4);
            for _col in 0..PAD_COLS {
                if let Some(btn) = drain.next() {
                    grid_row = grid_row.push(btn);
                }
            }
            grid = grid.push(grid_row);
        }

        // Selected pad controls
        let pad = &self.params.pads[self.selected_pad];
        let pad_name = PAD_MAPPINGS[self.selected_pad].name;

        let pad_controls = Column::new()
            .push(Text::new(format!("Pad: {pad_name}")).size(16))
            .push(
                Row::new()
                    .push(
                        Text::new("Volume")
                            .size(12)
                            .width(Length::Units(50)),
                    )
                    .push(
                        nih_widgets::ParamSlider::new(
                            &mut self.pad_volume_slider_state,
                            &pad.volume,
                        )
                        .map(Message::ParamUpdate),
                    )
                    .spacing(8)
                    .align_items(Alignment::Center),
            )
            .push(
                Row::new()
                    .push(Text::new("Pan").size(12).width(Length::Units(50)))
                    .push(
                        nih_widgets::ParamSlider::new(
                            &mut self.pad_pan_slider_state,
                            &pad.pan,
                        )
                        .map(Message::ParamUpdate),
                    )
                    .spacing(8)
                    .align_items(Alignment::Center),
            )
            .spacing(8);

        // Layout
        Column::new()
            .push(title)
            .push(Space::with_height(Length::Units(8)))
            .push(master_section)
            .push(Space::with_height(Length::Units(12)))
            .push(grid)
            .push(Space::with_height(Length::Units(12)))
            .push(pad_controls)
            .padding(16)
            .align_items(Alignment::Center)
            .into()
    }

    fn background_color(&self) -> Color {
        Color::from_rgb(0.12, 0.12, 0.14)
    }
}

// Pad container style
enum PadStyle {
    Normal,
    Selected,
}

impl container::StyleSheet for PadStyle {
    fn style(&self) -> container::Style {
        match self {
            PadStyle::Normal => container::Style {
                background: Some(Background::Color(Color::from_rgb(0.2, 0.2, 0.24))),
                text_color: Some(Color::from_rgb(0.8, 0.8, 0.8)),
                border_radius: 4.0,
                border_width: 1.0,
                border_color: Color::from_rgb(0.3, 0.3, 0.35),
            },
            PadStyle::Selected => container::Style {
                background: Some(Background::Color(Color::from_rgb(0.25, 0.3, 0.45))),
                text_color: Some(Color::WHITE),
                border_radius: 4.0,
                border_width: 2.0,
                border_color: Color::from_rgb(0.4, 0.5, 0.8),
            },
        }
    }
}

// Pad button style
struct PadButtonStyle {
    selected: bool,
}

impl button::StyleSheet for PadButtonStyle {
    fn active(&self) -> button::Style {
        button::Style {
            background: None, // Container handles background
            border_radius: 4.0,
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
            ..Default::default()
        }
    }

    fn hovered(&self) -> button::Style {
        let mut style = self.active();
        if !self.selected {
            style.background = Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.05)));
        }
        style
    }

    fn pressed(&self) -> button::Style {
        let mut style = self.active();
        style.background = Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1)));
        style
    }
}
