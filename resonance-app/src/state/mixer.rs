//! Mixer-tab UI state: which strip is focused, which parents are
//! expanded to show their sub-tracks, whether the add-track menu is open.

use resonance_audio::types::*;

/// The collapsible groups in the mixer inspector, in display order
/// (mixer-cleanup.md §3.1). Used as the key of
/// [`MixerUiState::collapsed_inspector_groups`] and carried by
/// `UiMessage::ToggleMixerInspectorGroup`.
///
/// The owner's own options group has a key per owner kind: TRACK on a
/// track, BUS on a bus, MASTER on the master. They sit in the same slot,
/// but folding one must not fold the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MixerInspectorGroup {
    Chain,
    Sends,
    Routing,
    Automation,
    Track,
    Bus,
    Master,
}

impl MixerInspectorGroup {
    /// Every group, in display order (the three owner groups last).
    pub const ALL: [MixerInspectorGroup; 7] = [
        MixerInspectorGroup::Chain,
        MixerInspectorGroup::Sends,
        MixerInspectorGroup::Routing,
        MixerInspectorGroup::Automation,
        MixerInspectorGroup::Track,
        MixerInspectorGroup::Bus,
        MixerInspectorGroup::Master,
    ];
}

/// Pure UI state for the mixer view and its menus.
#[derive(Debug, Default)]
pub struct MixerUiState {
    /// The open host-drawn generic plugin window, if any (see
    /// [`crate::state::plugin_window`]).
    pub plugin_window: Option<crate::state::PluginWindowState>,
    /// The plugin slot the user is working on (mixer-cleanup.md §2.1):
    /// set when its window opens (`OpenPluginWindow`, `OpenPluginEditor`,
    /// `OpenGenericParams`) or it is focused (`FocusSlot`), and cleared
    /// when the slot goes away or a project loads. It outlives the
    /// window: closing the window leaves the slot focused. The preset
    /// commands (◀ / ▶ / browse) and the media tab's double-click load
    /// act on it — through `Resonance::preset_target`, which also
    /// refuses a slot that is gone or hidden (Performance mode).
    pub focused_slot: Option<PluginInstanceId>,
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
    /// The inline rename open on a track strip's head (mixer-cleanup.md
    /// §2.3): the track and the edit buffer. Set by a double-click on the
    /// strip's name (`UiMessage::BeginStripRename`); Enter or a click
    /// elsewhere commits it through `TrackMessage::SetTrackName` (so undo
    /// and the control API's rename share one path), Esc drops it.
    /// Runtime UI state, never persisted.
    pub renaming: Option<(TrackId, String)>,
}
