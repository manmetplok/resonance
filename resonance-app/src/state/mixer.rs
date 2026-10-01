//! Mixer-tab UI state: which strip is focused, which parents are
//! expanded to show their sub-tracks, whether the add-track menu is open.

use resonance_audio::types::*;

/// The collapsible groups in the mixer inspector, in display order
/// (mixer-cleanup.md §3.1). Used as the key of
/// [`MixerUiState::collapsed_inspector_groups`] and carried by
/// `UiMessage::ToggleMixerInspectorGroup`.
///
/// `Track` is the owner's own options group: TRACK on a track, BUS on a
/// bus, MASTER on the master. They share one key because they sit in
/// the same slot and only one owner is ever shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MixerInspectorGroup {
    Chain,
    Sends,
    Routing,
    Automation,
    Track,
}

impl MixerInspectorGroup {
    /// Every group, in display order.
    pub const ALL: [MixerInspectorGroup; 5] = [
        MixerInspectorGroup::Chain,
        MixerInspectorGroup::Sends,
        MixerInspectorGroup::Routing,
        MixerInspectorGroup::Automation,
        MixerInspectorGroup::Track,
    ];
}

/// Pure UI state for the mixer view and its menus.
#[derive(Debug, Default)]
pub struct MixerUiState {
    /// The open host-drawn generic plugin window, if any (see
    /// [`crate::state::plugin_window`]).
    pub plugin_window: Option<crate::state::PluginWindowState>,
    /// Bus whose strip is selected, if any — the bus counterpart of
    /// [`ClipInteractionState::selected_track`](crate::state::ClipInteractionState::selected_track),
    /// which the mixer inspector reads to decide what to show. It is a
    /// separate field rather than a shared one because bus ids and track
    /// ids are separate id spaces that overlap numerically. The two are
    /// mutually exclusive: `UiMessage::SelectBus` clears the track
    /// selection and `UiMessage::SelectTrack` clears this, so the
    /// inspector always describes exactly one channel.
    pub selected_bus: Option<BusId>,
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
    /// Whether the arrangement-markers overview popover (anchored under the
    /// transport bar) is open. Runtime UI state — toggled by the transport
    /// "flag" button and dismissed by a backdrop click; it stays open across
    /// overview jumps so several sections can be auditioned in a row. Never
    /// persisted to projects (todo #370).
    pub markers_overview_open: bool,
    /// Whether the MASTER strip is selected, so the inspector describes
    /// the master (mixer-cleanup.md §3.3). Mutually exclusive with the
    /// track selection and [`Self::selected_bus`]: `UiMessage::SelectMaster`
    /// clears both, and `SelectTrack(Some(_))` / `SelectBus(Some(_))`
    /// clear this.
    pub selected_master: bool,
}
