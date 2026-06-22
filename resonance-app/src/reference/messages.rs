//! User-intent messages for the reference-track (A/B) feature. Each
//! variant is turned into the matching [`resonance_audio::types::AudioCommand`]
//! by `crate::update::reference`, which also mutates [`super::ReferenceState`]
//! optimistically.

use std::path::PathBuf;

use resonance_audio::types::ReferenceId;

#[derive(Debug, Clone)]
pub enum ReferenceMessage {
    /// Load a reference track from disk for A/B comparison.
    LoadRequested(PathBuf),
    /// Remove a loaded reference and free its decoded audio.
    Remove(ReferenceId),
    /// Select which loaded reference the A/B monitor auditions.
    SetActive(ReferenceId),
    /// Flip the monitored source between the mix and the active reference.
    ToggleAbSource,
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
