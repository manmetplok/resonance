/// Iced-based GUI for the IR convolution plugin.

use nih_plug::prelude::{Editor, GuiContext};
use nih_plug_iced::widgets as nih_widgets;
use nih_plug_iced::*;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::params::IrParams;
use crate::IrTask;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 340;

pub fn default_state() -> Arc<IcedState> {
    IcedState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

#[derive(Clone)]
pub struct EditorFlags {
    pub params: Arc<IrParams>,
    pub ir_name: Arc<Mutex<String>>,
    pub ir_info: Arc<Mutex<String>>,
    pub task_sender: Arc<dyn Fn(IrTask) + Send + Sync>,
}

pub fn create(flags: EditorFlags) -> Option<Box<dyn Editor>> {
    create_iced_editor::<IrEditor>(flags.params.editor_state.clone(), flags)
}

struct IrEditor {
    params: Arc<IrParams>,
    context: Arc<dyn GuiContext>,
    ir_name: Arc<Mutex<String>>,
    ir_info: Arc<Mutex<String>>,
    task_sender: Arc<dyn Fn(IrTask) + Send + Sync>,

    dry_wet_slider: nih_widgets::param_slider::State,
    output_gain_slider: nih_widgets::param_slider::State,
    load_button_state: button::State,
}

#[derive(Debug, Clone)]
enum Message {
    ParamUpdate(nih_widgets::ParamMessage),
    LoadIr,
}

impl IcedEditor for IrEditor {
    type Executor = executor::Default;
    type Message = Message;
    type InitializationFlags = EditorFlags;

    fn new(
        flags: Self::InitializationFlags,
        context: Arc<dyn GuiContext>,
    ) -> (Self, Command<Self::Message>) {
        (
            Self {
                params: flags.params,
                context,
                ir_name: flags.ir_name,
                ir_info: flags.ir_info,
                task_sender: flags.task_sender,
                dry_wet_slider: Default::default(),
                output_gain_slider: Default::default(),
                load_button_state: Default::default(),
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
            Message::LoadIr => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Impulse Response", &["wav", "WAV"])
                    .pick_file()
                {
                    let path_str = path.to_string_lossy().into_owned();
                    *self.params.ir_path.lock() = path_str.clone();
                    (self.task_sender)(IrTask::LoadIr(path_str));
                }
            }
        }
        Command::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        let title = Text::new("RESONANCE IR")
            .size(20)
            .width(Length::Fill)
            .horizontal_alignment(alignment::Horizontal::Center);

        // IR name display
        let name = self.ir_name.lock().clone();
        let name_display = if name.is_empty() {
            "No IR loaded".to_string()
        } else {
            name
        };
        let name_text = Text::new(name_display)
            .size(14)
            .width(Length::Fill)
            .horizontal_alignment(alignment::Horizontal::Center);

        // IR info (duration, channels)
        let info = self.ir_info.lock().clone();
        let info_text = Text::new(info)
            .size(11)
            .width(Length::Fill)
            .horizontal_alignment(alignment::Horizontal::Center);

        // Load button
        let load_btn = Button::new(
            &mut self.load_button_state,
            Text::new("Load IR")
                .size(13)
                .horizontal_alignment(alignment::Horizontal::Center),
        )
        .on_press(Message::LoadIr)
        .style(LoadButtonStyle)
        .width(Length::Units(200));

        // Dry/wet slider
        let drywet_section = Row::new()
            .push(Text::new("Dry/Wet").size(12).width(Length::Units(70)))
            .push(
                nih_widgets::ParamSlider::new(&mut self.dry_wet_slider, &self.params.dry_wet)
                    .map(Message::ParamUpdate),
            )
            .spacing(8)
            .align_items(Alignment::Center);

        // Output gain slider
        let gain_section = Row::new()
            .push(Text::new("Output").size(12).width(Length::Units(70)))
            .push(
                nih_widgets::ParamSlider::new(
                    &mut self.output_gain_slider,
                    &self.params.output_gain,
                )
                .map(Message::ParamUpdate),
            )
            .spacing(8)
            .align_items(Alignment::Center);

        Column::new()
            .push(title)
            .push(Space::with_height(Length::Units(16)))
            .push(name_text)
            .push(Space::with_height(Length::Units(4)))
            .push(info_text)
            .push(Space::with_height(Length::Units(12)))
            .push(load_btn)
            .push(Space::with_height(Length::Units(24)))
            .push(drywet_section)
            .push(Space::with_height(Length::Units(8)))
            .push(gain_section)
            .padding(20)
            .align_items(Alignment::Center)
            .into()
    }

    fn background_color(&self) -> Color {
        Color::from_rgb(0.12, 0.12, 0.14)
    }
}

struct LoadButtonStyle;

impl button::StyleSheet for LoadButtonStyle {
    fn active(&self) -> button::Style {
        button::Style {
            background: Some(Background::Color(Color::from_rgb(0.22, 0.25, 0.35))),
            border_radius: 4.0,
            border_width: 1.0,
            border_color: Color::from_rgb(0.35, 0.4, 0.55),
            text_color: Color::WHITE,
            ..Default::default()
        }
    }

    fn hovered(&self) -> button::Style {
        button::Style {
            background: Some(Background::Color(Color::from_rgb(0.28, 0.32, 0.45))),
            ..self.active()
        }
    }

    fn pressed(&self) -> button::Style {
        button::Style {
            background: Some(Background::Color(Color::from_rgb(0.18, 0.2, 0.3))),
            ..self.active()
        }
    }
}
