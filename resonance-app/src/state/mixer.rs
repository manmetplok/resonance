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
    /// The CHAIN row whose ☰ slot menu is open (mixer-cleanup.md §3.2).
    /// One at a time; picking an entry or pressing ☰ again closes it.
    pub slot_menu: Option<PluginInstanceId>,
    /// The slot "Replace…" was picked for: its chain's add picker offers
    /// replacements for it instead of additions until one is picked or
    /// the replace is cancelled.
    pub replacing_slot: Option<PluginInstanceId>,
    /// The "Save preset…" name prompt open under a CHAIN row.
    pub slot_preset_save: Option<SlotPresetSaveState>,
    /// A CHAIN row being dragged by its ⠿ handle (slice S7).
    pub chain_drag: Option<ChainDragState>,
    /// The track whose inspector colour palette is open (§3.1). Keyed by
    /// track so selecting another track does not show it open there.
    pub color_palette: Option<TrackId>,
    /// Set when a popover (slot menu / colour palette) is opened while
    /// another one was already open — i.e. by a press the click-away
    /// listener also sees. That listener's `ChainUiMessage::Dismiss`
    /// arrives after the widget's own message for the same press, so it
    /// spends this flag instead of closing what the press just opened.
    pub popover_switched: bool,
    /// The instrument track whose CHAIN `+ Add instrument` picker is cued
    /// (accent border and a "pick an instrument" line): set by a click on
    /// the strip's "No instrument" line (mixer-cleanup.md §2.1). iced
    /// cannot focus a `pick_list`, so this is the honest stand-in. Keyed
    /// by track; drawn only while that track still lacks an instrument.
    pub instrument_picker_cue: Option<TrackId>,
    /// The inline rename open on a channel's name (mixer-cleanup.md
    /// §2.3, §3.1): what is being renamed, on which surface (a strip head
    /// or the inspector header), and the edit buffer. Set by a
    /// double-click on the name (`UiMessage::BeginRename`); Enter or a
    /// click elsewhere commits it through `TrackMessage::SetTrackName` /
    /// `BusMessage::RenameBus` (so undo and the control API's renames
    /// share one path), Esc drops it. One rename is open at a time, and
    /// only its surface draws the field. Runtime UI state, never
    /// persisted.
    pub renaming: Option<RenameState>,
    /// Whether the pointer is over the open rename field. A press while
    /// it is not commits the rename (`update::inline_rename`): a press on
    /// a layer above the strips — the floating plugin window, a modal —
    /// never reaches the field, so its focus state cannot say the user
    /// clicked away. Not drawn, so not hashed.
    pub rename_hovered: bool,
}

/// What an inline rename renames: a track (never a sub-track — those are
/// named after their parent's output port) or a bus. The master has no
/// name to edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenameTarget {
    Track(TrackId),
    Bus(BusId),
}

/// Where an inline rename's field is drawn. A channel's name shows on its
/// strip head and in the inspector header; only the surface the
/// double-click landed on swaps its name for the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenameSurface {
    Strip,
    Inspector,
}

/// An open inline rename (see [`MixerUiState::renaming`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenameState {
    pub target: RenameTarget,
    pub surface: RenameSurface,
    /// Live edit buffer, seeded with the current name.
    pub buffer: String,
}

/// An in-progress "Save preset…" prompt on a CHAIN row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SlotPresetSaveState {
    pub instance_id: PluginInstanceId,
    /// Live edit buffer, seeded with the slot's loaded preset name.
    pub name: String,
    /// Whether a user preset of this name already exists for the plugin
    /// (the button then reads "Overwrite"). Recomputed per keystroke.
    pub exists: bool,
}

/// A CHAIN-row drag (slice S7). Only the drop changes the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainDragState {
    /// The slot being dragged.
    pub instance_id: PluginInstanceId,
    /// The slot whose row the pointer is over: the dragged slot takes
    /// its place on release. `None` until the pointer enters a row, and
    /// again once it leaves that row — a release off every row drops
    /// nothing.
    pub over: Option<PluginInstanceId>,
    /// Armed by a press while another drag was still armed (a stuck one
    /// whose release was lost). The window-level press listener sees
    /// that same press after the handle's `DragStart`, and spends this
    /// instead of disarming the drag the press just started.
    pub rearmed: bool,
}

impl MixerUiState {
    /// The edit buffer when `target`'s name is being renamed on
    /// `surface` — i.e. when that surface draws the field in place of the
    /// name. `None` on every other surface, so only one field is drawn.
    pub fn rename_buffer(&self, target: RenameTarget, surface: RenameSurface) -> Option<&str> {
        self.renaming
            .as_ref()
            .filter(|r| r.target == target && r.surface == surface)
            .map(|r| r.buffer.as_str())
    }

    /// Close the inspector's transient popovers: the slot menu and the
    /// colour palette.
    pub fn dismiss_inspector_popovers(&mut self) {
        self.slot_menu = None;
        self.color_palette = None;
        self.popover_switched = false;
    }

    /// Whether a slot menu or the colour palette is open.
    pub fn popover_open(&self) -> bool {
        self.slot_menu.is_some() || self.color_palette.is_some()
    }

    /// Drop every transient CHAIN / strip affordance: a drag, the preset
    /// prompt, replace mode, the popovers and the instrument-picker cue.
    /// Run when the view switches or the inspector changes owner, so none
    /// of them survives off-screen to act on a later release or key.
    pub fn reset_chain_ui(&mut self) {
        self.chain_drag = None;
        self.slot_preset_save = None;
        self.replacing_slot = None;
        self.instrument_picker_cue = None;
        self.dismiss_inspector_popovers();
    }

    /// Drop every CHAIN-row affordance that names `instance_id` (its
    /// slot is gone).
    pub fn forget_chain_ui(&mut self, instance_id: PluginInstanceId) {
        if self.slot_menu == Some(instance_id) {
            self.slot_menu = None;
        }
        if self.replacing_slot == Some(instance_id) {
            self.replacing_slot = None;
        }
        if self
            .slot_preset_save
            .as_ref()
            .is_some_and(|p| p.instance_id == instance_id)
        {
            self.slot_preset_save = None;
        }
        if self.chain_drag.is_some_and(|d| d.instance_id == instance_id) {
            self.chain_drag = None;
        }
    }
}
