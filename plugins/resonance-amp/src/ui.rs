/// Amp plugin UI: file browser for NAM models + input/output gain sliders.

use resonance_plugin::ui::*;
use resonance_plugin::ui::iced::Element;

#[derive(Debug, Clone)]
pub struct AmpUiState {
    pub model_name: String,
    pub file_list: Vec<String>,
    pub current_index: usize,
}

impl Default for AmpUiState {
    fn default() -> Self {
        Self {
            model_name: String::new(),
            file_list: Vec::new(),
            current_index: 0,
        }
    }
}

pub fn view(state: &AmpUiState, params: &[UiParam]) -> Element<'static, PluginUiEvent> {
    view_file_browser(
        &state.model_name,
        None,
        state.file_list.len(),
        state.current_index,
        params,
        &["Input Gain", "Output Gain"],
    )
}
