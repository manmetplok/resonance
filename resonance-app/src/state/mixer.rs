//! Mixer-tab UI state: which strip is focused, which parents are
//! expanded to show their sub-tracks, whether the add-track menu is open.

use resonance_audio::types::*;

/// The three collapsible groups in the mixer inspector. Used as the key
/// of [`MixerUiState::collapsed_inspector_groups`] and carried by
/// `UiMessage::ToggleMixerInspectorGroup`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MixerInspectorGroup {
    Signal,
    Routing,
    Chain,
}

/// Pure UI state for the mixer view and its menus.
#[derive(Debug, Default)]
pub struct MixerUiState {
    pub selected_plugin: Option<PluginInstanceId>,
    pub expanded_sub_track_parents: std::collections::HashSet<TrackId>,
    pub add_track_menu_open: bool,
    pub settings_open: bool,
    /// Whether the 360px Reference & A/B right-rail is open in the Mix
    /// view. Runtime UI state — toggled by the chrome "REF" button, never
    /// persisted to projects (the loaded references themselves are; this
    /// is just panel visibility).
    pub reference_panel_open: bool,
    /// Inspector groups the user has folded shut. Runtime UI state —
    /// empty by default (everything open), never persisted to projects.
    pub collapsed_inspector_groups: std::collections::HashSet<MixerInspectorGroup>,
}
