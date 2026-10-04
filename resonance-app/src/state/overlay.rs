//! The window-root overlay, derived from state in one priority order
//! (command-palette.md §3.2).
//!
//! `view()` renders `stack![base, overlay]` for exactly one root overlay at
//! a time. Both the renderer and the keyboard gate read
//! [`Resonance::root_overlay`], so the overlay the user sees is always the
//! one the shortcuts are gated on.

use crate::message::*;
use crate::state::ViewMode;
use crate::Resonance;

/// A window-root overlay, highest priority first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The autosave-recovery prompt (over the startup screen when nothing
    /// is open yet).
    Recovery,
    /// The startup screen: no project is open.
    Startup,
    BounceProgress,
    MixdownProgress,
    FreezeProgress,
    /// The command palette: above every other overlay but the recovery,
    /// startup and progress ones.
    Palette,
    ConfirmQuit,
    /// Save / Don't save / Cancel before a GUI Open or New replaces a
    /// project with unsaved changes (code review UX-01).
    ConfirmProjectSwitch,
    ConfirmDeleteTrack,
    BounceDialog,
    ExportDialog,
    ImportDialog,
    ImportProgress,
    MissingPlugins,
    Relink,
    Settings,
    /// The preset browser a preset bar or CHAIN ☰ menu opens (slice P6).
    PresetBrowser,
    AddTrackMenu,
    MarkersOverview,
    DrumGroupsManager,
    MarkerMenu,
    /// The track context menu and its "Save as preset…" prompt. Drawn in
    /// the arrange area (its anchor is in arrange-area space), not in the
    /// root stack, but gated and dismissed like any other overlay.
    TrackMenu,
    /// The floating "Group selected" bar. Non-modal: it layers over the
    /// arrange view without blocking it, so it gates no keys.
    SelectionBar,
}

impl Overlay {
    /// Whether this overlay is modal: while it shows, only ⌘/Ctrl chords
    /// and Esc dispatch.
    pub fn blocks_keys(self) -> bool {
        !matches!(self, Overlay::SelectionBar)
    }

    /// Whether the command palette may open over this overlay (closing it
    /// first). Running commands from the recovery, startup and progress
    /// overlays is not meaningful.
    pub fn allows_palette(self) -> bool {
        !matches!(
            self,
            Overlay::Recovery
                | Overlay::Startup
                | Overlay::BounceProgress
                | Overlay::MixdownProgress
                | Overlay::FreezeProgress
        )
    }

    /// Whether this is the command palette itself.
    pub fn is_palette(self) -> bool {
        matches!(self, Overlay::Palette)
    }

    /// What Esc sends to close this overlay — the same effect as its
    /// backdrop click or Cancel button. `None` for the overlays Esc must not
    /// close: the startup screen, and the progress modals (stopping a render
    /// takes the explicit Cancel button).
    pub fn dismiss_message(self, r: &Resonance) -> Option<Message> {
        let m = match self {
            Overlay::Recovery => {
                Message::ProjectIo(ProjectIoMessage::RecoveryChoice(RecoveryChoice::Cancel))
            }
            Overlay::Startup
            | Overlay::BounceProgress
            | Overlay::MixdownProgress
            | Overlay::FreezeProgress
            | Overlay::SelectionBar => return None,
            Overlay::Palette => Message::Ui(UiMessage::ClosePalette),
            Overlay::ConfirmQuit => Message::ProjectIo(ProjectIoMessage::CancelQuit),
            Overlay::ConfirmProjectSwitch => {
                Message::ProjectIo(ProjectIoMessage::SwitchChoice(SwitchChoice::Cancel))
            }
            Overlay::ConfirmDeleteTrack => Message::Track(TrackMessage::CancelRemoveTrack),
            Overlay::BounceDialog => Message::Track(TrackMessage::Bounce(BounceMessage::Cancel)),
            Overlay::ExportDialog => Message::Export(ExportMessage::Close),
            Overlay::ImportDialog => Message::Import(ImportMessage::Cancel),
            Overlay::ImportProgress => Message::Ui(UiMessage::DismissImportProgress),
            Overlay::MissingPlugins => Message::Ui(UiMessage::DismissMissingPlugins),
            Overlay::Relink => Message::Relink(RelinkMessage::DismissModal),
            Overlay::Settings => Message::Ui(UiMessage::CloseSettings),
            // Esc reverts the audition; a backdrop click keeps it.
            Overlay::PresetBrowser => Message::Plugin(PluginMessage::PresetUi(
                PresetUiMessage::CloseBrowser { keep: false },
            )),
            Overlay::AddTrackMenu => Message::Ui(UiMessage::CloseAddTrackMenu),
            Overlay::MarkersOverview => Message::Ui(UiMessage::CloseMarkersOverview),
            Overlay::DrumGroupsManager => Message::Compose(
                crate::compose::ComposeMessage::DrumGroups(
                    crate::compose::messages::DrumGroupsMessage::CloseManager,
                ),
            ),
            Overlay::MarkerMenu if r.ui.interaction.marker_rename.is_some() => {
                Message::MarkerUi(MarkerUiMessage::CancelRename)
            }
            Overlay::MarkerMenu => Message::MarkerUi(MarkerUiMessage::CloseMenu),
            Overlay::TrackMenu if r.ui.interaction.preset_save.is_some() => {
                Message::Track(TrackMessage::CloseSavePresetPrompt)
            }
            Overlay::TrackMenu => Message::Ui(UiMessage::CloseTrackMenu),
        };
        Some(m)
    }
}

impl Resonance {
    /// The overlay `view()` stacks over the window, if any. The order here
    /// is the render priority; see `view/mod.rs`.
    pub fn root_overlay(&self) -> Option<Overlay> {
        let o = if self.io.recovery_prompt.is_some() {
            Overlay::Recovery
        } else if !self.io.has_active_project {
            Overlay::Startup
        } else if self.modals.bounce_in_progress.is_some() {
            Overlay::BounceProgress
        } else if self.io.bouncing {
            Overlay::MixdownProgress
        } else if self.freeze.any_in_flight() {
            Overlay::FreezeProgress
        } else if self.ui.palette.is_some() {
            Overlay::Palette
        } else if self.modals.confirm_quit.is_some() {
            Overlay::ConfirmQuit
        } else if self.modals.confirm_switch.is_some() {
            Overlay::ConfirmProjectSwitch
        } else if self.modals.confirm_delete_track.is_some() {
            Overlay::ConfirmDeleteTrack
        } else if self.modals.bounce_dialog.is_some() {
            Overlay::BounceDialog
        } else if self.modals.export_dialog.is_some() {
            Overlay::ExportDialog
        } else if self.modals.import_dialog.is_some() {
            Overlay::ImportDialog
        } else if self.media.import_progress_modal_open {
            Overlay::ImportProgress
        } else if self.missing_plugins.modal_open && self.has_missing_plugins() {
            Overlay::MissingPlugins
        } else if self.media.relink.modal_open && !self.media.relink.modal_targets.is_empty() {
            Overlay::Relink
        } else if self.ui.mixer.settings_open {
            Overlay::Settings
        } else if self.presets.host_browser.is_some() {
            Overlay::PresetBrowser
        } else if self.ui.mixer.add_track_menu_open {
            Overlay::AddTrackMenu
        } else if self.ui.mixer.markers_overview_open {
            Overlay::MarkersOverview
        } else if self.compose.drumroll.manager_open
            && matches!(self.ui.view_mode, ViewMode::Compose)
        {
            Overlay::DrumGroupsManager
        } else if self.ui.interaction.marker_menu.is_some()
            || self.ui.interaction.marker_rename.is_some()
        {
            Overlay::MarkerMenu
        } else if self.ui.interaction.track_menu.is_some()
            || self.ui.interaction.preset_save.is_some()
        {
            Overlay::TrackMenu
        } else if matches!(self.ui.view_mode, ViewMode::Arrange)
            && self.ui.interaction.selected_tracks.len() >= 2
        {
            Overlay::SelectionBar
        } else {
            return None;
        };
        Some(o)
    }

    /// The root overlay when it is modal (see [`Overlay::blocks_keys`]).
    pub fn modal_overlay(&self) -> Option<Overlay> {
        self.root_overlay().filter(|o| o.blocks_keys())
    }

    /// Whether the canvases must ignore key presses: a modal overlay (the
    /// palette included) is open, or the Keyboard panel is capturing a
    /// chord. One predicate for all four canvases.
    pub fn canvas_keys_blocked(&self) -> bool {
        self.modal_overlay().is_some() || self.ui.keymap_editor.capturing.is_some()
    }
}
