pub mod drum_map;
pub mod euclidean;
pub mod groups;
pub mod humanize;
pub mod messages;

pub use drum_map::DrumPadMap;
pub use groups::{
    default_drum_groups, default_kit_pads, grid_label, DrumGroup, DrumGroupPad, KitPadInfo,
    GROUP_PALETTE,
};
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
    /// Pad map used by the grid. Currently always the built-in default;
    /// designed so a future file-loader replaces this at construction time.
    pub pad_map: DrumPadMap,

    /// Currently focused drum *group* (for the right-rail drum generator
    /// and the lane's highlight stripe). `None` = first available group.
    pub selected_group_id: Option<u64>,
    /// `Some(group_id)` when the Drum Groups Manager modal is open and
    /// editing that group. `None` when the modal is closed.
    pub managing_group_id: Option<u64>,
    /// True while the manager modal is on screen.
    pub manager_open: bool,
    /// Text typed into the manager modal's kit pad filter.
    pub manager_filter: String,
    /// Section's "base" grid (subdivision) reference used by the right
    /// rail to compute polymeter/polyrhythm presets. Defaults to 4
    /// (sixteenths) matching the design's `defaultGrid` for the section.
    pub base_grid: u8,
    /// Section's "base" cycle reference (steps per bar at base_grid).
    pub base_cycle: u32,

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
            pad_map: DrumPadMap::default_map(),
            selected_group_id: None,
            managing_group_id: None,
            manager_open: false,
            manager_filter: String::new(),
            base_grid: 4,
            base_cycle: 16,
            humanize_velocity: 0.15,
            humanize_timing: 0.1,
            humanize_swing: 0.0,
            humanize_accent: AccentPattern::None,
            humanize_accent_amount: 0.2,
            humanize_scope: HumanizeScope::AllPads,
        }
    }
}
