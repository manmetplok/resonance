/// Iced-based GUI for the amp simulator plugin.

use nih_plug::prelude::{Editor, GuiContext};
use nih_plug_iced::widgets as nih_widgets;
use nih_plug_iced::*;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::params::AmpParams;
use crate::AmpTask;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 320;

pub fn default_state() -> Arc<IcedState> {
    IcedState::from_size(WINDOW_WIDTH, WINDOW_HEIGHT)
}

/// Initialization data passed to the editor.
#[derive(Clone)]
pub struct EditorFlags {
    pub params: Arc<AmpParams>,
    pub model_name: Arc<Mutex<String>>,
    pub task_sender: Arc<dyn Fn(AmpTask) + Send + Sync>,
}

pub fn create(flags: EditorFlags) -> Option<Box<dyn Editor>> {
    create_iced_editor::<AmpEditor>(flags.params.editor_state.clone(), flags)
}

struct AmpEditor {
    params: Arc<AmpParams>,
    context: Arc<dyn GuiContext>,
    model_name: Arc<Mutex<String>>,
    task_sender: Arc<dyn Fn(AmpTask) + Send + Sync>,

    // Widget states
    input_gain_slider: nih_widgets::param_slider::State,
    output_gain_slider: nih_widgets::param_slider::State,
    load_button_state: button::State,
}

#[derive(Debug, Clone)]
enum Message {
    ParamUpdate(nih_widgets::ParamMessage),
    LoadModel,
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
                task_sender: flags.task_sender,
                input_gain_slider: Default::default(),
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
            Message::LoadModel => {
                // Open file dialog (blocking on GUI thread -- standard for file pickers)
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("NAM Model", &["nam"])
                    .pick_file()
                {
                    let path_str = path.to_string_lossy().into_owned();
                    *self.params.model_path.lock() = path_str.clone();
                    (self.task_sender)(AmpTask::LoadModel(path_str));
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

        // Load button
        let load_btn = Button::new(
            &mut self.load_button_state,
            Text::new("Load NAM Model")
                .size(13)
                .horizontal_alignment(alignment::Horizontal::Center),
        )
        .on_press(Message::LoadModel)
        .style(LoadButtonStyle)
        .width(Length::Units(200));

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
            .push(Space::with_height(Length::Units(12)))
            .push(load_btn)
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
