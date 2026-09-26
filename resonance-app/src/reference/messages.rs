//! User-intent messages for the reference-track (A/B) feature. Each
//! variant is turned into the matching [`resonance_audio::types::AudioCommand`]
//! by `crate::update::reference`, which also mutates [`super::ReferenceState`]
//! optimistically.

use std::path::PathBuf;

use resonance_audio::types::{ABSource, ReferenceId};

#[derive(Debug, Clone)]
pub enum ReferenceMessage {
    /// Open the OS file picker (filtered to audio formats) to choose a
    /// reference to load. Resolves to [`Self::FilePicked`].
    PickFile,
    /// Result of the file picker: `Some(path)` loads it, `None` (cancelled)
    /// is a no-op.
    FilePicked(Option<PathBuf>),
    /// Load a reference track from disk for A/B comparison.
    LoadRequested(PathBuf),
    /// Remove a loaded reference and free its decoded audio.
    Remove(ReferenceId),
    /// Select which loaded reference the A/B monitor auditions.
    SetActive(ReferenceId),
    /// Flip the monitored source between the mix and the active reference.
    ToggleAbSource,
    /// Set the monitored source directly — used by the two-segment A/B
    /// control where pressing a segment selects it (rather than toggling).
    SetAbSource(ABSource),
    /// Press-and-hold audition. `true` switches to the reference and
    /// remembers the prior source; `false` restores it.
    MomentaryAudition(bool),
    /// Toggle loudness-matching the active reference to the mix.
    ToggleLoudnessMatch,
    /// Manual reference level trim changed (dB). Coalesces while dragging.
    TrimChanged(f32),
    /// Add a comparison marker to a reference at a sample position.
    AddMarker {
        ref_id: ReferenceId,
        position_samples: u64,
        label: String,
    },
    /// Remove a comparison marker from a reference.
    RemoveMarker { ref_id: ReferenceId, marker_id: u32 },
    /// Seek a reference's own playback cursor to a sample position.
    Scrub {
        ref_id: ReferenceId,
        position_samples: u64,
    },
    /// Toggle whether the reference cursor follows the mix transport.
    ToggleLoopToMix,
    /// Dismiss the current load-failure notice.
    DismissError,
}

impl ReferenceMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            // Reference-track (A/B). Only the content-changing actions named
            // in the design (load / remove / set-active / loudness-match /
            // trim) are reversible; the trim drag coalesces. The monitoring
            // toggles, markers, scrub, and error dismissal are transient.
            Self::LoadRequested(_)
            // A picked file ends in the same load path as a drag-drop, so
            // a successful pick is just as reversible; a cancelled pick
            // (`None`) changes nothing.
            | Self::FilePicked(Some(_))
            | Self::Remove(_)
            | Self::SetActive(_)
            | Self::ToggleLoudnessMatch => UndoAction::Record,
            Self::TrimChanged(_) => {
                UndoAction::RecordCoalesced(CoalesceKey::ReferenceTrim)
            }
            // Opening the picker and a cancelled pick are pure UI / no-ops;
            // the monitoring toggles, markers, scrub, and error dismissal
            // are transient.
            Self::PickFile
            | Self::FilePicked(None)
            | Self::ToggleAbSource
            | Self::SetAbSource(_)
            | Self::MomentaryAudition(_)
            | Self::AddMarker { .. }
            | Self::RemoveMarker { .. }
            | Self::Scrub { .. }
            | Self::ToggleLoopToMix
            | Self::DismissError => UndoAction::Skip,
        }
    }
}
