/// IR plugin UI: file browser for impulse responses + dry/wet and output gain sliders.

use resonance_plugin::ui::*;
use resonance_plugin::ui::iced::Element;

#[derive(Debug, Clone)]
pub struct IrUiState {
    pub ir_name: String,
    pub ir_info: String,
    pub file_list: Vec<String>,
    pub current_index: usize,
}

impl Default for IrUiState {
    fn default() -> Self {
        Self {
            ir_name: String::new(),
            ir_info: String::new(),
            file_list: Vec::new(),
            current_index: 0,
        }
    }
}

pub fn view(state: &IrUiState, params: &[UiParam]) -> Element<'static, PluginUiEvent> {
    view_file_browser(
        &state.ir_name,
        Some(state.ir_info.as_str()),
        state.file_list.len(),
        state.current_index,
        params,
        &["Dry/Wet", "Output Gain"],
    )
}
