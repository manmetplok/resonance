pub mod drum_map;
pub mod euclidean;
pub mod messages;

pub use drum_map::DrumPadMap;
pub use messages::DrumrollMessage;

/// Transient UI state for the Compose drumroll view. Lives on
/// `ComposeState::drumroll` (not serialized — this is purely editor state).
#[derive(Debug, Clone)]
pub struct DrumrollViewState {
    /// Currently focused pad for the sidebar (velocity slider, Apply
    /// button). `None` = no pad selected, sidebar shows a hint.
    pub selected_pad: Option<usize>,
    /// Step-grid resolution. Default 16 = 1/16 notes in 4/4.
    pub steps_per_bar: u32,
    /// Velocity used for new hits. Exposed as a sidebar slider.
    pub default_velocity: f32,
    /// Buffered text inputs for the euclidean form. Parsed on Apply.
    pub euclid_steps_input: String,
    pub euclid_hits_input: String,
    pub euclid_rotation_input: String,
    /// Pad map used by the grid. Currently always the built-in default;
    /// designed so a future file-loader replaces this at construction time.
    pub pad_map: DrumPadMap,
}

impl Default for DrumrollViewState {
    fn default() -> Self {
        Self {
            selected_pad: None,
            steps_per_bar: 16,
            default_velocity: 0.9,
            euclid_steps_input: "16".to_string(),
            euclid_hits_input: "4".to_string(),
            euclid_rotation_input: "0".to_string(),
            pad_map: DrumPadMap::default_map(),
        }
    }
}
