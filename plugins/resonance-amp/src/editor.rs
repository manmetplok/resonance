/// Iced-based GUI for the amp simulator plugin.

use nih_plug::prelude::{Editor, GuiContext, ParamSetter};
use nih_plug_iced::widgets as nih_widgets;
use nih_plug_iced::*;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::params::AmpParams;
use crate::AmpTask;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 380;

pub fn default_state() -> Arc<IcedState> {
    IcedState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

#[derive(Clone)]
pub struct EditorFlags {
    pub params: Arc<AmpParams>,
    pub model_name: Arc<Mutex<String>>,
    pub file_list: Arc<Mutex<Vec<String>>>,
    pub task_sender: Arc<dyn Fn(AmpTask) + Send + Sync>,
}

pub fn create(flags: EditorFlags) -> Option<Box<dyn Editor>> {
    create_iced_editor::<AmpEditor>(flags.params.editor_state.clone(), flags)
}

struct AmpEditor {
    params: Arc<AmpParams>,
    context: Arc<dyn GuiContext>,
    model_name: Arc<Mutex<String>>,
    file_list: Arc<Mutex<Vec<String>>>,
    task_sender: Arc<dyn Fn(AmpTask) + Send + Sync>,

    input_gain_slider: nih_widgets::param_slider::State,
    output_gain_slider: nih_widgets::param_slider::State,
    load_button_state: button::State,
    prev_button_state: button::State,
    next_button_state: button::State,
}

#[derive(Debug, Clone)]
enum Message {
    ParamUpdate(nih_widgets::ParamMessage),
    LoadModel,
    PrevModel,
    NextModel,
}

impl IcedEditor for AmpEditor {
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
                model_name: flags.model_name,
                file_list: flags.file_list,
                task_sender: flags.task_sender,
                input_gain_slider: Default::default(),
                output_gain_slider: Default::default(),
                load_button_state: Default::default(),
                prev_button_state: Default::default(),
                next_button_state: Default::default(),
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
            Message::LoadModel => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("NAM Model", &["nam"])
                    .pick_file()
                {
                    let path_str = path.to_string_lossy().into_owned();
                    *self.params.model_path.lock() = path_str.clone();
                    (self.task_sender)(AmpTask::LoadModel(path_str));
                }
            }
            Message::PrevModel => {
                let list = self.file_list.lock();
                if !list.is_empty() {
                    let current = self.params.file_select.value() as usize;
                    let new_idx = if current == 0 { list.len() - 1 } else { current - 1 };
                    drop(list);
                    let setter = ParamSetter::new(self.context.as_ref());
                    setter.set_parameter(&self.params.file_select, new_idx as i32);
                }
            }
            Message::NextModel => {
                let list = self.file_list.lock();
                if !list.is_empty() {
                    let current = self.params.file_select.value() as usize;
                    let new_idx = if current >= list.len() - 1 { 0 } else { current + 1 };
                    drop(list);
                    let setter = ParamSetter::new(self.context.as_ref());
                    setter.set_parameter(&self.params.file_select, new_idx as i32);
                }
            }
        }
        Command::none()
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        let title = Text::new("RESONANCE AMP")
            .size(20)
            .width(Length::Fill)
            .horizontal_alignment(alignment::Horizontal::Center);

        // Model name display
        let name = self.model_name.lock().clone();
        let model_display = if name.is_empty() {
            "No model loaded".to_string()
        } else {
            name
        };
        let model_text = Text::new(model_display)
            .size(14)
            .width(Length::Fill)
            .horizontal_alignment(alignment::Horizontal::Center);

        // File count info
        let file_count = self.file_list.lock().len();
        let count_text = if file_count > 0 {
            Text::new(format!(
                "{} / {} models",
                self.params.file_select.value() + 1,
                file_count
            ))
            .size(11)
        } else {
            Text::new("").size(11)
        };
        let count_text = count_text
            .width(Length::Fill)
            .horizontal_alignment(alignment::Horizontal::Center);

        // Prev / Load / Next buttons
        let prev_btn = Button::new(
            &mut self.prev_button_state,
            Text::new("<")
                .size(14)
                .horizontal_alignment(alignment::Horizontal::Center),
        )
        .on_press(Message::PrevModel)
        .style(NavButtonStyle)
        .width(Length::Units(40));

        let load_btn = Button::new(
            &mut self.load_button_state,
            Text::new("Browse...")
                .size(13)
                .horizontal_alignment(alignment::Horizontal::Center),
        )
        .on_press(Message::LoadModel)
        .style(LoadButtonStyle)
        .width(Length::Units(120));

        let next_btn = Button::new(
            &mut self.next_button_state,
            Text::new(">")
                .size(14)
                .horizontal_alignment(alignment::Horizontal::Center),
        )
        .on_press(Message::NextModel)
        .style(NavButtonStyle)
        .width(Length::Units(40));

        let nav_row = Row::new()
            .push(prev_btn)
            .push(Space::with_width(Length::Units(8)))
            .push(load_btn)
            .push(Space::with_width(Length::Units(8)))
            .push(next_btn)
            .align_items(Alignment::Center);

        // Gain sliders
        let input_section = Row::new()
            .push(Text::new("Input").size(12).width(Length::Units(60)))
            .push(
                nih_widgets::ParamSlider::new(
                    &mut self.input_gain_slider,
                    &self.params.input_gain,
                )
                .map(Message::ParamUpdate),
            )
            .spacing(8)
            .align_items(Alignment::Center);

        let output_section = Row::new()
            .push(Text::new("Output").size(12).width(Length::Units(60)))
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
            .push(model_text)
            .push(Space::with_height(Length::Units(4)))
            .push(count_text)
            .push(Space::with_height(Length::Units(12)))
            .push(nav_row)
            .push(Space::with_height(Length::Units(24)))
            .push(input_section)
            .push(Space::with_height(Length::Units(8)))
            .push(output_section)
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

struct NavButtonStyle;

impl button::StyleSheet for NavButtonStyle {
    fn active(&self) -> button::Style {
        button::Style {
            background: Some(Background::Color(Color::from_rgb(0.2, 0.2, 0.24))),
            border_radius: 4.0,
            border_width: 1.0,
            border_color: Color::from_rgb(0.3, 0.3, 0.35),
            text_color: Color::WHITE,
            ..Default::default()
        }
    }

    fn hovered(&self) -> button::Style {
        button::Style {
            background: Some(Background::Color(Color::from_rgb(0.28, 0.28, 0.34))),
            ..self.active()
        }
    }

    fn pressed(&self) -> button::Style {
        button::Style {
            background: Some(Background::Color(Color::from_rgb(0.16, 0.16, 0.2))),
            ..self.active()
        }
    }
}
