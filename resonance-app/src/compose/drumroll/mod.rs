pub mod drum_map;
pub mod groups;

pub use drum_map::DrumPadMap;
pub use groups::{
    default_drum_groups, default_kit_pads, grid_label, DrumGroup, DrumGroupPad, KitPadInfo,
    GROUP_PALETTE,
};

/// Transient UI state for the Compose drumroll view. Lives on
/// `ComposeState::drumroll` (not serialized — this is purely editor state).
#[derive(Debug, Clone)]
pub struct DrumrollViewState {
    /// Velocity used for new hits.
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
}

impl Default for DrumrollViewState {
    fn default() -> Self {
        Self {
            default_velocity: 0.9,
            pad_map: DrumPadMap::default_map(),
            selected_group_id: None,
            managing_group_id: None,
            manager_open: false,
            manager_filter: String::new(),
            base_grid: 4,
            base_cycle: 16,
        }
    }
}
