pub mod drum_map;
pub mod euclidean;
pub mod humanize;
pub mod messages;

pub use drum_map::DrumPadMap;
pub use humanize::{AccentPattern, HumanizeScope};
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

    // --- Humanizer controls (applied below the euclidean section). ---
    pub humanize_velocity: f32,
    pub humanize_timing: f32,
    pub humanize_swing: f32,
    pub humanize_accent: AccentPattern,
    pub humanize_accent_amount: f32,
    pub humanize_scope: HumanizeScope,
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
            humanize_velocity: 0.15,
            humanize_timing: 0.1,
            humanize_swing: 0.0,
            humanize_accent: AccentPattern::None,
            humanize_accent_amount: 0.2,
            humanize_scope: HumanizeScope::AllPads,
        }
    }
}
