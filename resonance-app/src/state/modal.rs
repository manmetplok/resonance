//! Open-modal / confirmation-dialog flags and their transient state
//! (ARCH-06 A6-2/A6-3).
//!
//! Held as a sub-struct on [`Resonance`](crate::Resonance) so handlers
//! that only care about which modal is open can take `&ModalState` /
//! `&mut ModalState` instead of the whole app. This covers the
//! generic app-level dialogs; feature-specific transient overlays that
//! already have their own sub-state (e.g. the import-progress modal,
//! which rides `state::ImportProgressTracker` alongside its own flag)
//! are left where they are.

/// Which app-level modal/confirmation dialog is open, if any, plus its
/// transient input.
#[derive(Debug, Clone, Default)]
pub struct ModalState {
    /// When set, the confirmation dialog for deleting a track with
    /// content is shown. Holds the track id that the user wants to
    /// remove.
    pub confirm_delete_track: Option<resonance_audio::types::TrackId>,
    /// When set, the "Bounce in place" dialog is shown for an external
    /// MIDI track. Holds the source track id plus the user's current
    /// device/port selection.
    pub bounce_dialog: Option<crate::state::BounceDialogState>,
    /// When set, the "Import MIDI" modal is shown. Holds the import
    /// flow's stage, the parsed per-track rows, and the user's tempo /
    /// placement choices. `None` when the modal is closed.
    pub import_dialog: Option<crate::state::ImportDialogState>,
    /// When set, a bounce-in-place run is in flight. Drives the modal
    /// progress overlay and gates transport / mutating UI so the user
    /// can't disturb the render mid-flight. Cleared by
    /// `TrackBounceCompleted`, `TrackBounceError`, or
    /// `TrackBounceCancelled`.
    pub bounce_in_progress: Option<crate::state::BounceProgressState>,
    /// When set, the Export modal is open. Holds the shared shell state
    /// (mode tab, source selection, range, format, destination) - see
    /// `state::ExportDialogState` and `view::export_dialog`.
    pub export_dialog: Option<crate::state::ExportDialogState>,
    /// When set, the "unsaved changes" quit-confirmation dialog is
    /// shown. Holds the window id so we can close it if the user
    /// confirms.
    pub confirm_quit: Option<iced::window::Id>,
    /// When set, the app should quit after the current save completes.
    /// Set by the "Save & Quit" flow in the unsaved-changes dialog.
    pub quit_after_save: Option<iced::window::Id>,
}
